//! Desktop device identity (D5, 05 §11.4).
//!
//! `POST /api/device/enroll` mints `guest-<uuid v4>` plus a guest session JWT
//! (`aud: euchre-session`, 400 days, the same token a browser gets as its
//! `euchre_guest` cookie). `POST /api/device/refresh` takes that token as a bearer
//! and re-mints it for the same user id. The client keeps a [`DeviceRecord`] in
//! `device.json` in its app-data dir: mode 0600, written atomically, refreshed at
//! 80 % of the token lifetime.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::AuthError;
use super::guest::{self, GuestAudience, SESSION_TTL_SECS};
use crate::config::write_private_file_atomic;

/// File name of the device record in the app-data dir.
pub const DEVICE_FILE: &str = "device.json";
/// Fraction of the token lifetime after which a client refreshes.
pub const REFRESH_AT_FRACTION: f64 = 0.8;

/// Response body of `POST /api/device/enroll` and `/refresh`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceEnrollment {
    /// `guest-<uuid v4>`.
    pub user_id: String,
    /// Guest session JWT (`aud: euchre-session`).
    pub session_token: String,
    /// Token expiry, unix milliseconds.
    pub expires_at: i64,
}

impl fmt::Debug for DeviceEnrollment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceEnrollment")
            .field("user_id", &self.user_id)
            .field("session_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Enroll a new device at `iat` (unix seconds).
pub fn enroll(key: &[u8], iat: i64) -> Result<DeviceEnrollment, AuthError> {
    issue(key, guest::new_guest_id(), iat)
}

/// Verify `bearer` (a guest session token) at `now` and re-mint it for the same id.
pub fn refresh(key: &[u8], bearer: &str, now: i64) -> Result<DeviceEnrollment, AuthError> {
    let user_id = guest::verify(key, bearer, GuestAudience::Session, now)?;
    issue(key, user_id, now)
}

fn issue(key: &[u8], user_id: String, iat: i64) -> Result<DeviceEnrollment, AuthError> {
    let session_token = guest::sign(key, &user_id, GuestAudience::Session, iat)?;
    Ok(DeviceEnrollment {
        user_id,
        session_token,
        expires_at: (iat + SESSION_TTL_SECS) * 1000,
    })
}

/// The on-disk `device.json`: `{userId, sessionToken, expiresAt, serverUrl}`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    /// `guest-<uuid v4>`.
    pub user_id: String,
    /// Guest session JWT sent as `Authorization: Bearer`.
    pub session_token: String,
    /// Token expiry, unix milliseconds.
    pub expires_at: i64,
    /// Server the record was enrolled against.
    pub server_url: String,
}

impl fmt::Debug for DeviceRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceRecord")
            .field("user_id", &self.user_id)
            .field("session_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("server_url", &self.server_url)
            .finish()
    }
}

impl DeviceRecord {
    /// A record from an enrollment against `server_url`.
    pub fn from_enrollment(enrollment: DeviceEnrollment, server_url: impl Into<String>) -> Self {
        Self {
            user_id: enrollment.user_id,
            session_token: enrollment.session_token,
            expires_at: enrollment.expires_at,
            server_url: server_url.into(),
        }
    }

    /// Replace the token after a refresh (the user id never changes).
    pub fn apply_refresh(&mut self, enrollment: DeviceEnrollment) {
        self.session_token = enrollment.session_token;
        self.expires_at = enrollment.expires_at;
    }

    /// Load `path`; `Ok(None)` when it does not exist.
    pub fn load(path: &Path) -> anyhow::Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Save to `path` atomically with mode 0600.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let body = serde_json::to_vec_pretty(self)?;
        write_private_file_atomic(path, &body)
    }

    /// True once 80 % of the session lifetime has passed at `now_ms`.
    pub fn needs_refresh(&self, now_ms: i64) -> bool {
        let lifetime_ms = SESSION_TTL_SECS * 1000;
        let issued_at = self.expires_at - lifetime_ms;
        let refresh_at = issued_at + (lifetime_ms as f64 * REFRESH_AT_FRACTION) as i64;
        now_ms >= refresh_at
    }

    /// True once the token has expired at `now_ms` (re-enroll on 401).
    pub fn is_expired(&self, now_ms: i64) -> bool {
        now_ms >= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn enroll_and_refresh_keep_the_id() -> Result<(), AuthError> {
        let e = enroll(KEY, NOW)?;
        assert!(e.user_id.starts_with("guest-"));
        assert_eq!(e.expires_at, (NOW + SESSION_TTL_SECS) * 1000);
        assert_eq!(
            guest::verify(KEY, &e.session_token, GuestAudience::Session, NOW)?,
            e.user_id
        );
        let r = refresh(KEY, &e.session_token, NOW + 100)?;
        assert_eq!(r.user_id, e.user_id);
        assert_eq!(r.expires_at, (NOW + 100 + SESSION_TTL_SECS) * 1000);
        assert!(refresh(KEY, "garbage", NOW).is_err());
        let actor = guest::sign(KEY, &e.user_id, GuestAudience::Actors, NOW)?;
        assert!(
            refresh(KEY, &actor, NOW).is_err(),
            "actor token is not a session bearer"
        );
        assert!(!format!("{e:?}").contains(&e.session_token));
        Ok(())
    }

    #[test]
    fn record_round_trip_and_refresh_window() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(DEVICE_FILE);
        assert!(DeviceRecord::load(&path)?.is_none());
        let rec = DeviceRecord::from_enrollment(enroll(KEY, NOW)?, "http://127.0.0.1:3000");
        rec.save(&path)?;
        assert_eq!(DeviceRecord::load(&path)?, Some(rec.clone()));
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        for k in ["userId", "sessionToken", "expiresAt", "serverUrl"] {
            assert!(json.get(k).is_some(), "{k}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path)?.permissions().mode() & 0o777,
                0o600
            );
        }
        let issued_ms = NOW * 1000;
        let lifetime = SESSION_TTL_SECS * 1000;
        assert!(!rec.needs_refresh(issued_ms + lifetime * 79 / 100));
        assert!(rec.needs_refresh(issued_ms + lifetime * 80 / 100));
        assert!(!rec.is_expired(rec.expires_at - 1));
        assert!(rec.is_expired(rec.expires_at));
        assert!(!format!("{rec:?}").contains(&rec.session_token));
        Ok(())
    }
}
