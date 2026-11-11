//! Stored history written to `playerProfile` (protocol/index.ts section 6).
//!
//! `MatchRecord` and `HandJournalEntry` are **server-only**: `seed` plus
//! `deckOrder` plus `moves` reconstructs every hidden card. The client-facing row
//! is [`MatchSummary`], which structurally has neither.

use euchre_engine::{
    CardId, EngineConfig, GameStatus, HandResult, PlayerAction, SchemaV1, Seat, Suit, Team,
};
use serde::{Deserialize, Serialize};

use crate::persona::PersonaAssignment;
use crate::serde_util::{js_f64, string_enum};

string_enum! {
    /// How a match ended for the human (`MatchOutcome`).
    MatchOutcome {
        /// Team 0 won.
        Won => "won",
        /// Team 1 won.
        Lost => "lost",
        /// Left unfinished.
        Abandoned => "abandoned",
    }
}

/// Per-match counters (`MatchStats`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchStats {
    /// Euchres scored by team 0.
    pub euchres_for: u32,
    /// Euchres suffered by team 0.
    pub euchres_against: u32,
    /// Loners attempted by anyone.
    pub loners_attempted: u32,
    /// Loners that took all five.
    pub loners_made: u32,
    /// Hands where the makers took all five.
    pub marches: u32,
    /// Hands thrown in.
    pub throw_ins: u32,
    /// Tricks per team.
    pub tricks: [u32; 2],
}

/// Model token usage (`TokenUsage`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    /// Input tokens.
    pub tokens_in: u64,
    /// Output tokens.
    pub tokens_out: u64,
    /// Cache-read input tokens.
    pub cache_read_tokens: u64,
    /// Model calls.
    pub calls: u64,
}

/// One match, server-only (`MatchRecord`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchRecord {
    /// Always `1`.
    pub schema: SchemaV1,
    /// The game id.
    pub match_id: String,
    /// The owner.
    pub user_id: String,
    /// The deal seed (server-only).
    pub seed: String,
    /// The effective rules.
    pub cfg: EngineConfig,
    /// The first dealer.
    pub first_dealer: Seat,
    /// Epoch ms.
    pub started_at: u64,
    /// Epoch ms, or `null` while live.
    pub ended_at: Option<u64>,
    /// The match status.
    pub status: GameStatus,
    /// The human's outcome.
    pub outcome: MatchOutcome,
    /// Final or current score.
    pub score: [u8; 2],
    /// The winner, or `null`.
    pub winner_team: Option<Team>,
    /// Hands completed.
    pub hands_played: u32,
    /// The opponents snapshot.
    pub personas: Vec<PersonaAssignment>,
    /// Counters.
    pub stats: MatchStats,
    /// Model usage.
    pub tokens: TokenUsage,
}

/// One hand's replay journal, server-only (`HandJournalEntry`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandJournalEntry {
    /// The game id.
    pub match_id: String,
    /// The hand.
    pub hand_no: u32,
    /// The deal seed (server-only).
    pub seed: String,
    /// The dealer.
    pub dealer_seat: Seat,
    /// The full shuffled deck (server-only).
    pub deck_order: Vec<CardId>,
    /// Every action, in order.
    pub moves: Vec<PlayerAction>,
    /// Trump, or `null` for a throw-in.
    pub trump: Option<Suit>,
    /// The maker, or `null`.
    pub maker_seat: Option<Seat>,
    /// The loner, or `null`.
    pub alone_seat: Option<Seat>,
    /// Tricks per team.
    pub tricks_won: [u8; 2],
    /// The result, or `null`.
    pub result: Option<HandResult>,
    /// Points per team, or `null`.
    pub delta: Option<[u8; 2]>,
    /// Epoch ms.
    pub ended_at: u64,
}

/// The client-safe match row (`MatchSummary`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchSummary {
    /// The game id.
    pub match_id: String,
    /// Epoch ms.
    pub played_at: u64,
    /// The match status.
    pub status: GameStatus,
    /// The human's outcome.
    pub outcome: MatchOutcome,
    /// Final score.
    pub score: [u8; 2],
    /// Hands completed.
    pub hands_played: u32,
    /// Opponent names.
    pub opponents: Vec<String>,
}

/// A page of match rows (`MatchListPage`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchListPage {
    /// The rows.
    pub rows: Vec<MatchSummary>,
    /// Opaque cursor for the next page, or `null`.
    pub cursor: Option<String>,
}

/// Lifetime totals (`LifetimeStats`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifetimeStats {
    /// Matches played.
    pub matches: u32,
    /// Matches won.
    pub won: u32,
    /// Matches lost.
    pub lost: u32,
    /// Matches abandoned.
    pub abandoned: u32,
    /// Euchres scored.
    pub euchres_for: u32,
    /// Euchres suffered.
    pub euchres_against: u32,
    /// Loners made.
    pub loners_made: u32,
    /// Hands played.
    pub hands_played: u32,
    /// Epoch ms.
    pub updated_at: u64,
}

/// The profile's small state row (`PlayerProfileState`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerProfileState {
    /// Always `1`.
    pub schema: SchemaV1,
    /// The live match, or `null`.
    pub active_game_id: Option<String>,
    /// Today's model usage.
    pub daily_tokens: TokenUsage,
    /// Lifetime totals.
    pub lifetime: LifetimeStats,
}

string_enum! {
    /// What a remembered episode was about (`EpisodeKind`).
    EpisodeKind {
        /// A bid.
        Bid => "bid",
        /// A play.
        Play => "play",
        /// A loner.
        Loner => "loner",
        /// A euchre.
        Euchre => "euchre",
        /// A read on the human.
        Read => "read",
    }
}

/// One persona memory (`EpisodeRecord`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeRecord {
    /// The persona role id.
    pub role: String,
    /// The game id.
    pub match_id: String,
    /// The hand.
    pub hand_no: u32,
    /// Epoch ms.
    pub ts: u64,
    /// What it was about.
    pub kind: EpisodeKind,
    /// One short line.
    pub summary: String,
    /// 0..=1.
    #[serde(with = "js_f64")]
    pub salience: f64,
    /// Epoch ms.
    pub expires_at: u64,
}
