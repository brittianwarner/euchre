//! The seat's calls to other actors, by actor name and action name only (D29).
//!
//! Every call is a stateless action through the shared [`SeatClient`], bounded
//! by the runtime's call timeout. The bodies are `euchre_protocol` /
//! `euchre_ai` types serialized to their TS JSON shapes.
//!
//! **Callbacks into the table retry** ([`send_to_table`]). TS sent `aiDecision`
//! and `aiSay` on a durable queue, which outlives a table going to sleep. Here
//! they are actions (D6): one that reaches a table in its sleep grace fails
//! with `stopping`, or hangs while the grace lasts. So each try is bounded by
//! [`TABLE_TRY_TIMEOUT`], made on a fresh handle, and retried up to
//! [`TABLE_TRIES`] times, all inside the runtime's call timeout
//! (`requests/server-table-ai-seat-callback-retry.md`). A retry is safe: the
//! table drops a duplicate or stale decision and keys chat by `msgId`. A
//! refusal (`user/*`, `auth/*`) is final and never retried.

use std::time::Duration;

use anyhow::{Result, anyhow};
use euchre_server_core::rivet::{CallError, parse_rivet_error};
use rivetkit::client::GetOrCreateOptions;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::runtime::SeatClient;

/// The table's registry name (TS `euchreTable`).
pub const TABLE_ACTOR: &str = "euchreTable";
/// The profile's registry name (TS `playerProfile`).
pub const PROFILE_ACTOR: &str = "playerProfile";
/// The table action that takes an `AIDecision`.
pub const AI_DECISION: &str = "aiDecision";
/// The table action that takes an `AiSayMsg`.
pub const AI_SAY: &str = "aiSay";
/// The profile action that takes a `RecordEpisodesBody`.
pub const RECORD_EPISODES: &str = "recordEpisodes";
/// The profile action that takes a `RecordTokensBody`.
pub const RECORD_TOKENS: &str = "recordTokens";

/// Bound on one try of a callback into the table (`aiDecision`, `aiSay`).
pub const TABLE_TRY_TIMEOUT: Duration = Duration::from_secs(2);
/// Most tries at one callback into the table.
pub const TABLE_TRIES: u32 = 5;
/// Pause between two tries of a callback into the table.
pub const TABLE_RETRY_BACKOFF: Duration = Duration::from_millis(100);

/// The table's key: `["table", gameId]`.
pub fn table_key(game_id: &str) -> Vec<String> {
    vec!["table".to_owned(), game_id.to_owned()]
}

/// Where a call goes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Target<'a> {
    pub actor: &'a str,
    pub key: &'a [String],
    pub params: Option<&'a Value>,
    pub action: &'a str,
}

/// One bounded stateless call. The result value is ignored (every target
/// action returns `null`).
pub(crate) async fn send<T: Serialize + ?Sized>(
    client: &SeatClient,
    timeout: Duration,
    target: Target<'_>,
    body: &T,
) -> Result<()> {
    let client = client
        .get()
        .ok_or_else(|| anyhow!("aiSeat client is not bound"))?;
    let handle = client.get_or_create(
        target.actor,
        target.key.to_vec(),
        GetOrCreateOptions {
            params: target.params.cloned(),
            ..Default::default()
        },
    )?;
    let args = vec![serde_json::to_value(body)?];
    match tokio::time::timeout(timeout, handle.action(target.action, args)).await {
        Ok(r) => r.map(|_| ()),
        Err(_) => Err(CallError::Timeout {
            what: target.action.to_owned(),
            after: timeout,
        }
        .into()),
    }
}

/// Whether a failed call must not be retried: the target refused it
/// (`user/*` or `auth/*`), so another try gets the same answer.
fn refused(e: &anyhow::Error) -> bool {
    parse_rivet_error(e).is_some_and(|r| r.group == "user" || r.group == "auth")
}

/// A callback into the table, retried (see the module docs): each try bounded
/// by [`TABLE_TRY_TIMEOUT`], at most [`TABLE_TRIES`] tries
/// [`TABLE_RETRY_BACKOFF`] apart, and all of them within `timeout`. When every
/// try timed out, the error is the overall [`CallError::Timeout`] (`<action>
/// timed out after <timeout>`); otherwise it is the last try's error.
pub(crate) async fn send_to_table<T: Serialize + ?Sized>(
    client: &SeatClient,
    timeout: Duration,
    target: Target<'_>,
    body: &T,
) -> Result<()> {
    if client.get().is_none() {
        return Err(anyhow!("aiSeat client is not bound"));
    }
    let deadline = Instant::now() + timeout;
    let mut last: Option<anyhow::Error> = None;
    for attempt in 0..TABLE_TRIES {
        if attempt > 0 {
            if deadline.saturating_duration_since(Instant::now()) <= TABLE_RETRY_BACKOFF {
                break;
            }
            tokio::time::sleep(TABLE_RETRY_BACKOFF).await;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match send(client, remaining.min(TABLE_TRY_TIMEOUT), target, body).await {
            Ok(()) => return Ok(()),
            Err(e) if refused(&e) => return Err(e),
            Err(e) => {
                tracing::debug!(
                    action = target.action,
                    attempt,
                    "aiSeat callback into the table failed; retrying"
                );
                last = Some(e);
            }
        }
    }
    match last {
        Some(e)
            if !matches!(
                e.downcast_ref::<CallError>(),
                Some(CallError::Timeout { .. })
            ) =>
        {
            Err(e)
        }
        _ => Err(CallError::Timeout {
            what: target.action.to_owned(),
            after: timeout,
        }
        .into()),
    }
}

/// The table connection params: `{ internalToken }` (TS
/// `euchreTable.getOrCreate(['table', gameId], { params: { internalToken } })`).
pub(crate) fn table_params(internal_token: &str) -> Value {
    json!({ "internalToken": internal_token })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unbound_client_fails_without_panicking() {
        let key = table_key("g1");
        let params = table_params("t");
        let r = send(
            &SeatClient::deferred(),
            Duration::from_millis(10),
            Target {
                actor: TABLE_ACTOR,
                key: &key,
                params: Some(&params),
                action: AI_DECISION,
            },
            &json!({}),
        )
        .await;
        assert!(r.is_err());
        assert_eq!(key, vec!["table".to_owned(), "g1".to_owned()]);
        assert_eq!(params, json!({ "internalToken": "t" }));
    }

    #[tokio::test]
    async fn an_unbound_client_fails_the_table_callback_at_once() {
        let key = table_key("g1");
        let params = table_params("t");
        let started = Instant::now();
        let r = send_to_table(
            &SeatClient::deferred(),
            Duration::from_secs(5),
            Target {
                actor: TABLE_ACTOR,
                key: &key,
                params: Some(&params),
                action: AI_SAY,
            },
            &json!({}),
        )
        .await;
        assert!(r.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_refusal_is_final_and_a_transient_error_retries() {
        let internal =
            anyhow!("action failed (user/internal_only): This action is internal., metadata=None");
        assert!(refused(&internal));
        let forbidden = anyhow!("action failed (auth/forbidden): Forbidden, metadata=None");
        assert!(refused(&forbidden));
        let stopping = anyhow!("action failed (actor/stopping): Actor is stopping., metadata=None");
        assert!(!refused(&stopping));
        let timeout: anyhow::Error = CallError::Timeout {
            what: AI_DECISION.to_owned(),
            after: TABLE_TRY_TIMEOUT,
        }
        .into();
        assert!(!refused(&timeout));
    }

    // The retry bounds fit inside the default call timeout.
    const _: () = assert!(TABLE_TRIES > 1);
    const _: () =
        assert!(TABLE_TRY_TIMEOUT.as_millis() < crate::runtime::DEFAULT_CALL_TIMEOUT.as_millis());
}
