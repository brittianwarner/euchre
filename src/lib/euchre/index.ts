/**
 * `$lib/euchre` — the complete euchre rules engine, and the only public surface
 * anything else in the app is allowed to import.
 *
 * Zero runtime dependencies. Pure, deterministic, and framework-free: the same
 * module is imported unchanged by the authoritative table actor, the advisory AI
 * seats, the browser (legality preview, optimistic play, coach hints), vitest,
 * and the replay renderer. Nothing in here touches the network, the clock, or a
 * random number generator — time and randomness enter only as arguments.
 *
 * The three entry points that write state are {@link createGame},
 * {@link apply} and {@link advance} (plus {@link startHand}), all of them in
 * `reduce.ts`. No euchre rule is implemented anywhere else in the codebase: one
 * rulebook, one writer.
 *
 * The two entry points that *leave* the server are {@link project} and
 * {@link projectSteps}. Everything a client ever sees passes through one of
 * them.
 *
 * Normative source: `docs/02-GAME-RULES-ENGINE.md`.
 */

import { legalMoves } from './legal';
import { createGame } from './reduce';
import type {
	CardId,
	DealtPacket,
	EngineConfig,
	GameState,
	PublicGameView,
	Seat,
	Step
} from './types';

/* -------------------------------------------------------------------------- */
/* Types                                                                       */
/* -------------------------------------------------------------------------- */

export type {
	ActionKind,
	AIDecisionSource,
	AutoReason,
	Bid,
	BidRecord,
	CardId,
	DealPacket,
	DealtPacket,
	EngineConfig,
	GamePhase,
	GameState,
	GameStatus,
	HandResult,
	HandState,
	LegalMove,
	LegalMoveId,
	Move,
	PendingDecision,
	Play,
	PlayerAction,
	PublicGameView,
	Rank,
	RuleCode,
	Seat,
	SeatRole,
	Step,
	Suit,
	Team,
	Trick
} from './types';

export { DEFAULT_ENGINE_CONFIG } from './types';

/* -------------------------------------------------------------------------- */
/* Primitives                                                                  */
/* -------------------------------------------------------------------------- */

/** Cards: identity, colour, and the bower-aware ranking primitives. */
export {
	CARD_IDS,
	DECK,
	RANKS,
	SUITS,
	bowersOf,
	effectiveSuit,
	isLeftBower,
	plainRank,
	rankOf,
	sameColor,
	suitOf,
	trumpRank
} from './cards';

/** Seat arithmetic, team membership, and loner-aware turn order. */
export {
	HUMAN_SEAT,
	SEATS,
	activeSeatCount,
	biddingOrder,
	cutterOf,
	eldestOf,
	firstActiveFrom,
	isActiveSeat,
	nextActiveSeat,
	nextSeat,
	opponentTeam,
	partnerOf,
	seatRole,
	teamOf
} from './seats';

/** Seeded determinism and the physical deal. */
export type { DealResult } from './deal';
export {
	assertDeckOrder,
	cutAt,
	cutForHand,
	dealFrom,
	dealHand,
	dealPlan,
	dealtPackets,
	fnv1a,
	handRng,
	hashSeed,
	mulberry32,
	newDeck,
	pickFirstDealer,
	shuffle,
	shuffleForHand,
	startsWithThree
} from './deal';

/** What may happen next: the follow-suit primitive and the move-level API. */
export {
	RANK_NAME,
	SUIT_NAME,
	SUIT_NOUN,
	cardName,
	cardNameLower,
	findLegalMove,
	getLegalPlays,
	isStuckDealer,
	legalMoveIds,
	legalMoves,
	orderLabel,
	whyIllegal
} from './legal';

/** The total order inside one trick, and who takes it. */
export {
	cardValue,
	currentlyWinning,
	isTrickComplete,
	ledSuitOf,
	playsPerTrick,
	trickWinner
} from './trick';

/** The scoring table, the clamp, and game end. */
export type { ScoreOutcome } from './score';
export {
	applyDelta,
	isEuchre,
	isGameOver,
	isInTheBarn,
	rawPoints,
	scoreHand,
	throwInOutcome,
	winnerTeam
} from './score';

/* -------------------------------------------------------------------------- */
/* The reducer                                                                 */
/* -------------------------------------------------------------------------- */

export type { ApplyOptions, ApplyResult, GameInit } from './reduce';
export {
	RuleError,
	advance,
	apply,
	createGame,
	isRuleError,
	moveId,
	sayForBid,
	startHand
} from './reduce';

/* -------------------------------------------------------------------------- */
/* Convenience construction                                                    */
/* -------------------------------------------------------------------------- */

/**
 * A match from nothing but a seed. The `gameId` is derived from the seed, so two
 * calls with the same seed produce byte-identical states — which is exactly what
 * a fixture, a replay and a property test want.
 *
 * Production callers should prefer {@link createGame}, which takes a real
 * `gameId` and a cryptographically random `internalToken`.
 *
 * The returned state is in `lobby`: call {@link advance} once to shuffle, open
 * the first hand, and offer the cut.
 */
export function newGame(seed: string, cfg?: Partial<EngineConfig>): GameState {
	return createGame({ gameId: `g-${seed}`, seed, cfg });
}

/* -------------------------------------------------------------------------- */
/* Projection — the security boundary                                          */
/* -------------------------------------------------------------------------- */

/**
 * Cut the one and only view a client is ever allowed to see.
 *
 * **This function is a security boundary. Read this before you touch it.**
 *
 * It must be *impossible* for `project` to return another seat's cards, any of
 * the three buried kitty cards, or the dealer's face-down discard — not now, not
 * in a debug build, not in a post-game replay, not "just for the AI", not behind
 * a flag. That guarantee is structural rather than procedural: `PublicGameView`
 * has **no field capable of holding** a foreign hand, a buried kitty card or the
 * discard, so a leak is a compile error rather than a review catch (V12).
 *
 * The three rules that keep it that way:
 *
 * 1. **Never widen `PublicGameView`.** If a new field would be able to carry a
 *    `CardId` the receiving seat is not entitled to, the answer is no. There is
 *    no field to blank, so there is no blanking to forget.
 * 2. **`kittyCount` is a function of _whether_ a discard exists, never of which
 *    card it was** — `3 + (upCardTurnedDown || dealerDiscard !== null ? 1 : 0)`.
 *    Anything finer can be differenced against the visible up-card to identify
 *    the discard (V13). Two states that differ only in which card the dealer
 *    discarded must project identically.
 * 3. **`upCard` is never nulled** (V14). Every player saw it, it is load-bearing
 *    for counting the seven trump, and it stays publicly identified for the whole
 *    hand — including when the dealer took it into hand and including when it is
 *    the card they discarded. `upCardTurnedDown` is what distinguishes "buried"
 *    from "picked up".
 *
 * The sitting partner under a loner gets an empty `hand` and a `handCounts` entry
 * of `0` from the moment the call lands (V8); their five cards stay in
 * `state.hand.hands` for the journal and the teaching replay, and are dead.
 *
 * `legal` is empty unless `turnSeat === you`, and is never empty when it is
 * (V19). Pure: `state` is not mutated, and every array in the result is fresh.
 */
export function project(state: GameState, seat: Seat): PublicGameView {
	const h = state.hand;
	const isSittingOut = seat === h.sittingSeat;
	const count = (s: Seat): number => (s === h.sittingSeat ? 0 : h.hands[s].length);

	return {
		v: state.v,
		you: seat,
		status: state.status,
		phase: h.phase,
		handNo: h.handNo,
		dealerSeat: h.dealerSeat,
		turnSeat: h.turnSeat,
		turnId: state.turnId,
		score: [state.score[0], state.score[1]],

		trump: h.trump,
		makerSeat: h.makerSeat,
		aloneSeat: h.aloneSeat,
		sittingSeat: h.sittingSeat,
		upCard: h.upCard,
		upCardTurnedDown: h.upCardTurnedDown,
		turnedDownSuit: h.turnedDownSuit,

		// Your own cards, and only ever your own — indexed by the seat the caller
		// asked for, never by anything read out of the state.
		hand: isSittingOut ? [] : h.hands[seat].slice(),
		handCounts: [count(0), count(1), count(2), count(3)],
		kittyCount: 3 + (h.upCardTurnedDown || h.dealerDiscard !== null ? 1 : 0),

		trick: { ...h.trick, plays: h.trick.plays.slice() },
		trickLog: h.trickLog.map((t) => ({ ...t, plays: t.plays.slice() })),
		tricksWon: [h.tricksWon[0], h.tricksWon[1]],
		passes: h.passes,
		bids: h.bids.slice(),

		legal: legalMoves(state, seat),

		result: h.result,
		delta: h.delta === null ? null : [h.delta[0], h.delta[1]],
		winnerTeam: state.winnerTeam
	};
}

/**
 * Redact a transition's choreography for one seat. The **only** producer of
 * client-visible `Step[]`, and the second half of the {@link project} security
 * boundary.
 *
 * The animation channel is the most natural place in the whole system to leak
 * four hands: `dealt` is the one step that carries card identity for seats other
 * than the receiver, so every packet not addressed to `seat` has its `cards`
 * blanked to `null` — the count survives, so the deal still animates faithfully.
 *
 * Every other step is public by construction. `dealerDiscarded` in particular has
 * no card field to blank: it carries only the seat, so leaking the discard is a
 * compile error (V12/V13).
 *
 * Pure: `steps` and its members are never mutated.
 */
export function projectSteps(steps: readonly Step[], seat: Seat): Step[] {
	return steps.map((step): Step => {
		if (step.t !== 'dealt') return step;
		const packets: DealtPacket[] = step.packets.map((p) => ({
			seat: p.seat,
			count: p.count,
			cards: p.seat === seat && p.cards !== null ? (p.cards.slice() as CardId[]) : null
		}));
		return { t: 'dealt', packets };
	});
}

export { applyOptimistic, OPTIMISTIC_FIELDS } from './optimistic';
export type { OptimisticField } from './optimistic';
export { assertInvariants, assertLegalNonEmpty, InvariantError } from './invariants';
