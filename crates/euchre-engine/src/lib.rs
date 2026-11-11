//! Pure, deterministic euchre rules engine: a 1:1 port of the TS engine in
//! `src/lib/euchre`. No time, randomness, IO or env access: every hand is a
//! function of `(seed, hand_no)` and every transition of `(state, seat, move)`.
//!
//! The crate root re-exports the public surface of the TS `index.ts` (spec 01
//! section 14). Names are snake_case equivalents of the TS exports.

pub mod cards;
pub mod deal;
pub mod invariants;
pub mod legal;
pub mod optimistic;
pub mod project;
pub mod reduce;
pub mod score;
pub mod seats;
pub mod trick;
pub mod types;

pub use cards::{
    CARD_IDS, DECK, RANKS, SUITS, bowers_of, color_mate, effective_suit, is_left_bower, plain_rank,
    rank_of, same_color, suit_of, trump_rank,
};
pub use deal::{
    DealError, DealHandParams, DealResult, DealtHands, Mulberry32, assert_deck_order, cut_at,
    cut_for_hand, deal_from, deal_hand, deal_plan, dealt_packets, fnv1a, hand_rng, hash_seed,
    mulberry32, new_deck, pick_first_dealer, shuffle, shuffle_for_hand, starts_with_three,
    to_base36,
};
pub use invariants::{InvariantError, assert_invariants, assert_legal_non_empty};
pub use legal::{
    RANK_NAME, SUIT_NAME, SUIT_NOUN, WhyIllegalParams, card_name, card_name_lower, find_legal_move,
    get_legal_plays, is_stuck_dealer, legal_move_ids, legal_moves, order_label, rank_name,
    suit_name, suit_noun, why_illegal,
};
pub use optimistic::{OPTIMISTIC_FIELDS, OptimisticField, apply_optimistic};
pub use project::{project, project_steps};
pub use reduce::{
    ApplyError, ApplyOptions, ApplyResult, EngineFault, GameInit, RuleError, advance, apply,
    create_game, move_id, new_game, say_for_bid, start_hand, try_advance, try_apply,
    try_create_game, try_start_hand,
};
pub use score::{
    ScoreHandParams, ScoreOutcome, apply_delta, is_euchre, is_game_over, is_in_the_barn,
    raw_points, score_hand, throw_in_outcome, winner_team,
};
pub use seats::{
    HUMAN_SEAT, SEATS, active_seat_count, bidding_order, cutter_of, eldest_of, first_active_from,
    is_active_seat, next_active_seat, next_seat, opponent_team, partner_of, seat_role, team_of,
};
pub use trick::{
    EmptyTrickError, card_value, currently_winning, is_trick_complete, led_suit_of,
    plays_per_trick, trick_winner,
};
pub use types::{
    AIDecisionSource, ActionKind, AiDecisionSource, AutoReason, Bid, BidRecord, Card, CardId,
    DEFAULT_ENGINE_CONFIG, DealPacket, DealtPacket, DeckVariant, EngineConfig,
    EngineConfigOverrides, FarmersHand, GamePhase, GameState, GameStatus, HandResult, HandState,
    Hands, LegalMove, LegalMoveId, LonerRule, Move, ObservationKind, ObservationTiming, OutOfRange,
    ParseCardError, ParseEnumError, PendingDecision, Play, PlayerAction, PublicGameView, Rank,
    RuleCode, SchemaV1, Seat, SeatRole, Step, Suit, TableObservation, TablePlayer, TableRules,
    Team, Trick,
};
