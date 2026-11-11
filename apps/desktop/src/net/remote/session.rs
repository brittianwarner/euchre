//! The device session of one backend (contract §N1.5): load once, enroll when there is no
//! record, refresh at 80 %, enroll again when a refresh answers `401`, and recover once when a
//! call answers `401`. Single-flight: the record lives behind one async mutex held across the
//! network call, so eight parallel first calls share one enroll. Enrolling again in place of
//! a known identity (a refused refresh, a session that ran out) leaves `device-reset.json`,
//! so a `403` from an old table reads "signed out", not "another device" (review fix PP-7).

use std::future::Future;

use tokio::sync::Mutex;

use super::device::{DeviceRecord, DeviceStore, ResetMarker};
use super::http::Api;
use crate::net::NetError;

/// The bearer and the user id a call runs as.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Creds {
    /// The guest session JWT.
    pub bearer: String,
    /// The guest id it was issued for.
    pub user_id: String,
}

impl std::fmt::Debug for Creds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Creds")
            .field("bearer", &"<redacted>")
            .field("user_id", &self.user_id)
            .finish()
    }
}

/// What [`Session::ensure`] does with the record it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plan {
    /// No usable record: `POST /api/device/enroll`.
    Enroll,
    /// 80 % of the lifetime has passed: `POST /api/device/refresh`.
    Refresh,
    /// Use it as it is.
    Use,
}

/// The plan for `record` at wall-clock `now_ms`.
pub(super) fn plan(record: Option<&DeviceRecord>, now_ms: i64) -> Plan {
    match record {
        None => Plan::Enroll,
        Some(r) if r.is_expired(now_ms) => Plan::Enroll,
        Some(r) if r.needs_refresh(now_ms) => Plan::Refresh,
        Some(_) => Plan::Use,
    }
}

#[derive(Debug, Default)]
struct State {
    /// `device.json` was read (once per backend).
    loaded: bool,
    record: Option<DeviceRecord>,
    /// The user id of this server's record that ran out on disk, until the next enroll.
    expired_user: Option<String>,
    /// The device enrolled again in place of a known identity (`device-reset.json`).
    reset: Option<ResetMarker>,
}

/// One backend's device session.
#[derive(Debug)]
pub(super) struct Session {
    api: Api,
    store: DeviceStore,
    state: Mutex<State>,
}

impl Session {
    pub(super) fn new(api: Api, store: DeviceStore) -> Session {
        Session {
            api,
            store,
            state: Mutex::new(State::default()),
        }
    }

    /// Reads `device.json` the first time (off the async thread).
    async fn load_once(&self, state: &mut State) {
        if state.loaded {
            return;
        }
        let store = self.store.clone();
        let (loaded, reset) =
            tokio::task::spawn_blocking(move || (store.load_for_session(), store.load_reset()))
                .await
                .unwrap_or_default();
        state.record = loaded.record;
        state.expired_user = loaded.expired_user;
        state.reset = reset;
        state.loaded = true;
    }

    /// Saves `record` (a failure is logged; the session stays in memory).
    async fn persist(&self, record: &DeviceRecord) {
        let store = self.store.clone();
        let record = record.clone();
        // `DeviceStore::save` logs its own failure.
        let _ = tokio::task::spawn_blocking(move || store.save(&record)).await;
    }

    /// Enrolls (again). The identity it replaces is the record in `state` (a refused or
    /// expired session) or the one that ran out on disk; when there is one, the device is
    /// marked as signed out.
    async fn enroll(&self, state: &mut State) -> Result<(), NetError> {
        let previous = state
            .record
            .as_ref()
            .map(|r| r.user_id.clone())
            .or_else(|| state.expired_user.clone());
        let enrollment = self.api.enroll().await?;
        let record = DeviceRecord::from_enrollment(enrollment, self.store.server_key());
        tracing::info!(user_id = %record.user_id, "device enrolled");
        self.persist(&record).await;
        if let Some(previous_user_id) = previous.filter(|id| *id != record.user_id) {
            tracing::info!(
                user_id = %record.user_id,
                %previous_user_id,
                "device enrolled again; its older tables now answer 403"
            );
            let marker = ResetMarker {
                previous_user_id,
                at_ms: crate::net::wall_ms(),
            };
            let store = self.store.clone();
            let saved = marker.clone();
            let _ = tokio::task::spawn_blocking(move || store.save_reset(&saved)).await;
            state.reset = Some(marker);
        }
        state.expired_user = None;
        state.record = Some(record);
        Ok(())
    }

    /// Whether this device was signed out and enrolled again under a new identity, so a
    /// `403` from a table it remembers means "your old identity", not "another device".
    pub(super) async fn was_reset(&self) -> bool {
        let mut state = self.state.lock().await;
        self.load_once(&mut state).await;
        state.reset.is_some()
    }

    /// Refreshes the record in `state`: `Ok(true)` refreshed, `Ok(false)` the server refused
    /// the session (`401`), `Err` anything else.
    async fn refresh(&self, state: &mut State) -> Result<bool, NetError> {
        let Some(record) = state.record.as_mut() else {
            return Ok(false);
        };
        match self.api.refresh(&record.session_token).await {
            Ok(enrollment) => {
                record.apply_refresh(enrollment);
                let record = record.clone();
                self.persist(&record).await;
                Ok(true)
            }
            Err(NetError::Unauthorized) => {
                tracing::warn!(
                    user_id = %record.user_id,
                    "device session refused; enrolling again (tables of the old session now answer 403)"
                );
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    fn creds(state: &State) -> Result<Creds, NetError> {
        state
            .record
            .as_ref()
            .map(|r| Creds {
                bearer: r.session_token.clone(),
                user_id: r.user_id.clone(),
            })
            .ok_or(NetError::Unauthorized)
    }

    /// The session to call with: enrolls when there is none, refreshes at 80 % (a refresh
    /// that fails for any reason but `401` keeps the current record).
    pub(super) async fn ensure(&self) -> Result<Creds, NetError> {
        let mut state = self.state.lock().await;
        self.load_once(&mut state).await;
        match plan(state.record.as_ref(), euchre_server_core::auth::now_ms()) {
            Plan::Use => {}
            Plan::Enroll => self.enroll(&mut state).await?,
            Plan::Refresh => match self.refresh(&mut state).await {
                Ok(true) => {}
                Ok(false) => self.enroll(&mut state).await?,
                Err(err) => {
                    tracing::warn!(error = ?err, "device refresh failed; keeping the current session");
                }
            },
        }
        Self::creds(&state)
    }

    /// A call made with `failed` answered `401`: refresh, or enroll again when the refresh is
    /// refused too. A caller that lost the race gets the session another caller already
    /// recovered.
    pub(super) async fn recover(&self, failed: &str) -> Result<Creds, NetError> {
        let mut state = self.state.lock().await;
        self.load_once(&mut state).await;
        let current = state.record.as_ref().map(|r| r.session_token.as_str());
        if current.is_some_and(|token| token != failed) {
            return Self::creds(&state);
        }
        if current.is_none() || !self.refresh(&mut state).await? {
            self.enroll(&mut state).await?;
        }
        Self::creds(&state)
    }

    /// Runs `call` with the session; on `401`, recovers once and runs it again. A second
    /// `401` is [`NetError::Unauthorized`].
    pub(super) async fn authed<T, F, Fut>(&self, call: F) -> Result<T, NetError>
    where
        F: Fn(Creds) -> Fut,
        Fut: Future<Output = Result<T, NetError>>,
    {
        let creds = self.ensure().await?;
        match call(creds.clone()).await {
            Err(NetError::Unauthorized) => {
                let creds = self.recover(&creds.bearer).await?;
                call(creds).await
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_server_core::auth::guest::SESSION_TTL_SECS;

    #[test]
    fn plan_refreshes_at_eighty_percent_and_enrolls_when_expired() {
        let lifetime = SESSION_TTL_SECS * 1000;
        let issued = 1_800_000_000_000_i64;
        let record = DeviceRecord {
            user_id: "guest-1".into(),
            session_token: "t".into(),
            expires_at: issued + lifetime,
            server_url: "embedded".into(),
        };
        assert_eq!(plan(None, issued), Plan::Enroll);
        assert_eq!(plan(Some(&record), issued), Plan::Use);
        let eighty = issued + lifetime * 8 / 10;
        assert_eq!(plan(Some(&record), eighty - 1), Plan::Use);
        assert_eq!(plan(Some(&record), eighty), Plan::Refresh);
        assert_eq!(plan(Some(&record), issued + lifetime - 1), Plan::Refresh);
        assert_eq!(plan(Some(&record), issued + lifetime), Plan::Enroll);
    }

    #[test]
    fn creds_debug_hides_the_bearer() {
        let creds = Creds {
            bearer: "secret-bearer".into(),
            user_id: "guest-1".into(),
        };
        assert!(!format!("{creds:?}").contains("secret-bearer"));
    }
}
