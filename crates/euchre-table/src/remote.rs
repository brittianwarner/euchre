//! The table's calls to other actors (03 §10, §11, §16.7), by actor name and
//! action name only (D29).
//!
//! - **One ordered outbox per AI seat and one for the profile.** The core
//!   pushes jobs from inside its lane turn and never waits. The first push to an
//!   idle outbox starts one detached drainer (the actor holds a keep-awake
//!   region around it, G25) that sends the jobs one after another, so a seat
//!   sees `handEnd(N)`, `resetHand(N+1)`, `decide(N+1)` in the order the table
//!   made them (`requests/server-ai-seat-dispatch-order.md`). A slow or missing
//!   seat delays only its own later jobs, never the table or another seat.
//! - **Every send is bounded** by [`send_bounded`] against the actor's abort
//!   signal: `decide` gets `max(1, deadlineAt - now)`, everything else the TS
//!   1 s default.
//! - **Calls are stateless** actions through the shared JSON client
//!   (`get_or_create` + `action`), with `{ internalToken }` connection params to
//!   an `aiSeat` (`requests/done/server-ai-seat-table-params.md`) and none to a
//!   `playerProfile`, whose bodies carry the deployment token instead.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use euchre_engine::Seat;
use euchre_protocol::actor_names::{AI_SEAT, PLAYER_PROFILE};
use euchre_protocol::ai_seat_actions::DECIDE;
use euchre_protocol::profile_actions::{RECORD_GAME, RECORD_HANDS};
use euchre_protocol::{HandJournalEntry, MatchRecord, profile_key};
use euchre_server_core::config::Secret;
use rivetkit::client::{Client, GetOrCreateOptions};
use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::seams::{DecideCall, LifecycleCall, RecordGameCall, RecordHandsCall};
use crate::send_bounded::{DEFAULT_BUDGET, send_bounded};

/// The outbox of the profile (the three seats use `0..=2`).
pub(crate) const PROFILE_LANE: usize = 3;
const LANES: usize = 4;

/// The outbox of an AI seat (`1..=3` → `0..=2`).
pub(crate) fn seat_lane(seat: Seat) -> usize {
    usize::from(seat.get())
        .saturating_sub(1)
        .min(PROFILE_LANE - 1)
}

/// One outbound call.
#[derive(Clone, Debug)]
pub(crate) enum Job {
    /// `aiSeat.decide`.
    Decide(Box<DecideCall>),
    /// `aiSeat.resetHand` / `handEnd` / `gameEnd`.
    Notify(LifecycleCall),
    /// `playerProfile.recordHands`.
    RecordHands(RecordHandsCall),
    /// `playerProfile.recordGame`; its outcome goes back to the table.
    RecordGame(RecordGameCall),
}

impl Job {
    /// The action name, for tests and logs.
    #[cfg(test)]
    pub(crate) fn action(&self) -> &'static str {
        match self {
            Job::Decide(_) => DECIDE,
            Job::Notify(c) => c.message.kind.as_str(),
            Job::RecordHands(_) => RECORD_HANDS,
            Job::RecordGame(_) => RECORD_GAME,
        }
    }
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    draining: bool,
}

/// The per-actor outboxes. In memory: a wake starts empty, and whatever was
/// lost is re-derived from the persisted state (`pending` and the watchdog
/// for a decision, the `flushProfile` ledger for the match record).
#[derive(Default)]
pub(crate) struct Outbox {
    queues: parking_lot::Mutex<[Queue; LANES]>,
    recording: AtomicBool,
}

impl Outbox {
    /// Queue `job` on `lane`. `true` means the outbox was idle and the caller
    /// must start its drainer; otherwise the running drainer picks it up.
    pub(crate) fn push(&self, lane: usize, job: Job) -> bool {
        let mut queues = self.queues.lock();
        let Some(queue) = queues.get_mut(lane) else {
            return false;
        };
        queue.jobs.push_back(job);
        !std::mem::replace(&mut queue.draining, true)
    }

    /// The next job of `lane`, or `None` (and the outbox idle again) when it
    /// is empty. Popping and going idle happen under one lock, so a job pushed
    /// meanwhile either is popped here or starts a new drainer.
    pub(crate) fn next(&self, lane: usize) -> Option<Job> {
        let mut queues = self.queues.lock();
        let queue = queues.get_mut(lane)?;
        let job = queue.jobs.pop_front();
        if job.is_none() {
            queue.draining = false;
        }
        job
    }

    /// Claim the one in-flight `recordGame`. `false`: one is already queued
    /// or in flight, and its outcome will settle the match.
    pub(crate) fn begin_record(&self) -> bool {
        !self.recording.swap(true, Ordering::SeqCst)
    }

    /// The in-flight `recordGame` settled.
    pub(crate) fn end_record(&self) {
        self.recording.store(false, Ordering::SeqCst);
    }
}

/// Send every job of `lane` in order, one at a time, through `run`.
pub(crate) async fn drain<F, Fut>(outbox: &Outbox, lane: usize, mut run: F)
where
    F: FnMut(Job) -> Fut,
    Fut: Future<Output = ()>,
{
    while let Some(job) = outbox.next(lane) {
        run(job).await;
    }
}

/// `playerProfile.recordGame` body (TS `RecordGameMessage`, key order kept).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordGameBody<'a> {
    /// The deployment internal token.
    pub internal_token: &'a str,
    /// The match.
    pub record: &'a MatchRecord,
}

/// `playerProfile.recordHands` body (TS `RecordHandsMessage`, key order kept).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordHandsBody<'a> {
    /// The deployment internal token.
    pub internal_token: &'a str,
    /// The match.
    pub match_id: &'a str,
    /// The hands; the table sends one per call.
    pub hands: &'a [HandJournalEntry],
}

/// The connection params of an `aiSeat` call: `{ internalToken }`.
pub(crate) fn seat_params(internal_token: &str) -> Value {
    json!({ "internalToken": internal_token })
}

/// Where one call goes.
struct Target<'a> {
    actor: &'a str,
    key: Vec<String>,
    params: Option<Value>,
    input: Option<Value>,
    action: &'a str,
}

async fn call(client: &Client, target: Target<'_>, body: Value) -> Result<()> {
    let handle = client.get_or_create(
        target.actor,
        target.key,
        GetOrCreateOptions {
            params: target.params,
            create_with_input: target.input,
            ..Default::default()
        },
    )?;
    handle.action(target.action, vec![body]).await.map(|_| ())
}

/// The budget of a `decide`: TS `Math.max(1, deadlineAt - Date.now())`.
pub(crate) fn decide_budget(deadline_at: u64, now_ms: u64) -> Duration {
    Duration::from_millis(deadline_at.saturating_sub(now_ms).max(1))
}

/// Send one job, bounded (TS `sendBounded`). `Ok` means the target acked.
pub(crate) async fn deliver(
    client: &Client,
    job: &Job,
    profile_token: Option<&Secret>,
    lifecycle: &CancellationToken,
    now_ms: u64,
) -> Result<()> {
    match job {
        Job::Decide(c) => {
            let target = Target {
                actor: AI_SEAT,
                key: c.key.to_vec(),
                params: Some(seat_params(&c.request.internal_token)),
                input: Some(c.create_input.clone()),
                action: DECIDE,
            };
            let body = serde_json::to_value(&c.request)?;
            let budget = decide_budget(c.request.deadline_at, now_ms);
            send_bounded(call(client, target, body), Some(lifecycle), budget).await
        }
        Job::Notify(c) => {
            let target = Target {
                actor: AI_SEAT,
                key: c.key.to_vec(),
                params: Some(seat_params(&c.message.internal_token)),
                input: Some(c.create_input.clone()),
                action: c.message.kind.as_str(),
            };
            let body = serde_json::to_value(&c.message)?;
            send_bounded(call(client, target, body), Some(lifecycle), DEFAULT_BUDGET).await
        }
        Job::RecordHands(c) => {
            let token = profile_token.context("no profile token")?;
            let body = serde_json::to_value(RecordHandsBody {
                internal_token: token.expose(),
                match_id: &c.match_id,
                hands: std::slice::from_ref(&c.entry),
            })?;
            let target = Target {
                actor: PLAYER_PROFILE,
                key: profile_key(&c.profile_user).to_vec(),
                params: None,
                input: None,
                action: RECORD_HANDS,
            };
            send_bounded(call(client, target, body), Some(lifecycle), DEFAULT_BUDGET).await
        }
        Job::RecordGame(c) => {
            let token = profile_token.context("no profile token")?;
            let body = serde_json::to_value(RecordGameBody {
                internal_token: token.expose(),
                record: &c.record,
            })?;
            let target = Target {
                actor: PLAYER_PROFILE,
                key: profile_key(&c.profile_user).to_vec(),
                params: None,
                input: None,
                action: RECORD_GAME,
            };
            send_bounded(call(client, target, body), Some(lifecycle), DEFAULT_BUDGET).await
        }
    }
}

#[cfg(test)]
mod tests;
