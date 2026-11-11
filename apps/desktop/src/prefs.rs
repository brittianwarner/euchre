//! Local table preferences: what the web app keeps in `localStorage` (06 §13-§15, D14, D15).
//!
//! One JSON file in the app's data directory, written atomically (temp file, fsync, rename)
//! with mode 0600. A missing or corrupt file reads as the defaults. Reads happen once at
//! startup, before the window opens; writes go through the background thread.
//!
//! In the browser build the same JSON lives in `localStorage["euchre:desktop-prefs"]`; a page
//! whose storage is missing or throws reads the defaults and reports a failed save.

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::io;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The preferences file name inside the data directory.
pub const PREFS_FILE: &str = "prefs.json";

/// The per-device preferences.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// Table sound (`euchre:sound-enabled`); off by default (D14).
    pub sound: bool,
    /// Coach-mode hints (`euchre:hints:v1:enabled`); off by default (06 §13).
    pub hints: bool,
    /// Hint auto-retire streaks by situation class (`euchre:hints:v1:streaks`).
    pub hint_streaks: BTreeMap<String, u32>,
    /// Reduce motion, OR-ed with the OS preference (D15).
    pub reduce_motion: bool,
    /// The first-run tour was seen or dismissed (`euchre:onboarding:v1:seen`).
    pub tour_seen: bool,
}

/// Where the preferences live.
#[derive(Clone, Debug)]
pub struct PrefsStore {
    path: PathBuf,
}

impl PrefsStore {
    /// A store whose file is `dir/prefs.json`.
    pub fn in_dir(dir: impl Into<PathBuf>) -> PrefsStore {
        PrefsStore {
            path: dir.into().join(PREFS_FILE),
        }
    }

    /// The platform data directory (`ProjectDirs::from("sh", "euchre", "Euchre")`), or `None`
    /// when the platform has no home directory.
    ///
    /// In the browser it is the page's `localStorage` (the path is only a label there).
    pub fn platform() -> Option<PrefsStore> {
        #[cfg(target_arch = "wasm32")]
        {
            Some(PrefsStore::in_dir("localStorage"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            directories::ProjectDirs::from("sh", "euchre", "Euchre")
                .map(|dirs| PrefsStore::in_dir(dirs.data_dir()))
        }
    }

    /// Where this run keeps its preferences: the override ([`PrefsStore::dir_override`]) when
    /// it is set, none at all for a headless or scene run (it starts from the defaults and
    /// writes nothing, so renders are the same on every machine and never touch the player's
    /// file), else the platform directory (the same dir as `net::data_dir()`).
    pub fn for_run(scene_run: bool, dir_override: Option<PathBuf>) -> Option<PrefsStore> {
        match dir_override {
            Some(dir) => Some(PrefsStore::in_dir(dir)),
            None if scene_run => None,
            None => PrefsStore::platform(),
        }
    }

    /// The directory override for this process: `EUCHRE_PREFS_DIR`, else `EUCHRE_DATA_DIR`
    /// (one variable isolates a whole run: prefs, device, server, logs), else none.
    pub fn dir_override() -> Option<PathBuf> {
        override_from(
            std::env::var_os(PREFS_DIR_ENV),
            std::env::var_os(crate::net::DATA_DIR_ENV),
        )
    }

    /// The file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the preferences from `localStorage`; no storage, a throwing one, a missing key or
    /// corrupt JSON gives the defaults.
    #[cfg(target_arch = "wasm32")]
    pub fn load(&self) -> Prefs {
        let Some(json) = web_storage().and_then(|s| s.get_item(WEB_PREFS_KEY).ok().flatten())
        else {
            return Prefs::default();
        };
        serde_json::from_str(&json).unwrap_or_else(|err| {
            tracing::warn!(%err, "stored prefs unreadable; using defaults");
            Prefs::default()
        })
    }

    /// Writes the preferences to `localStorage` (a refused or throwing write is an error).
    #[cfg(target_arch = "wasm32")]
    pub fn save(&self, prefs: &Prefs) -> io::Result<()> {
        let json = serde_json::to_string(prefs).map_err(io::Error::other)?;
        web_storage()
            .ok_or_else(|| io::Error::other("no localStorage"))?
            .set_item(WEB_PREFS_KEY, &json)
            .map_err(|_| io::Error::other("localStorage refused the write"))
    }

    /// Reads the preferences; a missing, unreadable or corrupt file gives the defaults.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(&self) -> Prefs {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                tracing::warn!(path = %self.path.display(), %err, "prefs file unreadable; using defaults");
                Prefs::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Prefs::default(),
            Err(err) => {
                tracing::warn!(path = %self.path.display(), %err, "prefs file unreadable; using defaults");
                Prefs::default()
            }
        }
    }

    /// Writes the preferences atomically with owner-only permissions.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self, prefs: &Prefs) -> io::Result<()> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("prefs path has no parent"))?;
        // Owner-only, like the data dir (review fix LDS-4).
        crate::startup::create_private_dir(dir)?;
        let bytes = serde_json::to_vec_pretty(prefs).map_err(io::Error::other)?;
        // A name no other write (this process or another) is using, so two saves never share
        // a temp file.
        let seq = SAVE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = dir.join(format!(".{PREFS_FILE}.{}.{seq}.tmp", std::process::id()));
        let result = write_private(&tmp, &bytes).and_then(|()| fs::rename(&tmp, &self.path));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result?;
        sync_dir(dir)
    }
}

/// Environment variable moving the preferences file's directory (tests and CLI runs).
pub const PREFS_DIR_ENV: &str = "EUCHRE_PREFS_DIR";

/// [`PrefsStore::dir_override`] over the two values: the first non-empty one.
fn override_from(
    prefs_dir: Option<std::ffi::OsString>,
    data_dir: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    prefs_dir
        .filter(|d| !d.is_empty())
        .or_else(|| data_dir.filter(|d| !d.is_empty()))
        .map(PathBuf::from)
}

/// The `localStorage` key the browser build keeps its preferences under (same JSON as the
/// file).
#[cfg(target_arch = "wasm32")]
pub const WEB_PREFS_KEY: &str = "euchre:desktop-prefs";

/// The page's `localStorage`, when it has one: a private window or blocked site data may
/// refuse it (the accessor throws, which comes back as an `Err`).
#[cfg(target_arch = "wasm32")]
fn web_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Numbers each save's temp file.
#[cfg(not(target_arch = "wasm32"))]
static SAVE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Makes the rename durable: on unix the directory entry is synced too.
#[cfg(not(target_arch = "wasm32"))]
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Creates (or truncates) `path` with mode 0600, writes `bytes` and syncs it to disk.
#[cfg(not(target_arch = "wasm32"))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_run_never_touches_the_platform_prefs() {
        assert!(
            PrefsStore::for_run(true, None).is_none(),
            "headless and scene runs start from the defaults and save nothing"
        );
        let dir = tempfile::tempdir().unwrap();
        let store = PrefsStore::for_run(true, Some(dir.path().to_path_buf())).unwrap();
        assert_eq!(store.path(), dir.path().join(PREFS_FILE));
        if let (Some(run), Some(platform)) =
            (PrefsStore::for_run(false, None), PrefsStore::platform())
        {
            assert_eq!(
                run.path(),
                platform.path(),
                "a window run keeps the player's file"
            );
        }
    }

    #[test]
    fn prefs_follow_the_data_dir_override() {
        let prefs = std::ffi::OsString::from("/private/tmp/euchre-prefs");
        let data = std::ffi::OsString::from("/private/tmp/euchre-data");
        // EUCHRE_PREFS_DIR wins; else EUCHRE_DATA_DIR; empty values count as unset.
        assert_eq!(
            override_from(Some(prefs.clone()), Some(data.clone())),
            Some(PathBuf::from(&prefs))
        );
        assert_eq!(
            override_from(None, Some(data.clone())),
            Some(PathBuf::from(&data))
        );
        assert_eq!(
            override_from(Some(std::ffi::OsString::new()), Some(data.clone())),
            Some(PathBuf::from(&data))
        );
        assert_eq!(override_from(None, Some(std::ffi::OsString::new())), None);
        assert_eq!(override_from(None, None), None);
        // The store sits in that dir, scene run or not.
        for scene in [true, false] {
            let store = PrefsStore::for_run(scene, override_from(None, Some(data.clone())))
                .expect("a store in the data dir");
            assert_eq!(store.path(), PathBuf::from(&data).join(PREFS_FILE));
        }
    }

    #[test]
    fn missing_file_reads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = PrefsStore::in_dir(dir.path());
        assert_eq!(store.load(), Prefs::default());
        assert!(!store.load().sound, "sound is off by default (D14)");
    }

    #[test]
    fn corrupt_file_reads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = PrefsStore::in_dir(dir.path());
        fs::write(store.path(), b"{not json").unwrap();
        assert_eq!(store.load(), Prefs::default());
    }

    #[test]
    fn partial_file_keeps_known_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = PrefsStore::in_dir(dir.path());
        fs::write(store.path(), br#"{"tourSeen":true,"unknown":1}"#).unwrap();
        let prefs = store.load();
        assert!(prefs.tour_seen);
        assert!(!prefs.hints);
    }

    #[test]
    fn save_round_trips_with_owner_only_mode() {
        let dir = tempfile::tempdir().unwrap();
        let store = PrefsStore::in_dir(dir.path().join("nested"));
        let mut prefs = Prefs {
            sound: true,
            reduce_motion: true,
            ..Prefs::default()
        };
        prefs.hint_streaks.insert("lead".into(), 2);
        store.save(&prefs).unwrap();
        assert_eq!(store.load(), prefs);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            // A dir the save creates is owner-only too (review fix LDS-4).
            let mode = fs::metadata(dir.path().join("nested"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let leftovers: Vec<_> = fs::read_dir(dir.path().join("nested"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
