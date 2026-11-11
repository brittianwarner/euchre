//! The device identity (D5, 05 §11.4; contract §N1.5).
//!
//! `device.json` in the backend's state dir holds a [`DeviceRecord`] (`userId`,
//! `sessionToken`, `expiresAt`, `serverUrl` = the server key), mode 0600, written atomically
//! (`DeviceRecord::save`). The session enrolls once, refreshes at 80 % of the token lifetime
//! (`DeviceRecord::needs_refresh`), and enrolls again when a refresh answers `401`. One
//! session per backend, single-flight: concurrent callers share one enroll or refresh.

use std::path::{Path, PathBuf};

pub use euchre_server_core::auth::DeviceRecord;
pub use euchre_server_core::auth::device::DEVICE_FILE;

use crate::net::NetError;

/// The device session of one backend: where its record lives and which server it is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceStore {
    path: PathBuf,
    server_key: String,
}

/// The marker a silent re-enrollment leaves next to `device.json` (review fix PP-7).
pub const RESET_FILE: &str = "device-reset.json";

/// How many set-aside copies of one unreadable file are kept (review fix LDS-8).
const CORRUPT_KEEP: usize = 2;

/// Creates `dir` (and any missing parent) with mode 0700. An existing dir keeps its mode.
pub(super) fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    crate::startup::create_private_dir(dir)
}

/// Moves the unparsable file at `path` aside to `<name>.corrupt-<ms>` (mode 0600) so it is
/// never silently lost, keeping the newest [`CORRUPT_KEEP`] copies (review fix LDS-8). A
/// failure is logged; the caller starts over either way.
pub(super) fn set_aside_corrupt(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let prefix = format!("{name}.corrupt-");
    let aside = dir.join(format!("{prefix}{}", crate::net::wall_ms()));
    if let Err(err) = std::fs::rename(path, &aside) {
        tracing::warn!(path = %path.display(), kind = ?err.kind(), "unreadable file not set aside");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if let Err(err) = std::fs::set_permissions(&aside, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(path = %aside.display(), kind = ?err.kind(), "set-aside file not made private");
        }
    }
    tracing::info!(path = %aside.display(), "unreadable file set aside");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut copies: Vec<(u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file = entry.file_name();
            let stamp = file.to_str()?.strip_prefix(&prefix)?.parse::<u64>().ok()?;
            Some((stamp, entry.path()))
        })
        .collect();
    copies.sort_unstable();
    let extra = copies.len().saturating_sub(CORRUPT_KEEP);
    for (_, old) in copies.into_iter().take(extra) {
        if let Err(err) = std::fs::remove_file(&old) {
            tracing::warn!(path = %old.display(), kind = ?err.kind(), "old set-aside file not removed");
        }
    }
}

/// A short, quote-free name for why `err` failed to read: the IO kind or the JSON category.
/// Never the error text, which can quote the file.
pub(super) fn read_failure_kind(err: &anyhow::Error) -> String {
    if let Some(io) = err.downcast_ref::<std::io::Error>() {
        return format!("io:{:?}", io.kind());
    }
    if let Some(json) = err.downcast_ref::<serde_json::Error>() {
        return format!("json:{:?}", json.classify());
    }
    "unknown".to_owned()
}

/// What [`DeviceStore::load_for_session`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    /// The usable record, when there is one.
    pub record: Option<DeviceRecord>,
    /// The user id of this server's record that ran out (the device enrolls again under a
    /// new id, so its old tables answer `403`).
    pub expired_user: Option<String>,
}

/// A silent re-enrollment: this device had the identity `previousUserId` and lost it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetMarker {
    /// The user id the device had before.
    pub previous_user_id: String,
    /// When it enrolled again (wall-clock ms).
    pub at_ms: u64,
}

impl DeviceStore {
    /// The store for `server_key` in `state_dir` (`<state_dir>/device.json`).
    pub fn new(state_dir: &Path, server_key: &str) -> DeviceStore {
        DeviceStore {
            path: state_dir.join(DEVICE_FILE),
            server_key: server_key.to_owned(),
        }
    }

    /// The record's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The server key a usable record must carry as `serverUrl`.
    pub fn server_key(&self) -> &str {
        &self.server_key
    }

    /// The saved record for this server: `None` when missing, unreadable, for another
    /// server, or expired (each means "enroll"). Never fails: a bad file is logged and
    /// replaced on the next save.
    pub fn load(&self) -> Option<DeviceRecord> {
        self.load_at(euchre_server_core::auth::now_ms())
    }

    /// [`DeviceStore::load`] at wall-clock `now_ms`.
    fn load_at(&self, now_ms: i64) -> Option<DeviceRecord> {
        self.load_for_session_at(now_ms).record
    }

    /// The saved record, and the user id of this server's record when it ran out (the
    /// session marks the device as signed out once it enrolls again).
    pub fn load_for_session(&self) -> Loaded {
        self.load_for_session_at(euchre_server_core::auth::now_ms())
    }

    fn load_for_session_at(&self, now_ms: i64) -> Loaded {
        match DeviceRecord::load(&self.path) {
            Ok(None) => Loaded::default(),
            Ok(Some(record)) if record.server_url != self.server_key => {
                tracing::warn!(
                    path = %self.path.display(),
                    "device record is for another server; enrolling again"
                );
                Loaded::default()
            }
            Ok(Some(record)) if record.is_expired(now_ms) => {
                tracing::info!(path = %self.path.display(), "device session expired; enrolling again");
                Loaded {
                    record: None,
                    expired_user: Some(record.user_id),
                }
            }
            Ok(Some(record)) => Loaded {
                record: Some(record),
                expired_user: None,
            },
            Err(err) => {
                // Never log the error itself: a serde message can quote the file.
                let kind = read_failure_kind(&err);
                if err.downcast_ref::<std::io::Error>().is_some() {
                    tracing::warn!(path = %self.path.display(), %kind, "device record unreadable; enrolling again");
                } else {
                    tracing::warn!(path = %self.path.display(), %kind, "device record corrupt; setting it aside and enrolling again");
                    set_aside_corrupt(&self.path);
                }
                Loaded::default()
            }
        }
    }

    fn reset_path(&self) -> PathBuf {
        self.path.with_file_name(RESET_FILE)
    }

    /// The re-enrollment marker, when this device was signed out and enrolled again.
    pub fn load_reset(&self) -> Option<ResetMarker> {
        let bytes = std::fs::read(self.reset_path()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Records that the device lost `previous_user_id` and enrolled again (mode 0600,
    /// atomic). A failure is logged: the copy falls back to "another device".
    pub fn save_reset(&self, marker: &ResetMarker) {
        let saved = serde_json::to_vec(marker)
            .map_err(anyhow::Error::from)
            .and_then(|body| {
                euchre_server_core::config::write_private_file_atomic(&self.reset_path(), &body)
            });
        if let Err(err) = saved {
            tracing::warn!(error = %err, "re-enrollment marker not saved");
        }
    }

    /// Saves `record` atomically with mode 0600 (creating the dir 0700).
    pub fn save(&self, record: &DeviceRecord) -> Result<(), NetError> {
        let saved = match self.path.parent() {
            Some(dir) => ensure_private_dir(dir).map_err(anyhow::Error::from),
            None => Ok(()),
        }
        .and_then(|()| record.save(&self.path));
        saved.map_err(|err| {
            tracing::warn!(path = %self.path.display(), error = %err, "device record not saved");
            NetError::Other("device record not saved".to_owned())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_server_core::auth::guest::SESSION_TTL_SECS;

    const NOW: i64 = 1_900_000_000_000;

    fn record(server_url: &str, expires_at: i64) -> DeviceRecord {
        DeviceRecord {
            user_id: "guest-1".into(),
            session_token: "token-1".into(),
            expires_at,
            server_url: server_url.into(),
        }
    }

    #[test]
    fn missing_corrupt_foreign_or_expired_record_loads_none() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = DeviceStore::new(dir.path(), "http://127.0.0.1:8080");
        assert_eq!(store.path(), dir.path().join("device.json"));
        assert_eq!(store.server_key(), "http://127.0.0.1:8080");
        // Missing.
        assert_eq!(store.load_at(NOW), None);
        // Corrupt: set aside as `device.json.corrupt-<ms>`, owner-only, never read again
        // (review fix LDS-8).
        std::fs::write(store.path(), b"{not json")?;
        assert_eq!(store.load_at(NOW), None);
        assert!(!store.path().exists(), "the bad file is moved aside");
        let aside: Vec<PathBuf> = std::fs::read_dir(dir.path())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("device.json.corrupt-"))
            })
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(std::fs::read(&aside[0])?, b"{not json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&aside[0])?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(store.path(), br#"{"userId": 7}"#)?;
        assert_eq!(store.load_at(NOW), None);
        // Another server's record.
        let lifetime = SESSION_TTL_SECS * 1000;
        store.save(&record("https://euchre.example", NOW + lifetime))?;
        assert_eq!(store.load_at(NOW), None);
        // The embedded key never matches a remote one either.
        store.save(&record("embedded", NOW + lifetime))?;
        assert_eq!(store.load_at(NOW), None);
        // Expired.
        store.save(&record("http://127.0.0.1:8080", NOW))?;
        assert_eq!(store.load_at(NOW), None);
        // Good.
        let good = record("http://127.0.0.1:8080", NOW + 1);
        store.save(&good)?;
        assert_eq!(store.load_at(NOW), Some(good));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn save_is_atomic_and_private() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir()?;
        let state = root.path().join("remote").join("http_127.0.0.1_8080");
        let store = DeviceStore::new(&state, "http://127.0.0.1:8080");
        let first = record("http://127.0.0.1:8080", NOW + 10);
        store.save(&first)?;
        let mode = |p: &Path| -> anyhow::Result<u32> {
            Ok(std::fs::metadata(p)?.permissions().mode() & 0o777)
        };
        assert_eq!(mode(store.path())?, 0o600);
        assert_eq!(mode(&state)?, 0o700);
        assert_eq!(mode(&root.path().join("remote"))?, 0o700);
        // Replacing keeps the mode, leaves no temp file and reads back whole.
        let mut second = first.clone();
        second.session_token = "token-2".into();
        store.save(&second)?;
        assert_eq!(mode(store.path())?, 0o600);
        let names: Vec<String> = std::fs::read_dir(&state)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        assert_eq!(names, vec!["device.json".to_owned()]);
        assert_eq!(store.load_at(NOW), Some(second));
        // The token never shows in Debug.
        assert!(!format!("{first:?}").contains("token-1"));
        Ok(())
    }
}
