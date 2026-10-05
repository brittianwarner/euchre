/**
 * `types.ts` — the shared type block for the euchre rules engine.
 *
 * This module is **types-and-constants only**: it has no imports, no runtime
 * dependencies, and no logic beyond frozen literal tables. It is the definition
 * site for every name the rest of `src/lib/euchre/*` and every consumer (the
 * authoritative table actor, the advisory AI seats, the browser, vitest, and the
 * replay renderer) agrees on.
 *
 * Normative source: `docs/02-GAME-RULES-ENGINE.md`. Where that document and this
 * file disagree, this file is wrong.
 *
 * `RuleError` is deliberately *not* declared here — only the `RuleCode` union is.
 * The error class belongs with `apply()` in `reduce.ts`, the single throw site.
 */

/* -------------------------------------------------------------------------- */
/* Cards                                                                       */
/* -------------------------------------------------------------------------- */

/** The four suits. `S`/`C` are the black pair, `H`/`D` the red pair (see `sameColor`). */
export type Suit = 'S' | 'H' | 'D' | 'C';

/** The six euchre ranks. There is no 2–8 and no joker: the deck is 24 cards. */
export type Rank = '9' | 'T' | 'J' | 'Q' | 'K' | 'A';

/**
 * A card's stable identity — rank then suit, e.g. `"JS"`, `"TC"`, `"9D"`, `"AH"`.
 * Every `CardId` appears exactly once in a `deckOrder` (invariant V10).
 */
export type CardId = `${Rank}${Suit}`;

/* -------------------------------------------------------------------------- */
/* Seats and teams                                                             */
/* -------------------------------------------------------------------------- */

/**
 * A chair at the table, clockwise: `0` South (the human), `1` West, `2` North
 * (the human's partner), `3` East. Chairs are fixed for the life of the match;
 * only *roles* rotate with the dealer.
 */
export type Seat = 0 | 1 | 2 | 3;

/** `teamOf(seat) = seat & 1`. Team `0` is `{0, 2}` — always the human's team. */
export type Team = 0 | 1;

/**
 * A seat's role *relative to the current dealer*:
 * `0` dealer · `1` eldest (left of dealer; bids and leads first) ·
 * `2` dealer's partner · `3` third seat (right of dealer; the cutter).
 */
export type SeatRole = 0 | 1 | 2 | 3;

/* -------------------------------------------------------------------------- */
/* Phases and statuses                                                         */
/* -------------------------------------------------------------------------- */

/**
 * Every node of the phase state machine.
 *
 * `lobby` · `cutting` · `deal` · `bid_round_1` · `dealer_discard` ·
 * `bid_round_2` · `trick_play` · `trick_resolve` · `hand_score` · `game_over`.
 *
 * `deal`, `trick_resolve`, `hand_score` and `game_over` are **engine-only**
 * phases: `turnSeat` is `null` and `legalMoves()` returns `[]` for every seat.
 *
 * Stick-the-dealer is **not** a phase. It is a sub-state of `bid_round_2`
 * (`cfg.stickTheDealer && turnSeat === dealerSeat && passes === 3`) in which
 * `pass` is removed from the legal set; a `pass` arriving anyway is rejected with
 * the distinct `RuleCode` `"dealer_must_call"`, never `"bidding_closed"`.
 */
export type GamePhase =
	| 'lobby'
	| 'cutting'
	| 'deal'
	| 'bid_round_1'
	| 'dealer_discard'
	| 'bid_round_2'
	| 'trick_play'
	| 'trick_resolve'
	| 'hand_score'
	| 'game_over';

/** Match-level status. `legalMoves()` returns `[]` unless this is `"active"`. */
export type GameStatus = 'active' | 'complete' | 'abandoned';

/** The seven rows of the scoring table (§ 5.1). `throw_in` scores nothing. */
export type HandResult = 'point' | 'march' | 'lone_point' | 'lone_march' | 'euchre' | 'throw_in';

/**
 * The complete set of machine codes `apply()` may reject with. Each maps to
 * distinct client copy; in particular `dealer_must_call` is deliberately not
 * `bidding_closed`.
 */
export type RuleCode =
	| 'not_your_turn'
	| 'wrong_phase'
	| 'illegal_move'
	| 'must_follow_suit'
	| 'card_not_in_hand'
	| 'bidding_closed'
	| 'dealer_must_call'
	| 'sitting_out'
	| 'stale_turn';

/* -------------------------------------------------------------------------- */
/* Moves                                                                       */
/* -------------------------------------------------------------------------- */

/** A bidding action. `alone` rides on the winning bid; it is never a separate turn. */
export type Bid =
	| { readonly t: 'pass' }
	| { readonly t: 'orderUp'; readonly alone: boolean }
	| { readonly t: 'call'; readonly suit: Suit; readonly alone: boolean };

/** The full seat-action union accepted by `apply(state, seat, move)`. */
export type Move =
	| Bid
	| { readonly t: 'cut'; readonly cut: boolean }
	| { readonly t: 'discard'; readonly card: CardId }
	| { readonly t: 'play'; readonly card: CardId };

/**
 * The stable string id of a `Move`, shared verbatim by the client hit-test
 * whitelist, the AI's `z.enum`, the heuristic ranking key, the coach, and the
 * replay log — e.g. `pass`, `orderUp+alone`, `call:H`, `cut:no`, `discard:9C`,
 * `play:JS`.
 */
export type LegalMoveId =
	| 'pass'
	| 'orderUp'
	| 'orderUp+alone'
	| `call:${Suit}`
	| `call:${Suit}+alone`
	| 'cut:yes'
	| 'cut:no'
	| `discard:${CardId}`
	| `play:${CardId}`;

/** One offered move: its wire id, its structured form, and its human label. */
export interface LegalMove {
	readonly id: LegalMoveId;
	readonly move: Move;
	/** UI/spoken label, e.g. `"I assist"`, `"Hearts, alone"`, `"Jack of diamonds"`. */
	readonly label: string;
}

/** How a move reached `apply()`. `"auto"` covers timeout and abandon auto-play. */
export type ActionKind = 'human' | 'ai' | 'auto';

/** Where an AI seat's decision came from. */
export type AIDecisionSource = 'llm' | 'fallback' | 'forced';

/** Why an `"auto"` action was synthesised by the server. */
export type AutoReason = 'ai_timeout' | 'ai_unreachable' | 'abandon';

/**
 * An attributed move: what was played, by whom, and on whose behalf. This is the
 * journalled form — `{ seed, moves[] }` replays through `apply()` from these.
 */
export interface PlayerAction {
	readonly seat: Seat;
	readonly id: LegalMoveId;
	readonly move: Move;
	readonly kind: ActionKind;
	/** Present only when `kind === "ai"`. */
	readonly source?: AIDecisionSource;
	/** Present only when `kind === "auto"`. */
	readonly reason?: AutoReason;
}

/* -------------------------------------------------------------------------- */
/* Tricks and bidding records                                                  */
/* -------------------------------------------------------------------------- */

/** One card laid by one seat. */
export interface Play {
	readonly seat: Seat;
	readonly card: CardId;
}

/**
 * A trick, in progress or complete. `ledSuit` is the **effective** suit of the
 * first play (so a lead of the left bower leads trump), `null` before any card is
 * laid. `winnerSeat` is `null` until the trick resolves.
 */
export interface Trick {
	/** 0-based; a hand has exactly five tricks, indices `0..4`. */
	readonly index: number;
	readonly ledSuit: Suit | null;
	readonly plays: readonly Play[];
	readonly winnerSeat: Seat | null;
}

/** One auction entry, kept for the talk log and the AI's bid-history memory. */
export interface BidRecord {
	readonly seat: Seat;
	readonly move: Bid;
	/** The verbatim table line, e.g. `"I assist."`, `"Next."`, `"Turn it down."`. */
	readonly say: string;
}

/* -------------------------------------------------------------------------- */
/* Dealing                                                                     */
/* -------------------------------------------------------------------------- */

/** One packet of the 3-2 / 2-3 alternation: `count` cards to `seat`. */
export interface DealPacket {
	readonly seat: Seat;
	readonly count: number;
}

/**
 * A deal packet as it appears on the choreography channel. `cards` is `null`
 * whenever the packet is not addressed to the receiving seat — `projectSteps()`
 * blanks it, so the animation channel cannot leak the deal (invariant V12).
 */
export interface DealtPacket extends DealPacket {
	readonly cards: readonly CardId[] | null;
}

/* -------------------------------------------------------------------------- */
/* Steps — the choreography channel                                            */
/* -------------------------------------------------------------------------- */

/**
 * A `Step` describes *what just happened* so the client can animate and announce
 * it. Steps are always accompanied by an authoritative `PublicGameView`; they are
 * decoration, never truth.
 *
 * Note `dealerDiscarded` has **no card field**. That is structural: leaking the
 * dealer's discard is a compile error, not a review catch (V12/V13).
 */
export type Step =
	| { readonly t: 'cut'; readonly seat: Seat; readonly cut: boolean }
	| { readonly t: 'dealt'; readonly packets: readonly DealtPacket[] }
	| { readonly t: 'upCardTurned'; readonly card: CardId }
	| { readonly t: 'bid'; readonly seat: Seat; readonly id: LegalMoveId; readonly say: string }
	| { readonly t: 'turnedDown'; readonly suit: Suit }
	| {
			readonly t: 'trumpSet';
			readonly suit: Suit;
			readonly makerSeat: Seat;
			readonly aloneSeat: Seat | null;
			readonly sittingSeat: Seat | null;
			/** `true` when trump came from an accepted up-card, `false` from a round-2 call. */
			readonly viaOrderUp: boolean;
	  }
	| { readonly t: 'dealerDiscarded'; readonly seat: Seat }
	| { readonly t: 'cardPlayed'; readonly seat: Seat; readonly card: CardId }
	| {
			readonly t: 'trickWon';
			readonly index: number;
			readonly winnerSeat: Seat;
			readonly plays: readonly Play[];
			/** Drives the longer 1100 ms read pause when the trick makes the euchre certain. */
			readonly sealsEuchre: boolean;
	  }
	| {
			readonly t: 'handScored';
			readonly result: HandResult;
			readonly delta: readonly [number, number];
			readonly score: readonly [number, number];
			readonly makerSeat: Seat | null;
			readonly aloneSeat: Seat | null;
			readonly tricksWon: readonly [number, number];
	  }
	| { readonly t: 'throwIn' }
	| { readonly t: 'gameWon'; readonly team: Team; readonly score: readonly [number, number] };

/* -------------------------------------------------------------------------- */
/* Configuration                                                               */
/* -------------------------------------------------------------------------- */

/**
 * The rule variants the reducer branches on. Everything typed as a literal
 * (`"24"`, `false`, `"makerOnly"`, `"off"`) has exactly one code path in v1 — the
 * flag exists so the deletion is documented, not so it can be flipped.
 */
export interface EngineConfig {
	/** 24-card deck. The 25-card *benny* variant is out of scope. */
	readonly deckVariant: '24';
	/** Points to win. Default `10`; `5 | 7 | 11` are implemented and tested. */
	readonly gameTo: number;
	/** Default `true`: the dealer may not pass in round 2 once three have passed. */
	readonly stickTheDealer: boolean;
	/** A left bower alone does not qualify to name trump. Stuck dealers are exempt. */
	readonly requireNaturalTrump?: boolean;
	/** Only the maker may go alone, and only on the successful call. */
	readonly loner: 'makerOnly';
	/** Defending alone. Branch exists and is unit-tested; unreachable in v1. */
	readonly defendAlone: false;
	/** The 4-point all-tricks-to-defenders row does not exist in v1. */
	readonly superEuchre: false;
	/** No code path. */
	readonly farmersHand: 'off';
	/** No code path. */
	readonly railroading: false;
	/** No code path. */
	readonly dealerMustHoldTrump: false;
	/** Consecutive throw-ins before the engine speaks a "reshuffling" line. */
	readonly misdealLimit: number;
}

/** The v1 defaults. Frozen so a consumer cannot mutate the shared object. */
export const DEFAULT_ENGINE_CONFIG: EngineConfig = Object.freeze({
	deckVariant: '24',
	gameTo: 10,
	stickTheDealer: true,
	requireNaturalTrump: false,
	loner: 'makerOnly',
	defendAlone: false,
	superEuchre: false,
	farmersHand: 'off',
	railroading: false,
	dealerMustHoldTrump: false,
	misdealLimit: 3
});

/* -------------------------------------------------------------------------- */
/* State                                                                       */
/* -------------------------------------------------------------------------- */

/**
 * Everything that belongs to the hand currently in progress. Reset wholesale by
 * `startHand()`; every collection in it is explicitly bounded (V18) because the
 * whole of `GameState` is deserialized on every actor wake.
 */
export interface HandState {
	/** 0-based hand counter within the match. */
	readonly handNo: number;
	readonly phase: GamePhase;
	readonly dealerSeat: Seat;
	/** `null` in every engine-only phase. */
	readonly turnSeat: Seat | null;

	/** The persisted 24-card permutation. Written *before* `cutting` opens. */
	readonly deckOrder: readonly CardId[];
	/** `null` until the cutter has answered; `true` = bump, `false` = run 'em. */
	readonly cut: boolean | null;

	/** All four hands. Foreign hands never reach a `PublicGameView`. */
	readonly hands: Readonly<Record<Seat, readonly CardId[]>>;
	/** `deckOrder[20..23]`. `kitty[0]` is the up-card; `kitty[1..3]` are never revealed. */
	readonly kitty: readonly [CardId, CardId, CardId, CardId];
	/** `kitty[0]`. Stays publicly identified for the whole hand — never nulled (V14). */
	readonly upCard: CardId | null;
	/** `true` once the dealer has turned it down; distinguishes buried from picked up. */
	readonly upCardTurnedDown: boolean;
	/** The suit that may not be named in round 2. `null` until the turn-down. */
	readonly turnedDownSuit: Suit | null;

	readonly trump: Suit | null;
	readonly makerSeat: Seat | null;
	/** Non-null implies `aloneSeat === makerSeat` and `sittingSeat === partnerOf(aloneSeat)`. */
	readonly aloneSeat: Seat | null;
	/** The loner's partner. Their cards stay in `hands` for the journal but are dead. */
	readonly sittingSeat: Seat | null;
	/** The dealer's face-down discard. Never revealed to any client, ever (V12). */
	readonly dealerDiscard: CardId | null;

	/** Passes *within the current round*; reset to `0` on entry to `bid_round_2`. `≤ 4`. */
	readonly passes: number;
	/** The auction so far. `≤ 8` entries (V18). */
	readonly bids: readonly BidRecord[];

	readonly trick: Trick;
	/** Completed tricks, oldest first. `≤ 5` entries (V18). */
	readonly trickLog: readonly Trick[];
	readonly tricksWon: readonly [number, number];

	/** Set at `hand_score`; `null` while the hand is live. */
	readonly result: HandResult | null;
	/** Post-clamp points awarded by this hand; `null` while the hand is live. */
	readonly delta: readonly [number, number] | null;
}

/** An early AI decision parked behind the persisted pacing floor. */
export interface PendingDecision {
	readonly seat: Seat;
	readonly turnId: string;
	readonly requestedAt: number;
	readonly revealAt: number;
	decision: LegalMoveId | null;
	source: AIDecisionSource | null;
}

/**
 * The whole authoritative match state, held verbatim in the table actor's
 * `c.state`. Fields marked `readonly` are immutable for the life of the match;
 * the reducer never touches `v`, the schedule ids, `pending`, `recentMoveIds` or
 * `appliedSeq` — those belong to the actor's run loop.
 */
export interface GameState {
	readonly schema: 1;
	/** Monotonic version. The actor bumps it; the reducer never reads or writes it. */
	v: number;
	readonly gameId: string;
	/** Minted in `createState`; the actor↔actor authorization boundary. */
	readonly internalToken: string;
	/** The RNG closure is rebuilt from this on every wake, and never persisted. */
	seed: string;
	readonly cfg: EngineConfig;
	/** Drawn once from the seeded RNG at creation and then immutable. */
	readonly firstDealer: Seat;

	status: GameStatus;
	score: readonly [number, number];
	/** 0-based; increments on every scored hand *and* every throw-in. */
	handNo: number;
	/** Consecutive throw-ins. Capped at `cfg.misdealLimit`; reset by any scored hand. */
	misdealStreak: number;
	/** Set exactly when a team reaches `cfg.gameTo`. */
	winnerTeam: Team | null;

	hand: HandState;

	/** Server turn nonce, regenerated on every advance. Stale submits are rejected. */
	turnId: string;
	turnDeadlineAt: number | null;
	timerId: string | null;
	watchdogId: string | null;
	revealId: string | null;
	pending: PendingDecision | null;
	/** Ring buffer for client double-click dedupe. Cap 32 (V18). */
	recentMoveIds: string[];
	/** Server idempotency; lives in the same `c.state` as the mutation it guards. */
	appliedSeq: number;
}

/* -------------------------------------------------------------------------- */
/* Projection                                                                  */
/* -------------------------------------------------------------------------- */

/**
 * The **only** shape ever sent to a client, produced solely by `project(state, seat)`.
 *
 * Redaction here is structural, not procedural: there is no field capable of
 * holding a foreign hand, a buried kitty card, or the dealer's discard, so a leak
 * is a compile error (V12). `kittyCount` is a function of *whether* a discard
 * exists, never of which card it was (V13), and `upCard` is never nulled (V14).
 */
export interface PublicGameView {
	/** Actor-owned accessibility preference; omitted by the pure rules projection. */
	readonly reviewTricks?: boolean;
	readonly rules?: { readonly stickTheDealer: boolean; readonly requireNaturalTrump: boolean };
	/** True only after the server-held read pause has elapsed. */
	readonly awaitingTrickReview?: boolean;
	/** The state version this view was cut from; the client drops `v <= seen`. */
	readonly v: number;
	/** The seat this view was projected for. */
	readonly you: Seat;
	readonly status: GameStatus;
	readonly phase: GamePhase;
	readonly handNo: number;
	readonly dealerSeat: Seat;
	readonly turnSeat: Seat | null;
	/** Echoed back on `submitMove`; a mismatch is rejected as `stale_turn`. */
	readonly turnId: string;
	readonly score: readonly [number, number];

	readonly trump: Suit | null;
	readonly makerSeat: Seat | null;
	readonly aloneSeat: Seat | null;
	readonly sittingSeat: Seat | null;
	readonly upCard: CardId | null;
	readonly upCardTurnedDown: boolean;
	readonly turnedDownSuit: Suit | null;

	/** Your own cards, and only ever your own. Empty for the sitting seat (V8). */
	readonly hand: readonly CardId[];
	/** Card counts by seat, including your own. `handCounts[sittingSeat] === 0`. */
	readonly handCounts: readonly [number, number, number, number];
	/** `3 + (upCardTurnedDown || dealerDiscard !== null ? 1 : 0)`. */
	readonly kittyCount: number;

	readonly trick: Trick;
	readonly trickLog: readonly Trick[];
	readonly tricksWon: readonly [number, number];
	readonly passes: number;
	readonly bids: readonly BidRecord[];

	/** Empty unless `turnSeat === you`. Never empty when it is (V19). */
	readonly legal: readonly LegalMove[];

	readonly result: HandResult | null;
	readonly delta: readonly [number, number] | null;
	readonly winnerTeam: Team | null;
}
