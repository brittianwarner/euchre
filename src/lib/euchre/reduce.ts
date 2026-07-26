/**
 * `reduce.ts` — the whole game, as three pure functions over `GameState`.
 *
 * | Export        | Who calls it                                                  |
 * |---------------|---------------------------------------------------------------|
 * | `createGame`  | the table actor's `createState`, and every test                |
 * | `startHand`   | the table actor's `startHand` queue handler (engine-only)      |
 * | `apply`       | **the** seat-move entry point — every human, AI and auto move  |
 * | `advance`     | the table actor's `tick`/`onTempoGate` handler (engine-only)   |
 *
 * Nothing else in the system is permitted to write a `GameState`. One rulebook,
 * one writer.
 *
 * Three properties this module holds itself to, all of them load-bearing:
 *
 * 1. **Purity (V21).** No argument is ever mutated. Every returned `GameState`,
 *    `HandState`, array and tuple is freshly allocated. Deep-freezing an input
 *    and replaying the whole match through it must not throw.
 * 2. **Determinism (V20).** No `Date.now()`, no `Math.random()`, no ambient
 *    input of any kind — not even for the turn nonce, which is a hash chain over
 *    the previous nonce. Time and randomness enter only as arguments: a seed for
 *    the deal, an explicit timestamp if one is ever needed. Replaying
 *    `{ seed, moves[] }` reproduces every intermediate state bit-for-bit.
 * 3. **Totality.** Every phase and every transition in the § 7 table is
 *    implemented here: all-pass, stick-the-dealer, the dealer's discard, going
 *    alone, trick resolution, hand scoring, throw-in and game over. `advance()`
 *    is total: it is a no-op in any phase with a seat to act, and it always makes
 *    progress in any phase without one, so the machine can never stall.
 *
 * The acting seat is **never** read from the move payload. `Move` has no seat
 * field by construction; the caller states who is acting and `apply` validates
 * that claim against `hand.turnSeat`. A mismatch is a `RuleError`, never a
 * silent correction.
 *
 * Normative source: `docs/02-GAME-RULES-ENGINE.md` §§ 3–7.
 */

import { sameColor, suitOf } from './cards';
import {
	cutForHand,
	dealHand,
	dealtPackets,
	fnv1a,
	pickFirstDealer,
	mulberry32,
	shuffleForHand
} from './deal';
import {
	SUIT_NAME,
	findLegalMove,
	getLegalPlays,
	isStuckDealer,
	legalMoves,
	orderLabel
} from './legal';
import {
	cutterOf,
	eldestOf,
	firstActiveFrom,
	nextActiveSeat,
	nextSeat,
	partnerOf,
	teamOf
} from './seats';
import { applyDelta, scoreHand, winnerTeam } from './score';
import { isTrickComplete, ledSuitOf, trickWinner } from './trick';
import {
	DEFAULT_ENGINE_CONFIG,
	type Bid,
	type BidRecord,
	type CardId,
	type EngineConfig,
	type GameState,
	type HandState,
	type LegalMove,
	type LegalMoveId,
	type Move,
	type Play,
	type RuleCode,
	type Seat,
	type Step,
	type Suit,
	type Trick
} from './types';

/* -------------------------------------------------------------------------- */
/* The reducer contract                                                        */
/* -------------------------------------------------------------------------- */

/**
 * What every transition returns: the next state, and the ordered choreography
 * that describes how we got there.
 *
 * `steps` is decoration, never truth — the client animates from it but renders
 * from the accompanying `PublicGameView`. It is ordered: the first step is the
 * first thing that happened. It is also **unredacted**: run it through
 * `projectSteps(steps, seat)` before it leaves the server (V12).
 */
export interface ApplyResult {
	readonly state: GameState;
	readonly steps: readonly Step[];
}

/** Optional caller-supplied checks that `apply` performs before anything else. */
export interface ApplyOptions {
	/**
	 * The turn nonce the caller believes is current. When supplied and stale, the
	 * move is rejected with `stale_turn` — this is how two tabs submitting the
	 * same turn are resolved. Omit it and no nonce check is performed.
	 */
	readonly turnId?: string;
}

/**
 * The only error `apply` throws for a rules violation.
 *
 * `code` is a typed {@link RuleCode} from a closed union — never a bare string —
 * because the client keys distinct copy off it and the run loop turns it into a
 * typed rejection. `legal` carries the mover's actual legal set so the caller can
 * attach it as `metadata.legal` and let the client snap back without a round
 * trip.
 *
 * A `RuleError` is a *rules event*: expected, recoverable, shown to a player. A
 * plain `Error` from this module is a *programmer error* (a broken invariant) and
 * must never be shown to anyone.
 */
export class RuleError extends Error {
	readonly code: RuleCode;
	readonly legal: readonly LegalMove[];

	constructor(code: RuleCode, legal: readonly LegalMove[] = []) {
		super(code);
		this.name = 'RuleError';
		this.code = code;
		this.legal = legal;
		// Keeps `instanceof` working if this is ever downlevelled below ES2015.
		Object.setPrototypeOf(this, RuleError.prototype);
	}
}

/** Narrowing type guard for {@link RuleError}, safe across module realms. */
export function isRuleError(e: unknown): e is RuleError {
	return e instanceof RuleError || (e instanceof Error && e.name === 'RuleError');
}

/** The `trumpSet` member of the `Step` union, named so it can be built directly. */
type TrumpSetStep = Extract<Step, { t: 'trumpSet' }>;

/** A broken engine invariant. Never a rules event; never surfaced to a player. */
function invariant(condition: unknown, message: string): asserts condition {
	if (!condition) throw new Error(`euchre engine invariant violated: ${message}`);
}

/* -------------------------------------------------------------------------- */
/* Move identity                                                               */
/* -------------------------------------------------------------------------- */

/**
 * The stable wire id of a move — the same string the client hit-tests, the AI's
 * `z.enum` admits, the heuristic ranks and the replay log stores.
 *
 * Total over `Move`, and the exact inverse of the `move` field on the
 * `LegalMove`s that `legalMoves()` produces, so "is this legal" and "what is
 * this" can never disagree.
 */
export function moveId(move: Move): LegalMoveId {
	switch (move.t) {
		case 'pass':
			return 'pass';
		case 'orderUp':
			return move.alone ? 'orderUp+alone' : 'orderUp';
		case 'call':
			return move.alone ? `call:${move.suit}+alone` : `call:${move.suit}`;
		case 'cut':
			return move.cut ? 'cut:yes' : 'cut:no';
		case 'discard':
			return `discard:${move.card}`;
		case 'play':
			return `play:${move.card}`;
	}
}

/** Whether a move belongs to the bidding vocabulary (V2). */
function isBidMove(move: Move): move is Bid {
	return move.t === 'pass' || move.t === 'orderUp' || move.t === 'call';
}

/* -------------------------------------------------------------------------- */
/* The table script (§ 10)                                                     */
/* -------------------------------------------------------------------------- */

/**
 * The verbatim line spoken at the table for one bid. Engine-generated, never LLM
 * output, never filtered — the personas vary their banter *around* these, never
 * instead of them.
 *
 * Round 1: the dealer turns it down, everyone else passes. Round 2: naming the
 * turned-down suit's colour mate is **"Next."**, the other colour is
 * **"Crossing the creek."**, and the stuck dealer says so out loud.
 */
export function sayForBid(state: GameState, seat: Seat, move: Bid): string {
	const h = state.hand;
	const alone = move.t !== 'pass' && move.alone ? ' Alone.' : '';

	if (move.t === 'pass') {
		// Only the dealer can end round 1, and what they end it with is a turn-down.
		const turningDown = h.phase === 'bid_round_1' && seat === h.dealerSeat && h.passes === 3;
		return turningDown ? 'Turn it down.' : 'Pass.';
	}

	if (move.t === 'orderUp') return `${orderLabel(seat, h.dealerSeat)}.${alone}`;

	if (isStuckDealer(state, seat)) return `${SUIT_NAME[move.suit]}, I guess.${alone}`;
	if (h.turnedDownSuit !== null) {
		return sameColor(move.suit, h.turnedDownSuit) ? `Next.${alone}` : `Crossing the creek.${alone}`;
	}
	return `${SUIT_NAME[move.suit]}.${alone}`;
}

/* -------------------------------------------------------------------------- */
/* Immutable update helpers                                                    */
/* -------------------------------------------------------------------------- */

/**
 * A fresh turn nonce, minted by hashing the previous one together with the
 * match's private token.
 *
 * Deterministic on purpose: a nonce drawn from `Math.random()` would break
 * bit-for-bit replay (V20). It is not a security boundary — the actor↔actor
 * boundary is the constant-time `internalToken` compare — it exists only so a
 * stale submit from a second tab can be recognised and rejected as `stale_turn`.
 */
function mintTurnId(state: GameState): string {
	const a = fnv1a(`${state.internalToken}#${state.turnId}#a`).toString(36);
	const b = fnv1a(`${state.internalToken}#${state.turnId}#b`).toString(36);
	return `${a}${b}`;
}

/** A new `GameState` with `changes` applied and a fresh turn nonce. Never mutates. */
function next(state: GameState, changes: Partial<Omit<GameState, 'turnId'>>): GameState {
	return { ...state, ...changes, turnId: mintTurnId(state) };
}

/** A new `HandState` with `changes` applied. Never mutates. */
function withHandChanges(hand: HandState, changes: Partial<HandState>): HandState {
	return { ...hand, ...changes };
}

/** A fresh hands record with one seat replaced. Never mutates the original. */
function replaceHand(
	hands: Readonly<Record<Seat, readonly CardId[]>>,
	seat: Seat,
	cards: readonly CardId[]
): Record<Seat, readonly CardId[]> {
	const out: Record<Seat, readonly CardId[]> = {
		0: hands[0],
		1: hands[1],
		2: hands[2],
		3: hands[3]
	};
	out[seat] = cards;
	return out;
}

/** `cards` without the first occurrence of `card`. Returns a fresh array. */
function removeCard(cards: readonly CardId[], card: CardId): CardId[] {
	const i = cards.indexOf(card);
	invariant(i >= 0, `removeCard: ${card} is not held`);
	return [...cards.slice(0, i), ...cards.slice(i + 1)];
}

/** An empty trick at `index`. */
function emptyTrick(index: number): Trick {
	return { index, ledSuit: null, plays: [], winnerSeat: null };
}

/** The four undealt cards of a permutation: `deckOrder[20..23]`. */
function kittyOf(deckOrder: readonly CardId[]): [CardId, CardId, CardId, CardId] {
	invariant(deckOrder.length === 24, 'kittyOf: deckOrder must hold 24 cards');
	return [deckOrder[20]!, deckOrder[21]!, deckOrder[22]!, deckOrder[23]!];
}

/* -------------------------------------------------------------------------- */
/* Construction                                                                */
/* -------------------------------------------------------------------------- */

/** Everything the sole constructor needs. Only `gameId` and `seed` are required. */
export interface GameInit {
	readonly gameId: string;
	/** The match seed. A replay is `{ seed, moves[] }`; this never reaches a client. */
	readonly seed: string;
	/** Rule overrides on top of {@link DEFAULT_ENGINE_CONFIG}. */
	readonly cfg?: Partial<EngineConfig>;
	/**
	 * The actor↔actor authorization secret. Derived deterministically from the
	 * seed when omitted, so a replayed match reproduces bit-for-bit; production
	 * callers (`createState`) should pass a cryptographically random value, which
	 * is the one field a replay comparison must exclude.
	 */
	readonly internalToken?: string;
	/** Overrides the seeded draw. For fixtures and the scripted tutorial deal. */
	readonly firstDealer?: Seat;
}

/** A deterministic pseudo-token derived from the seed. See {@link GameInit}. */
function deriveToken(seed: string, gameId: string): string {
	let out = '';
	for (let i = 0; i < 4; i++) out += fnv1a(`${seed}#${gameId}#token#${i}`).toString(36);
	return out;
}

/**
 * The sole constructor. Produces a `lobby` state: no cards are in anyone's hand
 * yet and no seat is to act. Call {@link startHand} — or just {@link advance},
 * which does it for you — to open the first hand.
 *
 * `firstDealer` is drawn once here from a dedicated derived stream, so it cannot
 * shift the per-hand shuffle stream, and is then immutable for the life of the
 * match; after every scored hand and every throw-in the deal moves one seat left.
 */
export function createGame(init: GameInit): GameState {
	const cfg: EngineConfig = { ...DEFAULT_ENGINE_CONFIG, ...init.cfg };
	const firstDealer =
		init.firstDealer ?? pickFirstDealer(mulberry32(fnv1a(`${init.seed}#firstDealer`)));
	const deckOrder = shuffleForHand(init.seed, 0);

	const hand: HandState = {
		handNo: 0,
		phase: 'lobby',
		dealerSeat: firstDealer,
		turnSeat: null,
		deckOrder,
		cut: null,
		hands: { 0: [], 1: [], 2: [], 3: [] },
		// Provisional: the cut rotates the permutation, so the deal re-derives both.
		kitty: kittyOf(deckOrder),
		upCard: null,
		upCardTurnedDown: false,
		turnedDownSuit: null,
		trump: null,
		makerSeat: null,
		aloneSeat: null,
		sittingSeat: null,
		dealerDiscard: null,
		passes: 0,
		bids: [],
		trick: emptyTrick(0),
		trickLog: [],
		tricksWon: [0, 0],
		result: null,
		delta: null
	};

	return {
		schema: 1,
		v: 0,
		gameId: init.gameId,
		internalToken: init.internalToken ?? deriveToken(init.seed, init.gameId),
		seed: init.seed,
		cfg,
		firstDealer,
		status: 'active',
		score: [0, 0],
		handNo: 0,
		misdealStreak: 0,
		winnerTeam: null,
		hand,
		turnId: fnv1a(`${init.seed}#turn0`).toString(36),
		turnDeadlineAt: null,
		timerId: null,
		watchdogId: null,
		revealId: null,
		pending: null,
		recentMoveIds: [],
		appliedSeq: 0
	};
}

/* -------------------------------------------------------------------------- */
/* startHand — lobby | hand_score(next) -> cutting                             */
/* -------------------------------------------------------------------------- */

/**
 * Open a hand: shuffle from `(seed, handNo)`, persist `deckOrder`, reset the
 * whole `HandState`, and hand the turn to the cutter.
 *
 * The shuffle is persisted **before** the `cutting` phase opens, so a crash
 * during the cut cannot reshuffle a hand a player has already partly seen. The
 * cut itself is applied later, deterministically, by `cutForHand`.
 *
 * Engine-only: there is no seat to attribute it to. Carries `dealerSeat` and
 * `handNo` in from `state` — rotating the deal is the caller's job (and
 * {@link advance} does it), so this is idempotent.
 */
export function startHand(state: GameState): ApplyResult {
	const dealerSeat = state.hand.dealerSeat;
	const handNo = state.handNo;
	const deckOrder = shuffleForHand(state.seed, handNo);

	const hand: HandState = {
		handNo,
		phase: 'cutting',
		dealerSeat,
		turnSeat: cutterOf(dealerSeat),
		deckOrder,
		cut: null,
		hands: { 0: [], 1: [], 2: [], 3: [] },
		kitty: kittyOf(deckOrder),
		upCard: null,
		upCardTurnedDown: false,
		turnedDownSuit: null,
		trump: null,
		makerSeat: null,
		aloneSeat: null,
		sittingSeat: null,
		dealerDiscard: null,
		passes: 0,
		bids: [],
		trick: emptyTrick(0),
		trickLog: [],
		tricksWon: [0, 0],
		result: null,
		delta: null
	};

	return { state: next(state, { hand }), steps: [] };
}

/**
 * The state rotated onto the next hand: counter up, deal one seat left. The
 * `HandState` itself is still the old one — {@link startHand} replaces it.
 */
function rotateToNextHand(state: GameState, misdealStreak: number): GameState {
	const handNo = state.handNo + 1;
	return next(state, {
		handNo,
		misdealStreak,
		hand: withHandChanges(state.hand, { handNo, dealerSeat: nextSeat(state.hand.dealerSeat) })
	});
}

/* -------------------------------------------------------------------------- */
/* deal — engine-only, atomic                                                  */
/* -------------------------------------------------------------------------- */

/**
 * Deal from the persisted (and, if the cutter bumped, already-rotated)
 * permutation and turn the up-card. Atomic: `deal` is a transition, not a
 * resting phase, so this always lands on `bid_round_1`.
 */
function runDeal(state: GameState): ApplyResult {
	const h = state.hand;
	const { plan, hands, kitty, upCard } = dealHand(h.deckOrder, h.dealerSeat, state.seed, h.handNo);

	const hand = withHandChanges(h, {
		phase: 'bid_round_1',
		turnSeat: eldestOf(h.dealerSeat),
		hands,
		kitty,
		upCard,
		passes: 0,
		bids: [],
		trick: emptyTrick(0)
	});

	return {
		state: next(state, { hand }),
		steps: [
			{ t: 'dealt', packets: dealtPackets(h.deckOrder, plan) },
			{ t: 'upCardTurned', card: upCard }
		]
	};
}

/* -------------------------------------------------------------------------- */
/* apply — THE seat move                                                       */
/* -------------------------------------------------------------------------- */

/**
 * Apply one seat's move. The single mutation entry point for everything a
 * player, an AI or the auto-play ladder can do.
 *
 * `seat` is the caller's claim about who is acting and is **validated**, not
 * trusted: it must equal `hand.turnSeat`, it must not be the loner's sitting
 * partner, and the move must appear in `legalMoves(state, seat)`. The move
 * payload carries no seat and none is ever inferred from it.
 *
 * Throws {@link RuleError} — and nothing else — for every rules violation, with
 * the most specific applicable {@link RuleCode}:
 *
 * | Code               | Raised when                                              |
 * |--------------------|----------------------------------------------------------|
 * | `stale_turn`       | `opts.turnId` was supplied and is not `state.turnId`      |
 * | `sitting_out`      | any action from the loner's sitting partner               |
 * | `wrong_phase`      | the move kind does not belong to `hand.phase`             |
 * | `not_your_turn`    | a seat that is not `hand.turnSeat` acted                  |
 * | `bidding_closed`   | a bid arriving after trump is set                         |
 * | `dealer_must_call` | `pass` from the stuck dealer — *never* `bidding_closed`   |
 * | `card_not_in_hand` | a `play`/`discard` naming a card the seat does not hold   |
 * | `must_follow_suit` | a `play` that reneges while a card of the led suit is held |
 * | `illegal_move`     | anything else absent from the legal set                   |
 *
 * Pure: `state` is never mutated and the returned state shares no mutable
 * structure with it.
 */
export function apply(state: GameState, seat: Seat, move: Move, opts?: ApplyOptions): ApplyResult {
	const h = state.hand;
	const reject = (code: RuleCode): never => {
		throw new RuleError(code, legalMoves(state, seat));
	};

	/* --- who may speak at all ------------------------------------------------ */

	if (state.status !== 'active') reject('wrong_phase');
	if (opts?.turnId !== undefined && opts.turnId !== state.turnId) reject('stale_turn');
	if (seat === h.sittingSeat) reject('sitting_out');
	if (h.turnSeat === null) reject('wrong_phase');
	if (h.turnSeat !== seat) reject('not_your_turn');

	/* --- does this move kind belong to this phase (V1, V2) ------------------- */

	switch (h.phase) {
		case 'cutting':
			if (move.t !== 'cut') reject('wrong_phase');
			break;
		case 'bid_round_1':
			// Round 1 admits only `pass | orderUp`: naming a suit is a round-2 move.
			if (move.t !== 'pass' && move.t !== 'orderUp') reject('wrong_phase');
			break;
		case 'bid_round_2':
			// The up-card is buried; there is nothing left to order up.
			if (move.t !== 'pass' && move.t !== 'call') reject('wrong_phase');
			break;
		case 'dealer_discard':
			if (isBidMove(move)) reject('bidding_closed');
			if (move.t !== 'discard') reject('wrong_phase');
			break;
		case 'trick_play':
			if (isBidMove(move)) reject('bidding_closed');
			if (move.t !== 'play') reject('wrong_phase');
			break;
		default:
			// Engine-only phases have a null `turnSeat` and were rejected above.
			reject('wrong_phase');
	}

	/* --- the specific codes, most specific first ----------------------------- */

	if (move.t === 'pass' && isStuckDealer(state, seat)) reject('dealer_must_call');
	if (move.t === 'call' && move.suit === h.turnedDownSuit) reject('illegal_move');
	if (move.t === 'discard' && !h.hands[seat].includes(move.card)) reject('card_not_in_hand');
	if (move.t === 'play') {
		if (!h.hands[seat].includes(move.card)) reject('card_not_in_hand');
		if (!getLegalPlays(h.hands[seat], h.trick.ledSuit, h.trump).includes(move.card)) {
			reject('must_follow_suit');
		}
	}

	/* --- and the backstop: it must be in the legal set, full stop ------------ */

	if (findLegalMove(state, seat, moveId(move)) === null) reject('illegal_move');

	/* --- dispatch ------------------------------------------------------------ */

	switch (move.t) {
		case 'cut':
			return applyCut(state, seat, move.cut);
		case 'pass':
			return h.phase === 'bid_round_1'
				? applyPassRound1(state, seat)
				: applyPassRound2(state, seat);
		case 'orderUp':
			return applyOrderUp(state, seat, move.alone);
		case 'call':
			return applyCall(state, seat, move.suit, move.alone);
		case 'discard':
			return applyDiscard(state, seat, move.card);
		case 'play':
			return applyPlay(state, seat, move.card);
	}
}

/* -------------------------------------------------------------------------- */
/* cutting                                                                     */
/* -------------------------------------------------------------------------- */

/**
 * "Bump." or "Run 'em." Either answer advances; declining is a recorded move, not
 * a pass. The cut is a deterministic rotation of the already-persisted
 * permutation at an index derived from `(seed, handNo)` — never a reshuffle — so
 * a crash here cannot change anyone's hand. The deal follows atomically.
 */
function applyCut(state: GameState, seat: Seat, cut: boolean): ApplyResult {
	const h = state.hand;
	const deckOrder = cut ? cutForHand(h.deckOrder, state.seed, h.handNo) : h.deckOrder;

	const afterCut = next(state, {
		hand: withHandChanges(h, {
			cut,
			deckOrder,
			kitty: kittyOf(deckOrder),
			phase: 'deal',
			turnSeat: null
		})
	});
	const dealt = runDeal(afterCut);

	return { state: dealt.state, steps: [{ t: 'cut', seat, cut }, ...dealt.steps] };
}

/* -------------------------------------------------------------------------- */
/* bidding                                                                     */
/* -------------------------------------------------------------------------- */

/** Appends one auction entry. `bids` is bounded at 8: four seats × two rounds. */
function recordBid(h: HandState, seat: Seat, move: Bid, say: string): readonly BidRecord[] {
	return [...h.bids, { seat, move, say }];
}

/** A `bid` step, which is what the talk log and the seat bubbles animate from. */
function bidStep(seat: Seat, move: Bid, say: string): Step {
	return { t: 'bid', seat, id: moveId(move), say };
}

/**
 * Round 1 pass. On the fourth the dealer turns it down: the up-card's suit is
 * forbidden for the rest of the auction, the round-2 pass counter resets, and
 * eldest speaks again. The up-card itself stays publicly identified (V14).
 */
function applyPassRound1(state: GameState, seat: Seat): ApplyResult {
	const h = state.hand;
	const move: Bid = { t: 'pass' };
	const say = sayForBid(state, seat, move);
	const bids = recordBid(h, seat, move, say);
	const passes = h.passes + 1;

	if (passes < 4) {
		return {
			state: next(state, {
				hand: withHandChanges(h, { passes, bids, turnSeat: nextSeat(seat) })
			}),
			steps: [bidStep(seat, move, say)]
		};
	}

	invariant(h.upCard !== null, 'round 1 ended with no up-card');
	const turnedDownSuit = suitOf(h.upCard);
	return {
		state: next(state, {
			hand: withHandChanges(h, {
				phase: 'bid_round_2',
				turnSeat: eldestOf(h.dealerSeat),
				upCardTurnedDown: true,
				turnedDownSuit,
				passes: 0,
				bids
			})
		}),
		steps: [bidStep(seat, move, say), { t: 'turnedDown', suit: turnedDownSuit }]
	};
}

/**
 * Round 2 pass. With `stickTheDealer` on, the dealer never reaches this on the
 * fourth pass — `apply` has already rejected it as `dealer_must_call`. With it
 * off, the fourth pass throws the hand in.
 */
function applyPassRound2(state: GameState, seat: Seat): ApplyResult {
	const h = state.hand;
	const move: Bid = { t: 'pass' };
	const say = sayForBid(state, seat, move);
	const bids = recordBid(h, seat, move, say);
	const passes = h.passes + 1;

	if (passes < 4) {
		return {
			state: next(state, {
				hand: withHandChanges(h, { passes, bids, turnSeat: nextSeat(seat) })
			}),
			steps: [bidStep(seat, move, say)]
		};
	}

	// Exhaustion — the only misdeal a digital deal can produce (§ 5.2). Nobody
	// scores, the streak grows, the deal moves left, and a fresh hand opens.
	invariant(!state.cfg.stickTheDealer, 'four round-2 passes with stick-the-dealer on');
	const thrownIn = next(state, { hand: withHandChanges(h, { passes, bids }) });
	const streak = Math.min(state.cfg.misdealLimit, state.misdealStreak + 1);
	const opened = startHand(rotateToNextHand(thrownIn, streak));

	return {
		state: opened.state,
		steps: [bidStep(seat, move, say), { t: 'throwIn' }, ...opened.steps]
	};
}

/**
 * The shared tail of every successful call: trump exists, the maker is known, and
 * a loner's partner is seated out in the *same* transition (V4). `alone` can
 * never be declared later.
 */
function setTrump(
	h: HandState,
	suit: Suit,
	makerSeat: Seat,
	alone: boolean
): { hand: HandState; step: TrumpSetStep } {
	const aloneSeat = alone ? makerSeat : null;
	const sittingSeat = alone ? partnerOf(makerSeat) : null;
	return {
		hand: withHandChanges(h, { trump: suit, makerSeat, aloneSeat, sittingSeat }),
		step: { t: 'trumpSet', suit, makerSeat, aloneSeat, sittingSeat, viaOrderUp: false }
	};
}

/** The trick-1 lead: eldest, or the next active seat if eldest is sitting out. */
function openingLead(h: HandState): Seat {
	return firstActiveFrom(eldestOf(h.dealerSeat), h.sittingSeat);
}

/**
 * Round 1 acceptance — "Order it up." / "I assist." / "I take it."
 *
 * Makes the up-card's suit trump, puts the up-card in the dealer's hand, and
 * routes to `dealer_discard` **always**, regardless of who called.
 *
 * One exception, and it is forced: if the dealer's partner ordered up *alone*,
 * the dealer is the sitting partner, and asking a seated-out player to discard
 * would deadlock the machine (their legal set is empty by rule, V8/V19). In that
 * case the pickup and the discard collapse into the canonical no-op — the dealer
 * takes the up-card and lays it straight back down — which keeps `dealerDiscard`
 * non-null exactly once per round-1 call (V5) and leaves the dead hand at five.
 */
function applyOrderUp(state: GameState, seat: Seat, alone: boolean): ApplyResult {
	const h = state.hand;
	invariant(h.upCard !== null, 'orderUp with no up-card');
	const upCard = h.upCard;

	const move: Bid = { t: 'orderUp', alone };
	const say = sayForBid(state, seat, move);
	const bids = recordBid(h, seat, move, say);

	const trumped = setTrump(h, suitOf(upCard), seat, alone);
	const trumpSet: TrumpSetStep = { ...trumped.step, viaOrderUp: true };
	const steps: Step[] = [bidStep(seat, move, say), trumpSet];

	const dealerIsSittingOut = trumped.hand.sittingSeat === h.dealerSeat;
	if (dealerIsSittingOut) {
		const hand = withHandChanges(trumped.hand, {
			phase: 'trick_play',
			turnSeat: openingLead(trumped.hand),
			bids,
			dealerDiscard: upCard,
			trick: emptyTrick(0)
		});
		return {
			state: next(state, { hand }),
			steps: [...steps, { t: 'dealerDiscarded', seat: h.dealerSeat }]
		};
	}

	const hand = withHandChanges(trumped.hand, {
		phase: 'dealer_discard',
		turnSeat: h.dealerSeat,
		bids,
		hands: replaceHand(h.hands, h.dealerSeat, [...h.hands[h.dealerSeat], upCard])
	});
	return { state: next(state, { hand }), steps };
}

/**
 * Round 2 call. The turned-down suit is unnameable (already rejected by `apply`),
 * there is no pickup and no discard, and play begins immediately.
 */
function applyCall(state: GameState, seat: Seat, suit: Suit, alone: boolean): ApplyResult {
	const h = state.hand;
	const move: Bid = { t: 'call', suit, alone };
	const say = sayForBid(state, seat, move);
	const bids = recordBid(h, seat, move, say);

	const trumped = setTrump(h, suit, seat, alone);
	const hand = withHandChanges(trumped.hand, {
		phase: 'trick_play',
		turnSeat: openingLead(trumped.hand),
		bids,
		trick: emptyTrick(0)
	});

	return { state: next(state, { hand }), steps: [bidStep(seat, move, say), trumped.step] };
}

/* -------------------------------------------------------------------------- */
/* dealer_discard                                                              */
/* -------------------------------------------------------------------------- */

/**
 * The dealer lays one of their six face down. The up-card itself is a legal
 * choice — declining to use it while its suit stays trump.
 *
 * The discarded card is stored for the journal and is **never revealed to any
 * client, ever** (V12). The emitted step carries only the seat: there is no card
 * field on it, so leaking the discard is a compile error rather than a review
 * catch.
 */
function applyDiscard(state: GameState, seat: Seat, card: CardId): ApplyResult {
	const h = state.hand;
	const hands = replaceHand(h.hands, seat, removeCard(h.hands[seat], card));
	invariant(hands[seat].length === 5, 'dealer hand is not five cards after the discard');

	const hand = withHandChanges(h, {
		phase: 'trick_play',
		turnSeat: openingLead(h),
		hands,
		dealerDiscard: card,
		trick: emptyTrick(0)
	});

	return { state: next(state, { hand }), steps: [{ t: 'dealerDiscarded', seat }] };
}

/* -------------------------------------------------------------------------- */
/* trick_play                                                                  */
/* -------------------------------------------------------------------------- */

/**
 * Lay a card. The led suit is the **effective** suit of the first play, so a lead
 * of the left bower leads trump.
 *
 * When the last active seat has played — three under a loner, four otherwise —
 * the trick is resolved here and the machine parks in `trick_resolve`, a real
 * server-held phase: the next card genuinely does not exist yet, and the pause
 * is where the read happens. {@link advance} releases it.
 */
function applyPlay(state: GameState, seat: Seat, card: CardId): ApplyResult {
	const h = state.hand;
	invariant(h.trump !== null, 'a card was played before trump was set');

	const hands = replaceHand(h.hands, seat, removeCard(h.hands[seat], card));
	const plays: readonly Play[] = [...h.trick.plays, { seat, card }];
	const ledSuit = ledSuitOf(plays, h.trump);
	invariant(ledSuit !== null, 'a trick with plays has no led suit');
	const played: Step = { t: 'cardPlayed', seat, card };

	if (!isTrickComplete(plays, h.sittingSeat)) {
		const hand = withHandChanges(h, {
			hands,
			trick: { ...h.trick, ledSuit, plays },
			turnSeat: nextActiveSeat(seat, h.sittingSeat)
		});
		return { state: next(state, { hand }), steps: [played] };
	}

	const winnerSeat = trickWinner(plays, ledSuit, h.trump);
	invariant(winnerSeat !== h.sittingSeat, 'the sitting seat took a trick (V9)');

	const tricksWon: [number, number] = [h.tricksWon[0], h.tricksWon[1]];
	tricksWon[teamOf(winnerSeat)] += 1;

	const trick: Trick = { index: h.trick.index, ledSuit, plays, winnerSeat };
	const hand = withHandChanges(h, {
		phase: 'trick_resolve',
		turnSeat: null,
		hands,
		trick,
		trickLog: [...h.trickLog, trick],
		tricksWon
	});

	// The defenders reaching three tricks makes the euchre arithmetic certain;
	// the client holds this trick 1100 ms instead of 700–1000 so it can be read.
	invariant(h.makerSeat !== null, 'a trick resolved with no maker');
	const defenders = (1 - teamOf(h.makerSeat)) as 0 | 1;
	const sealsEuchre = tricksWon[defenders] === 3;

	return {
		state: next(state, { hand }),
		steps: [played, { t: 'trickWon', index: trick.index, winnerSeat, plays, sealsEuchre }]
	};
}

/* -------------------------------------------------------------------------- */
/* advance — the engine-only transitions                                       */
/* -------------------------------------------------------------------------- */

/**
 * Advance out of a phase that has no acting seat.
 *
 * Total, and a no-op in any phase where a seat is to act — so a run loop may call
 * it unconditionally, and the fuzz harness does exactly that: `turnSeat === null
 * ? advance(state) : apply(state, turnSeat, …)`. Because it always makes progress
 * when `turnSeat` is `null`, the machine cannot stall.
 *
 * - `lobby` → open the first hand.
 * - `deal` → deal and turn the up-card (defensive; the deal is normally atomic
 *   with the cut).
 * - `trick_resolve` → the winner leads the next trick, or the hand is scored.
 * - `hand_score` → the match ends, or the deal moves left and a hand opens.
 * - `game_over` and any phase with a `turnSeat` → identity.
 */
export function advance(state: GameState): ApplyResult {
	if (state.status !== 'active') return { state, steps: [] };

	switch (state.hand.phase) {
		case 'lobby':
			return startHand(state);
		case 'deal':
			return runDeal(state);
		case 'trick_resolve':
			return advanceTrickResolve(state);
		case 'hand_score':
			return advanceHandScore(state);
		default:
			// cutting · bid_round_1 · dealer_discard · bid_round_2 · trick_play ·
			// game_over — a seat is to act, or the match is over. Nothing to do.
			return { state, steps: [] };
	}
}

/**
 * Release the trick-resolve hold: the winner leads the next trick — asserted
 * never to be the sitting seat (V9) — or, after the fifth, the hand is scored.
 */
function advanceTrickResolve(state: GameState): ApplyResult {
	const h = state.hand;
	const winnerSeat = h.trick.winnerSeat;
	invariant(winnerSeat !== null, 'trick_resolve with an unresolved trick');

	if (h.trickLog.length < 5) {
		const hand = withHandChanges(h, {
			phase: 'trick_play',
			turnSeat: winnerSeat,
			trick: emptyTrick(h.trick.index + 1)
		});
		return { state: next(state, { hand }), steps: [] };
	}

	return enterHandScore(state);
}

/**
 * Score the hand: apply the § 5.1 table, clamp at `cfg.gameTo`, and park in
 * `hand_score` so the client can hold the recap. `delta` is post-clamp, so
 * `score[i] + delta[i]` is always the new score — a march at 9 wins at 10, never
 * 11.
 *
 * The completed fifth trick is deliberately left in `hand.trick` for the recap;
 * `trickLog` holds all five.
 */
function enterHandScore(state: GameState): ApplyResult {
	const h = state.hand;
	invariant(h.makerSeat !== null, 'a hand was scored with no maker');
	invariant(h.trickLog.length === 5, 'a hand was scored before five tricks');

	const { result, delta } = scoreHand(
		h.makerSeat,
		h.aloneSeat,
		h.tricksWon,
		state.score,
		state.cfg.gameTo
	);
	const score = applyDelta(state.score, delta);

	const hand = withHandChanges(h, { phase: 'hand_score', turnSeat: null, result, delta });
	return {
		state: next(state, { hand, score, winnerTeam: winnerTeam(score, state.cfg.gameTo) }),
		steps: [
			{
				t: 'handScored',
				result,
				delta,
				score,
				makerSeat: h.makerSeat,
				aloneSeat: h.aloneSeat,
				tricksWon: h.tricksWon
			}
		]
	};
}

/**
 * Release the recap hold: a team at `gameTo` ends the match, otherwise the streak
 * clears, the deal moves one seat left, and the next hand opens on the cut.
 */
function advanceHandScore(state: GameState): ApplyResult {
	if (state.winnerTeam !== null) {
		const hand = withHandChanges(state.hand, { phase: 'game_over', turnSeat: null });
		return {
			state: next(state, { hand, status: 'complete' }),
			steps: [{ t: 'gameWon', team: state.winnerTeam, score: state.score }]
		};
	}
	// Any scored hand clears the misdeal streak.
	return startHand(rotateToNextHand(state, 0));
}
