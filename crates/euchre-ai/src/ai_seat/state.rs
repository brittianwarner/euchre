//! `aiSeat` durable state (`ai-seat/types.ts` + `createState`), serialized in TS
//! key order so a snapshot round-trips byte for byte.

use euchre_engine::{Card, HandResult, Seat, Suit, Team, team_of};
use euchre_protocol::serde_util::js_f64;
use euchre_protocol::{
    AIDecisionKind, AiDecisionSource, Difficulty, EpisodeKind, LegalMoveId, MODEL_BID, MODEL_PLAY,
    PersonaConfig,
};
use serde::{Deserialize, Serialize};

/// Publicly shown voids per seat (`Record<Seat, Suit[]>`, keys `"0"`..`"3"`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatVoids {
    /// Seat 0.
    #[serde(rename = "0")]
    pub s0: Vec<Suit>,
    /// Seat 1.
    #[serde(rename = "1")]
    pub s1: Vec<Suit>,
    /// Seat 2.
    #[serde(rename = "2")]
    pub s2: Vec<Suit>,
    /// Seat 3.
    #[serde(rename = "3")]
    pub s3: Vec<Suit>,
}

impl SeatVoids {
    /// The voids of one seat.
    pub fn get(&self, seat: Seat) -> &[Suit] {
        match seat.get() {
            0 => &self.s0,
            1 => &self.s1,
            2 => &self.s2,
            _ => &self.s3,
        }
    }

    /// The voids of one seat, mutably.
    pub fn get_mut(&mut self, seat: Seat) -> &mut Vec<Suit> {
        match seat.get() {
            0 => &mut self.s0,
            1 => &mut self.s1,
            2 => &mut self.s2,
            _ => &mut self.s3,
        }
    }
}

/// The public facts about the hand in progress (`HandSnapshot`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandSnapshot {
    /// The hand.
    pub hand_no: u32,
    /// Trump, once named.
    pub trump: Option<Suit>,
    /// The maker.
    pub maker_seat: Option<Seat>,
    /// The lone maker.
    pub alone_seat: Option<Seat>,
    /// Tricks by team.
    pub tricks_won: [u8; 2],
    /// The scored result.
    pub result: Option<HandResult>,
}

/// Per-hand card-counting memory (`AiSeatHandMemory`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeatHandMemory {
    /// The hand this memory describes.
    pub hand_no: u32,
    /// Trump seen this hand (cap `MEM_TRUMP_SEEN_CAP`).
    pub trump_seen: Vec<Card>,
    /// Publicly shown voids (cap `MEM_VOID_SUITS_CAP` per seat).
    pub voids: SeatVoids,
    /// Cards led this hand (cap `MEM_LEAD_CAP`).
    pub lead_history: Vec<Card>,
    /// Auction lines (cap `MEM_BID_CAP`).
    pub bid_history: Vec<String>,
    /// The latest snapshot; the note source.
    pub snapshot: Option<HandSnapshot>,
}

/// One buffered cross-game memory (`AiSeatEpisode`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeatEpisode {
    /// What it was about.
    pub kind: EpisodeKind,
    /// At most `NOTE_MAX_CHARS`.
    pub summary: String,
    /// `0`-`1`.
    #[serde(with = "js_f64")]
    pub salience: f64,
    /// The hand.
    pub hand_no: u32,
    /// Caller-supplied timestamp.
    pub ts: u64,
}

/// Model spend for this seat in this match (`AiSeatBudget`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeatBudget {
    /// Prompt tokens.
    pub tokens_in: u64,
    /// Completion tokens.
    pub tokens_out: u64,
    /// Cache reads.
    pub cache_read_tokens: u64,
    /// Calls that spent tokens.
    pub calls: u64,
    /// Heuristic-only for the rest of the match.
    pub degraded: bool,
}

/// Decision counters (`AiSeatMeter`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeatMeter {
    /// Decisions answered.
    pub decisions: u64,
    /// Model calls.
    pub llm_calls: u64,
    /// Decisions the model won.
    pub llm_wins: u64,
    /// Heuristic fallbacks.
    pub fallbacks: u64,
    /// Forced moves.
    pub forced: u64,
    /// Rejected attempts.
    pub illegal_attempts: u64,
    /// First-legal rescues.
    pub first_legal_rescues: u64,
    /// Bad-token drops.
    pub token_rejections: u64,
    /// Wrong-seat drops.
    pub seat_rejections: u64,
    /// Redelivered turns.
    pub stale_turns: u64,
    /// Banter lines shipped.
    pub banter_emitted: u64,
    /// Banter lines rejected.
    pub banter_rejected: u64,
    /// Mean of `samples`.
    #[serde(with = "js_f64")]
    pub avg_latency_ms: f64,
    /// p95 of `samples`.
    #[serde(with = "js_f64")]
    pub p95_latency_ms: f64,
    /// Rolling latency samples (cap `LATENCY_SAMPLE_CAP`).
    pub samples: Vec<u64>,
}

/// Banter rate-limiter state (`AiSeatBanterGate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeatBanterGate {
    /// `-1` until the first line.
    pub last_hand_no: i64,
    /// `-1` until the first line.
    pub last_trick_index: i64,
    /// Lines this trick.
    pub in_trick: u32,
    /// Lines this hand.
    pub in_hand: u32,
    /// Timestamp of the last line.
    pub last_at: u64,
}

/// One debug decision-log entry (`DecisionRecord`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRecord {
    /// The turn.
    pub turn_id: String,
    /// The hand.
    pub hand_no: u32,
    /// The decision kind.
    pub kind: AIDecisionKind,
    /// The chosen move.
    pub move_id: LegalMoveId,
    /// Where it came from.
    pub source: AiDecisionSource,
    /// Wall-clock latency.
    pub latency_ms: u64,
    /// Candidate-set size.
    pub candidates: usize,
    /// The screened rationale.
    pub rationale: String,
}

/// The whole of `aiSeat`'s `c.state` (`AiSeatState`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeatState {
    /// Always `1`.
    pub schema: u8,
    /// `false` until create input arrived.
    pub provisioned: bool,
    /// The match.
    pub game_id: String,
    /// The journalling match id.
    pub match_id: String,
    /// The chair.
    pub seat: Seat,
    /// The chair's team.
    pub team_id: Team,
    /// Actor-to-actor secret. Never logged.
    pub internal_token: String,
    /// Fence nonce for untrusted persona text.
    pub fence_nonce: String,
    /// The snapshotted persona.
    pub persona: PersonaConfig,
    /// Semi-trusted dossier.
    pub dossier: String,
    /// `playerProfile` key, when wired.
    pub profile_key: Option<Vec<String>>,
    /// Per-hand memory.
    pub memory: AiSeatHandMemory,
    /// Cross-hand notes (cap `MEM_NOTE_CAP`).
    pub notes: Vec<String>,
    /// Hands already noted.
    pub noted_hands: Vec<u32>,
    /// Match spend.
    pub budget: AiSeatBudget,
    /// Counters.
    pub meter: AiSeatMeter,
    /// Banter gate.
    pub banter: AiSeatBanterGate,
    /// The last answered turn.
    pub last_turn_id: Option<String>,
    /// Buffered episodes (cap `EPISODE_BUFFER_CAP`).
    pub episode_buffer: Vec<AiSeatEpisode>,
    /// Debug log (cap `DECISION_LOG_CAP`).
    pub decision_log: Vec<DecisionRecord>,
}

/// What the table hands an AI seat at create (`AiSeatCreateInput`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeatCreateInput {
    /// The match.
    pub game_id: String,
    /// Advisory; the key wins.
    pub seat: Seat,
    /// The table's internal token.
    pub internal_token: String,
    /// The fence nonce.
    pub fence_nonce: String,
    /// The persona snapshot.
    pub persona: PersonaConfig,
    /// Rendered cross-game episodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dossier: Option<String>,
    /// `playerProfile`'s key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_key: Option<Vec<String>>,
    /// Defaults to `game_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_id: Option<String>,
}

/// `blankPersona()`: the placeholder for a seat woken with no create input.
pub fn blank_persona() -> PersonaConfig {
    PersonaConfig {
        id: "unprovisioned".to_owned(),
        version: 0,
        name: "Seat".to_owned(),
        blurb: String::new(),
        prompt: String::new(),
        house_prompt: String::new(),
        difficulty: Difficulty::Casual,
        aggression: 0.5,
        risk: 0.5,
        chattiness: 0.0,
        temperature: 0.5,
        model_id: MODEL_PLAY,
        bid_model_id: MODEL_BID,
    }
}

/// The key did not name seat 1, 2 or 3.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("aiSeat must be keyed ['table', gameId, 'seat', '1'|'2'|'3']; got {0}")]
pub struct SeatKeyError(pub String);

/// `seatFromKey(key)`.
pub fn seat_from_key(key: &[String]) -> Result<Seat, SeatKeyError> {
    let seat = match key.get(3).map(String::as_str) {
        Some("1") => Seat::new(1),
        Some("2") => Seat::new(2),
        Some("3") => Seat::new(3),
        _ => None,
    };
    seat.ok_or_else(|| SeatKeyError(serde_json::to_string(key).unwrap_or_default()))
}

/// Parameters of [`create_state`].
#[derive(Clone, Copy, Debug)]
pub struct CreateStateParams<'a> {
    /// The actor key `["table", gameId, "seat", n]`.
    pub key: &'a [String],
    /// The create input, when the table supplied one.
    pub input: Option<&'a AiSeatCreateInput>,
}

/// `createState(c, input)`.
pub fn create_state(params: CreateStateParams<'_>) -> Result<AiSeatState, SeatKeyError> {
    let seat = seat_from_key(params.key)?;
    let input = params.input;
    let game_id = input
        .map(|i| i.game_id.clone())
        .or_else(|| params.key.get(1).cloned())
        .unwrap_or_default();
    Ok(AiSeatState {
        schema: 1,
        provisioned: input.is_some(),
        match_id: input
            .and_then(|i| i.match_id.clone())
            .unwrap_or_else(|| game_id.clone()),
        game_id,
        seat,
        team_id: team_of(seat),
        internal_token: input.map(|i| i.internal_token.clone()).unwrap_or_default(),
        fence_nonce: input.map(|i| i.fence_nonce.clone()).unwrap_or_default(),
        persona: input.map_or_else(blank_persona, |i| i.persona.clone()),
        dossier: input.and_then(|i| i.dossier.clone()).unwrap_or_default(),
        profile_key: input.and_then(|i| i.profile_key.clone()),
        memory: AiSeatHandMemory::default(),
        notes: Vec::new(),
        noted_hands: Vec::new(),
        budget: AiSeatBudget::default(),
        meter: AiSeatMeter::default(),
        banter: AiSeatBanterGate {
            last_hand_no: -1,
            last_trick_index: -1,
            in_trick: 0,
            in_hand: 0,
            last_at: 0,
        },
        last_turn_id: None,
        episode_buffer: Vec::new(),
        decision_log: Vec::new(),
    })
}
