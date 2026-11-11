//! The euchre wire contract (spec 02): every payload that crosses an actor or
//! client boundary, with JSON byte-compatible with the TS app, plus the pure
//! client [`TableStore`] state machine (02 section 6).
//!
//! The crate re-exports the engine's wire vocabulary so clients depend on
//! `euchre-protocol` alone. `GameState` is deliberately **not** re-exported: it
//! holds all four hands and the buried kitty, and lives only inside the table
//! actor. If a file that imports this crate needs it, that file is on the wrong
//! side of the redaction boundary.
//!
//! JSON conventions (02 section 13): fields are camelCase; a TS `T | null` is an
//! `Option<T>` that is always written (as `null` when `None`); a TS `T?` is an
//! `Option<T>` that is omitted when `None` and accepts absent or `null` on read.

pub mod actions;
pub mod ai;
pub mod conn;
pub mod errors;
pub mod events;
pub mod hand_review;
pub mod internal;
pub mod pacing;
pub mod persona;
pub mod profile;
pub mod records;
pub mod serde_util;
pub mod store;
pub mod wire;

pub use euchre_engine::{
    AIDecisionSource, ActionKind, AiDecisionSource, AutoReason, Bid, BidRecord, Card, CardId,
    DealPacket, DealtPacket, EngineConfig, EngineConfigOverrides, GamePhase, GameStatus,
    HandResult, LegalMove, LegalMoveId, Move, ObservationKind, ObservationTiming, PendingDecision,
    Play, PlayerAction, PublicGameView, Rank, RuleCode, SchemaV1, Seat, SeatRole, Step, Suit,
    TableObservation, TablePlayer, TableRules, Team, Trick,
};

pub use actions::*;
pub use ai::*;
pub use conn::*;
pub use errors::*;
pub use events::*;
pub use hand_review::*;
pub use internal::*;
pub use pacing::*;
pub use persona::*;
pub use records::*;
pub use serde_util::True;
pub use store::{
    ChatLine, HandReviewOutcome, HandReviewPanel, LoadingNotice, OptimisticView, StoreCommand,
    StoreEffects, StoreEvent, StoreStatus, SubmitOutcome, TableStore, TableTalk,
};
pub use wire::{WireView, serialize_view, view_to_value};

/// The wire-contract version (`PROTOCOL_VERSION`). Bump it when a payload here
/// changes shape in a way an older client cannot read. It is not `GameState.v`.
pub const PROTOCOL_VERSION: u32 = 1;
