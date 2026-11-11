//! TS `send-bounded.ts`: bound a call to a sibling actor so a cold or wedged
//! actor can never park the table (03 §10.5).
//!
//! The TS version raced the send against an `AbortSignal` and a timer and
//! aborted the request on either. Here the send is a future: it is dropped (the
//! Rust abort) when the lifecycle token fires or the budget runs out, and it is
//! never polled when the table is already stopping.

use std::future::Future;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

/// The TS error message (`new Error('Actor queue delivery interrupted')`).
pub const INTERRUPTED_MESSAGE: &str = "Actor queue delivery interrupted";

/// The TS default budget (`budgetMs = 1000`).
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(1000);

/// The bounded send was stopped: the budget ran out or the table is stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Actor queue delivery interrupted")]
pub struct Interrupted;

/// Run `send` for at most `budget`, or until `lifecycle` is cancelled.
///
/// An already-cancelled `lifecycle` returns [`Interrupted`] without polling
/// `send`, so nothing is sent. An interrupted `send` is dropped, which cancels
/// the request.
pub async fn send_bounded<T, F>(
    send: F,
    lifecycle: Option<&CancellationToken>,
    budget: Duration,
) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>>,
{
    if lifecycle.is_some_and(CancellationToken::is_cancelled) {
        return Err(Interrupted.into());
    }
    let stopped = async {
        match lifecycle {
            Some(token) => token.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        biased;
        () = stopped => Err(Interrupted.into()),
        () = tokio::time::sleep(budget) => Err(Interrupted.into()),
        out = send => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Flips its flag when dropped: the Rust analog of the TS abort signal.
    struct AbortProbe(Arc<AtomicBool>);

    impl Drop for AbortProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn releases_the_table_when_actor_startup_never_completes_and_cancels_the_request() {
        let aborted = Arc::new(AtomicBool::new(false));
        let probe = AbortProbe(aborted.clone());
        let started = std::time::Instant::now();
        let delivery = send_bounded(
            async move {
                let _probe = probe;
                std::future::pending::<anyhow::Result<()>>().await
            },
            None,
            Duration::from_millis(50),
        )
        .await;
        let err = delivery.err().map(|e| e.to_string());
        assert!(
            err.as_deref().is_some_and(|m| m.contains("interrupted")),
            "{err:?}"
        );
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(aborted.load(Ordering::SeqCst), "the request was cancelled");
    }

    #[tokio::test]
    async fn stops_delivery_when_the_table_shuts_down_without_waiting_for_its_deadline() {
        let lifecycle = CancellationToken::new();
        let stopper = lifecycle.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            stopper.cancel();
        });
        let started = std::time::Instant::now();
        let delivery = send_bounded(
            std::future::pending::<anyhow::Result<()>>(),
            Some(&lifecycle),
            Duration::from_secs(30),
        )
        .await;
        assert_eq!(
            delivery.err().map(|e| e.to_string()).as_deref(),
            Some(INTERRUPTED_MESSAGE)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn does_not_send_anything_for_a_stopped_table() {
        let lifecycle = CancellationToken::new();
        lifecycle.cancel();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let delivery = send_bounded(
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            Some(&lifecycle),
            DEFAULT_BUDGET,
        )
        .await;
        assert!(delivery.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_prompt_send_returns_its_result() {
        let out = send_bounded(async { Ok(7) }, None, DEFAULT_BUDGET).await;
        assert_eq!(out.ok(), Some(7));
    }
}
