//! The tables this device opened (06 §17.1; contract §N1.7): `recent-tables.json` in the
//! backend's state dir, a JSON array of game ids, newest first, at most [`RECENT_LIMIT`].
//! Bookmarks grant no access: `GET /api/tables/recent?ids=` rechecks ownership per table, so
//! a `null` row never prunes one (the table may just be asleep). The cap prunes, and so does a
//! table the server says does not exist (`404` on its session; review fix PP-1).

use std::path::{Path, PathBuf};

use euchre_protocol::is_game_id;

use super::device::{ensure_private_dir, read_failure_kind, set_aside_corrupt};
use crate::net::NetError;

/// The file name in the state dir.
pub const RECENT_FILE: &str = "recent-tables.json";
/// How many tables are remembered (the web keeps 12 per browser).
pub const RECENT_LIMIT: usize = 12;

/// The recent-tables file of one backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentStore {
    path: PathBuf,
}

/// `ids` deduplicated (first wins), lowercase game ids only, capped at [`RECENT_LIMIT`].
fn normalize(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(RECENT_LIMIT);
    for id in ids {
        if out.len() == RECENT_LIMIT {
            break;
        }
        if is_game_id(&id) && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

impl RecentStore {
    /// The store in `state_dir`.
    pub fn new(state_dir: &Path) -> RecentStore {
        RecentStore {
            path: state_dir.join(RECENT_FILE),
        }
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The remembered ids, newest first; empty when missing or unreadable. Entries that are
    /// not lowercase game ids are skipped.
    pub fn load(&self) -> Vec<String> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(err) => {
                tracing::warn!(path = %self.path.display(), kind = ?err.kind(), "recent tables unreadable");
                return Vec::new();
            }
        };
        match serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) {
            Ok(values) => normalize(
                values
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned)),
            ),
            Err(err) => {
                let kind = read_failure_kind(&anyhow::Error::from(err));
                tracing::warn!(path = %self.path.display(), %kind, "recent tables corrupt; setting them aside");
                set_aside_corrupt(&self.path);
                Vec::new()
            }
        }
    }

    /// Drops `game_id` (the server says it does not exist) and saves. Nothing to drop
    /// writes nothing.
    pub fn forget(&self, game_id: &str) -> Result<(), NetError> {
        let before = self.load();
        if !before.iter().any(|id| id == game_id) {
            return Ok(());
        }
        let ids: Vec<String> = before.into_iter().filter(|id| id != game_id).collect();
        self.save(&ids)
    }

    /// Moves `game_id` to the front (deduplicated, capped) and saves atomically, mode 0600.
    /// An id that is not a lowercase game id is ignored.
    pub fn remember(&self, game_id: &str) -> Result<(), NetError> {
        if !is_game_id(game_id) {
            tracing::warn!("not a game id; not remembered");
            return Ok(());
        }
        let ids = normalize(std::iter::once(game_id.to_owned()).chain(self.load()));
        self.save(&ids)
    }

    /// Saves `ids` atomically, mode 0600 (creating the dir 0700).
    fn save(&self, ids: &[String]) -> Result<(), NetError> {
        let saved = serde_json::to_vec(ids)
            .map_err(anyhow::Error::from)
            .and_then(|body| {
                if let Some(dir) = self.path.parent() {
                    ensure_private_dir(dir)?;
                }
                euchre_server_core::config::write_private_file_atomic(&self.path, &body)
            });
        saved.map_err(|err| {
            tracing::warn!(path = %self.path.display(), error = %err, "recent tables not saved");
            NetError::Other("recent tables not saved".to_owned())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: usize) -> String {
        format!("00000000-0000-4000-8000-{n:012x}")
    }

    #[test]
    fn remember_moves_to_front_dedupes_and_caps() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = RecentStore::new(&dir.path().join("state"));
        assert!(store.load().is_empty());
        store.remember(&id(1))?;
        store.remember(&id(2))?;
        store.remember(&id(3))?;
        assert_eq!(store.load(), vec![id(3), id(2), id(1)]);
        // Again: moves to the front, no duplicate.
        store.remember(&id(1))?;
        assert_eq!(store.load(), vec![id(1), id(3), id(2)]);
        for n in 4..=20 {
            store.remember(&id(n))?;
        }
        let ids = store.load();
        assert_eq!(ids.len(), RECENT_LIMIT);
        assert_eq!(ids.first(), Some(&id(20)));
        assert_eq!(ids.last(), Some(&id(9)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(store.path())?.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // No temp file left next to it.
        assert_eq!(std::fs::read_dir(dir.path().join("state"))?.count(), 1);
        Ok(())
    }

    #[test]
    fn bad_file_loads_empty() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = RecentStore::new(dir.path());
        for body in [&b"not json"[..], b"{\"ids\": []}", b"\"x\"", b""] {
            std::fs::write(store.path(), body)?;
            assert!(store.load().is_empty());
        }
        // A bad file is replaced by the next remember.
        store.remember(&id(5))?;
        assert_eq!(store.load(), vec![id(5)]);
        Ok(())
    }

    /// Review fix PP-1: a table the server says is gone leaves the list; the rest keep
    /// their order.
    #[test]
    fn forget_drops_one_id_and_keeps_the_order() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = RecentStore::new(dir.path());
        // Nothing remembered: forgetting writes nothing.
        store.forget(&id(1))?;
        assert!(!store.path().exists());
        for n in 1..=3 {
            store.remember(&id(n))?;
        }
        store.forget(&id(2))?;
        assert_eq!(store.load(), vec![id(3), id(1)]);
        store.forget(&id(9))?;
        assert_eq!(store.load(), vec![id(3), id(1)]);
        Ok(())
    }

    /// Review fix LDS-8: an unparsable file is set aside (0600, newest two kept), never
    /// silently lost.
    #[test]
    fn a_corrupt_file_is_set_aside_and_only_two_copies_are_kept() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = RecentStore::new(dir.path());
        let copies = || -> anyhow::Result<Vec<String>> {
            let mut names: Vec<String> = std::fs::read_dir(dir.path())?
                .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect::<Result<_, _>>()?;
            names.retain(|n| n.starts_with("recent-tables.json.corrupt-"));
            names.sort();
            Ok(names)
        };
        for round in 0..4 {
            std::fs::write(store.path(), format!("not json {round}"))?;
            assert!(store.load().is_empty());
            assert!(!store.path().exists(), "the bad file moved aside");
            // Distinct stamps.
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let kept = copies()?;
        assert_eq!(kept.len(), 2, "{kept:?}");
        let newest = std::fs::read_to_string(dir.path().join(&kept[1]))?;
        assert_eq!(newest, "not json 3");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(&kept[0]))?
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        Ok(())
    }

    #[test]
    fn invalid_ids_are_ignored() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = RecentStore::new(dir.path());
        let upper = id(7)
            .to_uppercase()
            .replace("0000-4000-8000", "0000-4000-A000");
        let body = serde_json::to_vec(&serde_json::json!([
            id(1),
            "not-a-game",
            upper,
            42,
            null,
            id(1),
            id(2),
        ]))?;
        std::fs::write(store.path(), body)?;
        assert_eq!(store.load(), vec![id(1), id(2)]);
        // Remembering a bad id changes nothing.
        store.remember("../../etc/passwd")?;
        store.remember(&upper)?;
        assert_eq!(store.load(), vec![id(1), id(2)]);
        Ok(())
    }
}
