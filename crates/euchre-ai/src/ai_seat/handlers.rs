//! The aiSeat queue handlers (`ai-seat/index.ts` `onDecide` / `onResetHand` /
//! `onHandEnd` / `onGameEnd`) as pure functions over [`AiSeatState`].
//!
//! No actor runtime here: each handler mutates the state and returns what the
//! glue must persist and send, so the glue keeps the TS ordering exactly:
//!
//! ```text
//! decide:   on_decide → save(immediate) → send aiDecision
//!           → begin_banter → save(immediate) → spawn run_banter_job → send aiSay
//! resetHand: on_reset_hand → save(immediate)
//! handEnd:  on_hand_end → send recordEpisodes → complete_episode_flush → save
//! gameEnd:  on_game_end → send recordEpisodes → complete_episode_flush
//!           → send recordTokens → save
//! ```

use euchre_protocol::{AIDecideRequest, AIDecision, AiSayMsg, AiSeatLifecycleMessage, EpisodeKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use subtle::ConstantTimeEq;

use crate::ai_seat::bridge::{LlmBanterInput, LlmProvider, call_banter_writer};
use crate::ai_seat::ladder::{LadderDeps, LadderOutcome, run_ladder};
use crate::ai_seat::memory::{
    EpisodeParams, RememberParams, absorb_public_state, episode_for, remember_hand, reset_memory,
    situation_line,
};
use crate::ai_seat::state::AiSeatState;
use crate::ai_seat::status::{RecordDecisionParams, apply_usage, record_decision};
use crate::config::{BANTER_BUDGET_MS, EPISODE_BUFFER_CAP};
use crate::screen::seat::public_only_view;
use crate::types::{Clock, SeatLog};

/// The deployment-wide dev fallback for `playerProfile` calls.
pub const DEV_PROFILE_TOKEN: &str = "euchre-dev-insecure-secret";

/// Log messages, byte-exact.
pub mod messages {
    /// A forged `decide` token.
    pub const FORGED_DECIDE: &str = "dropped forged decide request";
    /// A `decide` for another seat.
    pub const WRONG_SEAT: &str = "dropped decide request for a different seat";
    /// An empty legal set.
    pub const EMPTY_LEGAL: &str = "decide requested with an empty legal set";
    /// The banter send failed.
    pub const BANTER_UNDELIVERED: &str = "optional table talk could not be delivered";
    /// The episode send failed.
    pub const FLUSH_EPISODES_FAILED: &str = "flushEpisodes failed; will retry at the next boundary";
    /// The token send failed.
    pub const FLUSH_TOKENS_FAILED: &str = "flushTokens failed";
    /// A queue handler failed.
    pub const QUEUE_HANDLER_FAILED: &str = "aiSeat queue handler failed";
}

/// `internalTokenOk(expected, got)`: constant-time; an empty expected token
/// never matches.
pub fn internal_token_ok(expected: &str, got: &str) -> bool {
    if expected.is_empty() || got.len() != expected.len() {
        return false;
    }
    expected.as_bytes().ct_eq(got.as_bytes()).into()
}

/// `profileInternalToken()`: `EUCHRE_INTERNAL_TOKEN` when at least 16 UTF-16
/// units, else [`DEV_PROFILE_TOKEN`] when `RIVET_ENDPOINT` is unset or empty,
/// else `None`.
pub fn profile_internal_token(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(secret) = env("EUCHRE_INTERNAL_TOKEN")
        && crate::js::utf16_len(&secret) >= 16
    {
        return Some(secret);
    }
    let local_dev = env("RIVET_ENDPOINT").is_none_or(|v| v.is_empty());
    local_dev.then(|| DEV_PROFILE_TOKEN.to_owned())
}

/// The environment the handlers run in.
#[derive(Clone, Copy)]
pub struct SeatEnv<'a> {
    /// Wall clock (`Date.now`).
    pub clock: &'a (dyn Clock + Send + Sync),
    /// The actor's log.
    pub log: &'a (dyn SeatLog + Send + Sync),
    /// The model seam.
    pub provider: &'a LlmProvider,
}

impl std::fmt::Debug for SeatEnv<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeatEnv")
            .field("provider", self.provider)
            .finish_non_exhaustive()
    }
}

/// Why a `decide` message was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecideDrop {
    /// Bad token (`meter.tokenRejections` bumped).
    ForgedToken,
    /// Another match's message (silent).
    WrongGame,
    /// Another seat's message (`meter.seatRejections` bumped).
    WrongSeat,
    /// A redelivered turn (`meter.staleTurns` bumped).
    StaleTurn,
    /// Nothing to decide (memory absorbed the view first).
    EmptyLegal,
}

/// An answered decision: persist, then send `decision` on `aiDecision`.
#[derive(Clone, Debug, PartialEq)]
pub struct Answered {
    /// The `aiDecision` body.
    pub decision: AIDecision,
    /// The ladder's outcome.
    pub outcome: LadderOutcome,
}

/// The result of [`on_decide`].
#[derive(Clone, Debug, PartialEq)]
pub enum DecideHandled {
    /// Dropped; nothing to send.
    Dropped(DecideDrop),
    /// Answered; save immediately, then send.
    Answered(Box<Answered>),
}

impl DecideHandled {
    /// `true` when the state changed (the glue persists it).
    pub fn state_changed(&self) -> bool {
        !matches!(self, DecideHandled::Dropped(DecideDrop::WrongGame))
    }
}

fn obj(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// `onDecide(c, body)` up to (not including) the save and the send.
pub async fn on_decide(
    state: &mut AiSeatState,
    body: &AIDecideRequest,
    env: SeatEnv<'_>,
) -> DecideHandled {
    if !internal_token_ok(&state.internal_token, &body.internal_token) {
        state.meter.token_rejections += 1;
        env.log.warn(
            messages::FORGED_DECIDE,
            &obj(&[("seat", json!(state.seat.get()))]),
        );
        return DecideHandled::Dropped(DecideDrop::ForgedToken);
    }
    if body.game_id != state.game_id {
        return DecideHandled::Dropped(DecideDrop::WrongGame);
    }
    if body.seat != state.seat {
        state.meter.seat_rejections += 1;
        env.log.warn(
            messages::WRONG_SEAT,
            &obj(&[
                ("mine", json!(state.seat.get())),
                ("claimed", json!(body.seat.get())),
            ]),
        );
        return DecideHandled::Dropped(DecideDrop::WrongSeat);
    }
    if state.last_turn_id.as_deref() == Some(body.turn_id.as_str()) {
        state.meter.stale_turns += 1;
        return DecideHandled::Dropped(DecideDrop::StaleTurn);
    }

    absorb_public_state(&mut state.memory, &body.view);

    let started_at = env.clock.now_ms();
    let outcome = run_ladder(
        LadderDeps {
            persona: &state.persona,
            dossier: &state.dossier,
            nonce: &state.fence_nonce,
            degraded: state.budget.degraded,
            clock: env.clock,
            log: env.log,
            decider: env.provider.decider.as_deref(),
        },
        body,
    )
    .await;
    let Some(outcome) = outcome else {
        env.log.error(
            messages::EMPTY_LEGAL,
            &obj(&[
                ("seat", json!(state.seat.get())),
                ("turnId", json!(body.turn_id)),
            ]),
        );
        return DecideHandled::Dropped(DecideDrop::EmptyLegal);
    };

    let latency_ms = env.clock.now_ms().saturating_sub(started_at);
    state.last_turn_id = Some(body.turn_id.clone());
    record_decision(RecordDecisionParams {
        state,
        turn_id: &body.turn_id,
        hand_no: body.view.hand_no,
        kind: body.kind,
        outcome: &outcome,
        latency_ms,
    });
    apply_usage(state, outcome.usage);

    let decision = AIDecision {
        internal_token: state.internal_token.clone(),
        game_id: state.game_id.clone(),
        seat: state.seat,
        turn_id: body.turn_id.clone(),
        move_id: outcome.move_id.clone(),
        source: outcome.source,
        banter: None,
        rationale: Some(outcome.rationale.clone()),
        latency_ms,
    };
    DecideHandled::Answered(Box::new(Answered { decision, outcome }))
}

/// One pending line of table talk, started after the decision was sent.
#[derive(Clone, Debug, PartialEq)]
pub struct BanterJob {
    /// The writer's input (hand-stripped view, situation, 1800 ms).
    pub input: LlmBanterInput,
    /// `aiSay` envelope: token.
    pub internal_token: String,
    /// `aiSay` envelope: match.
    pub game_id: String,
    /// `aiSay`: the turn the line belongs to.
    pub turn_id: String,
    /// `aiSay`: the hand.
    pub hand_no: u32,
    /// `${turnId}:talk:${seat}`.
    pub msg_id: String,
}

/// The banter gate after an answered decision: a writer exists, the seat is
/// not degraded, the persona talks at all, one line per hand, trump is set.
/// When it opens, `banter.lastHandNo` is set (save immediately, then spawn
/// [`run_banter_job`]).
pub fn begin_banter(
    state: &mut AiSeatState,
    body: &AIDecideRequest,
    provider: &LlmProvider,
) -> Option<BanterJob> {
    let hand_no = body.view.hand_no;
    let open = provider.banter.is_some()
        && !state.budget.degraded
        && state.persona.chattiness > 0.0
        && state.banter.last_hand_no != i64::from(hand_no)
        && body.view.trump.is_some();
    if !open {
        return None;
    }
    state.banter.last_hand_no = i64::from(hand_no);
    let view = public_only_view(&body.view);
    let situation = situation_line(&view, state.seat);
    Some(BanterJob {
        input: LlmBanterInput {
            seat: state.seat,
            view,
            situation,
            persona: state.persona.clone(),
            dossier: String::new(),
            nonce: state.fence_nonce.clone(),
            budget_ms: BANTER_BUDGET_MS,
        },
        internal_token: state.internal_token.clone(),
        game_id: state.game_id.clone(),
        turn_id: body.turn_id.clone(),
        hand_no,
        msg_id: format!("{}:talk:{}", body.turn_id, state.seat.get()),
    })
}

/// Runs a [`BanterJob`]: the `aiSay` message to send, or `None` (no writer,
/// silence, failure or timeout).
pub async fn run_banter_job(provider: &LlmProvider, job: &BanterJob) -> Option<AiSayMsg> {
    let writer = provider.banter.as_deref()?;
    let text = call_banter_writer(writer, &job.input).await?;
    Some(AiSayMsg {
        internal_token: job.internal_token.clone(),
        game_id: job.game_id.clone(),
        seat: job.input.seat,
        turn_id: job.turn_id.clone(),
        hand_no: Some(job.hand_no),
        msg_id: job.msg_id.clone(),
        delta: None,
        text: Some(text),
        is_final: true,
    })
}

fn lifecycle_ok(state: &AiSeatState, body: &AiSeatLifecycleMessage) -> bool {
    internal_token_ok(&state.internal_token, &body.internal_token) && body.game_id == state.game_id
}

/// `onResetHand`: `resetMemory(handNo ?? memory.handNo + 1)`. Returns whether
/// it applied; the glue saves immediately either way.
pub fn on_reset_hand(state: &mut AiSeatState, body: &AiSeatLifecycleMessage) -> bool {
    if !lifecycle_ok(state, body) {
        return false;
    }
    let next = body
        .hand_no
        .unwrap_or_else(|| state.memory.hand_no.saturating_add(1));
    reset_memory(&mut state.memory, next);
    true
}

/// One episode as sent to `playerProfile.recordEpisodes`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeOut {
    /// `persona.id`.
    pub role: String,
    /// The match.
    pub match_id: String,
    /// The hand.
    pub hand_no: u32,
    /// Epoch ms.
    pub ts: u64,
    /// What it was about.
    pub kind: EpisodeKind,
    /// One short line.
    pub summary: String,
    /// `0..=1`.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub salience: f64,
}

/// The `recordEpisodes` body. (Local until the protocol owns it; see
/// `docs/port/requests/ai-profile-bodies.md`.)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordEpisodesBody {
    /// The deployment-wide profile secret.
    pub internal_token: String,
    /// The buffered episodes, oldest first.
    pub episodes: Vec<EpisodeOut>,
}

/// The `recordTokens` body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordTokensBody {
    /// The deployment-wide profile secret.
    pub internal_token: String,
    /// Match input tokens.
    pub tokens_in: u64,
    /// Match output tokens.
    pub tokens_out: u64,
    /// Match cache reads.
    pub cache_read_tokens: u64,
    /// Match calls.
    pub calls: u64,
}

/// What `flushEpisodes` must do.
#[derive(Clone, Debug, PartialEq)]
pub enum EpisodeFlush {
    /// The buffer was empty.
    Nothing,
    /// No profile key: the buffer was dropped.
    Dropped,
    /// No deployment secret: keep the buffer for the next boundary.
    Deferred,
    /// Send `body` to `playerProfile` at `profile_key`, then call
    /// [`complete_episode_flush`].
    Send {
        /// The profile actor key.
        profile_key: Vec<String>,
        /// The body.
        body: RecordEpisodesBody,
    },
}

/// A `recordTokens` send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenFlush {
    /// The profile actor key.
    pub profile_key: Vec<String>,
    /// The body.
    pub body: RecordTokensBody,
}

/// `flushEpisodes` up to the send.
pub fn prepare_episode_flush(state: &mut AiSeatState, profile_token: Option<&str>) -> EpisodeFlush {
    if state.episode_buffer.is_empty() {
        return EpisodeFlush::Nothing;
    }
    let Some(profile_key) = state.profile_key.clone() else {
        state.episode_buffer.clear();
        return EpisodeFlush::Dropped;
    };
    let Some(token) = profile_token else {
        return EpisodeFlush::Deferred;
    };
    let episodes = state
        .episode_buffer
        .iter()
        .map(|e| EpisodeOut {
            role: state.persona.id.clone(),
            match_id: state.match_id.clone(),
            hand_no: e.hand_no,
            ts: e.ts,
            kind: e.kind,
            summary: e.summary.clone(),
            salience: e.salience,
        })
        .collect();
    EpisodeFlush::Send {
        profile_key,
        body: RecordEpisodesBody {
            internal_token: token.to_owned(),
            episodes,
        },
    }
}

/// After the `recordEpisodes` send: clear on success; on failure keep the
/// buffer and warn with `err` (cut to 200 UTF-16 units).
pub fn complete_episode_flush(
    state: &mut AiSeatState,
    result: Result<(), &str>,
    log: &(dyn SeatLog + Send + Sync),
) {
    match result {
        Ok(()) => state.episode_buffer.clear(),
        Err(err) => log.warn(
            messages::FLUSH_EPISODES_FAILED,
            &obj(&[("err", json!(crate::js::utf16_prefix(err, 200)))]),
        ),
    }
}

/// `flushTokens` up to the send: `None` without a profile key, with nothing
/// spent, or without a deployment secret.
pub fn prepare_token_flush(state: &AiSeatState, profile_token: Option<&str>) -> Option<TokenFlush> {
    let profile_key = state.profile_key.clone()?;
    let b = &state.budget;
    if b.tokens_in == 0 && b.tokens_out == 0 && b.calls == 0 {
        return None;
    }
    let token = profile_token?;
    Some(TokenFlush {
        profile_key,
        body: RecordTokensBody {
            internal_token: token.to_owned(),
            tokens_in: b.tokens_in,
            tokens_out: b.tokens_out,
            cache_read_tokens: b.cache_read_tokens,
            calls: b.calls,
        },
    })
}

/// `onHandEnd` up to the episode send: remember the hand, buffer its episode
/// (capped at 12, oldest dropped). `None` when the message was dropped.
pub fn on_hand_end(
    state: &mut AiSeatState,
    body: &AiSeatLifecycleMessage,
    now_ms: u64,
    profile_token: Option<&str>,
) -> Option<EpisodeFlush> {
    if !lifecycle_ok(state, body) {
        return None;
    }
    let seat = state.seat;
    remember_hand(RememberParams {
        notes: &mut state.notes,
        noted_hands: &mut state.noted_hands,
        snapshot: state.memory.snapshot.as_ref(),
        seat,
    });
    if let Some(episode) = episode_for(EpisodeParams {
        snapshot: state.memory.snapshot.as_ref(),
        seat,
        ts: now_ms,
    }) {
        state.episode_buffer.push(episode);
        while state.episode_buffer.len() > EPISODE_BUFFER_CAP {
            state.episode_buffer.remove(0);
        }
    }
    Some(prepare_episode_flush(state, profile_token))
}

/// What `onGameEnd` must send, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct GameEndFlush {
    /// First: the episodes.
    pub episodes: EpisodeFlush,
    /// Then: the match's token spend.
    pub tokens: Option<TokenFlush>,
}

/// `onGameEnd` up to the sends. `None` when the message was dropped.
pub fn on_game_end(
    state: &mut AiSeatState,
    body: &AiSeatLifecycleMessage,
    profile_token: Option<&str>,
) -> Option<GameEndFlush> {
    if !lifecycle_ok(state, body) {
        return None;
    }
    let episodes = prepare_episode_flush(state, profile_token);
    let tokens = prepare_token_flush(state, profile_token);
    Some(GameEndFlush { episodes, tokens })
}
