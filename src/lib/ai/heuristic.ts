/**
 * `$lib/ai/heuristic` — the deterministic euchre judgement engine.
 *
 * **Zero runtime dependencies beyond `$lib/euchre`. Pure and total.** No
 * `Math.random()`, no `Date.now()`, no clock, no network, no I/O, no mutation of
 * its arguments. The same inputs always produce byte-identical output, which is
 * what makes an AI seat reproducible in replay and what makes this safe to run
 * inside a table's serialized run loop.
 *
 * This module is **judgement, not rules**. It never decides what is *legal* — it
 * is handed the legal set that `legalMoves()` already computed and it only ever
 * orders that set. Binding rule 1 (one rulebook) is preserved structurally: the
 * only euchre facts imported here are the engine's own primitives
 * (`effectiveSuit`, `trumpRank`, `plainRank`, `cardValue`, `seatRole`, …) and no
 * rule is reimplemented.
 *
 * Four consumers share this one implementation:
 *
 * 1. the AI's *prior* — the bracketed scores shown to the model in `LEGAL:`;
 * 2. the AI's *fallback* — rung 2 of the decision ladder, taken whenever the
 *    model errors, times out, or names something outside the candidate set;
 * 3. the coach hints in the browser;
 * 4. the table's abandon / watchdog auto-play.
 *
 * Because of (2) and (4) this module carries two hard obligations:
 *
 * - **It must never throw.** Every public entry point is wrapped; a defect
 *   degrades to engine order rather than stalling a hand.
 * - **It must never name a move that was not in `legal`.** Every score is
 *   attached to a member of the caller's own array; ids are never synthesised.
 *
 * Ties break on `id` ascending, so a ranking is a total order and two runs of the
 * same hand rank identically.
 *
 * Normative source: `docs/03-AI-AGENTS.md` §8. Numbers here are the spec's
 * starting values and are **not frozen** — invariant 22 ("no number frozen before
 * it is measured") applies to every threshold in this file.
 */

import {
	DEFAULT_ENGINE_CONFIG,
	SUITS,
	cardValue,
	currentlyWinning,
	effectiveSuit,
	partnerOf,
	plainRank,
	playsPerTrick,
	rankOf,
	sameColor,
	seatRole,
	suitOf,
	teamOf,
	trumpRank
} from '#lib/euchre/index.ts';
import type {
	CardId,
	LegalMove,
	LegalMoveId,
	PublicGameView,
	RankedMove,
	Rank,
	Seat,
	SeatRole,
	Suit,
	Team
} from '#lib/protocol/index.ts';

/* ========================================================================== */
/* Tunables                                                                    */
/* ========================================================================== */

/**
 * Value of a trump card, keyed by `trumpRank()`.
 * Right bower `8` · left `7` · A `6` · K `5` · Q `4` · T `3` · 9 `2`.
 */
const TRUMP_VALUE: Readonly<Record<number, number>> = Object.freeze({
	8: 1.0,
	7: 0.9,
	6: 0.8,
	5: 0.5,
	4: 0.35,
	3: 0.15,
	2: 0.1
});

/** Value of an off-suit card by rank. Off-suit jacks and below are worth nothing. */
const OFF_VALUE: Readonly<Record<Rank, number>> = Object.freeze({
	A: 0.7,
	K: 0.3,
	Q: 0.1,
	J: 0.0,
	T: 0.0,
	'9': 0.0
});

/**
 * Round-1 "order it up" thresholds on the {@link handStrength} scale, keyed by
 * {@link SeatRole} (`0` dealer · `1` eldest · `2` dealer's partner · `3` third).
 *
 * Seat role 2 is the most aggressive because the up-card lands in their
 * *partner's* hand; roles 1 and 3 are giving it to an opponent.
 */
const ORDER_UP_THRESHOLD: Readonly<Record<SeatRole, number>> = Object.freeze({
	0: 2.3,
	1: 2.6,
	2: 2.2,
	3: 2.55
});

/** Round-2 threshold for calling *next* (the same colour as the turned-down suit). */
const NEXT_THRESHOLD: Readonly<Record<SeatRole, number>> = Object.freeze({
	0: 2.4,
	1: 2.35,
	2: 2.7,
	3: 2.4
});

/** Round-2 threshold for "crossing the creek" (the opposite colour). */
const CROSS_THRESHOLD: Readonly<Record<SeatRole, number>> = Object.freeze({
	0: 2.5,
	1: 2.7,
	2: 2.7,
	3: 2.75
});

/** Threshold for declaring a loner, in either round. */
const ALONE_THRESHOLD: Readonly<Record<SeatRole, number>> = Object.freeze({
	0: 4.0,
	1: 4.3,
	2: 4.2,
	3: 4.3
});

/** How far a full-throttle `aggression` dial may move a bidding threshold. */
const AGGRESSION_SWING = 0.5;

/** How far a full-throttle `risk` dial may move the loner threshold. */
const RISK_SWING = 0.6;

/** Separates two otherwise-equal scores so a strict comparison is decisive. */
const EPS = 1e-6;

/* ========================================================================== */
/* Public options                                                              */
/* ========================================================================== */

/**
 * Persona flavouring. Every field is optional and every default is neutral, so
 * `rankMoves(view, hand, legal)` is the plain, persona-free ranking.
 *
 * The dials move *bidding thresholds only*. They never touch legality, they never
 * change which cards exist, and they cannot make the ranking non-deterministic.
 */
export interface HeuristicOptions {
	/** `0`–`1`, default `0.5`. Above `0.5` lowers the bidding thresholds. */
	readonly aggression?: number;
	/** `0`–`1`, default `0.5`. Above `0.5` lowers the loner threshold. */
	readonly risk?: number;
	/** Points to win. Defaults to {@link DEFAULT_ENGINE_CONFIG}`.gameTo` (10). */
	readonly gameTo?: number;
}

/** The context a strength evaluation needs beyond the cards themselves. */
export interface StrengthCtx {
	readonly seat: Seat;
	readonly dealerSeat: Seat;
	/** `round1` adds the up-card term; `round2` adds next/cross; `none` adds neither. */
	readonly phase: 'round1' | 'round2' | 'none';
	readonly upCard: CardId | null;
	readonly turnedDownSuit: Suit | null;
	readonly myScore: number;
	readonly oppScore: number;
	readonly gameTo: number;
}

/* ========================================================================== */
/* Small helpers                                                               */
/* ========================================================================== */

const clamp01 = (n: number): number => (n < 0 ? 0 : n > 1 ? 1 : n);

const isFiniteNumber = (n: number): boolean => Number.isFinite(n);

/** `true` when this card counts as trump (bower-aware). */
const isTrumpCard = (card: CardId, trump: Suit): boolean => effectiveSuit(card, trump) === trump;

/** The seven cards that are trump under `trump`, highest first. */
function trumpCards(trump: Suit): CardId[] {
	const mate: Suit = trump === 'S' ? 'C' : trump === 'C' ? 'S' : trump === 'H' ? 'D' : 'H';
	return [
		`J${trump}` as CardId,
		`J${mate}` as CardId,
		`A${trump}` as CardId,
		`K${trump}` as CardId,
		`Q${trump}` as CardId,
		`T${trump}` as CardId,
		`9${trump}` as CardId
	];
}

/** Power inside the current trick: `0`–`1`, trump-aware, comparable across suits. */
function power(card: CardId, trump: Suit): number {
	return isTrumpCard(card, trump) ? trumpRank(card, trump) / 8 : plainRank(card) / 6;
}

/** Every card visible on the table this hand: completed tricks plus the live one. */
function playedCards(view: PublicGameView): CardId[] {
	const out: CardId[] = [];
	const liveIndex = view.trick.index;
	for (const t of view.trickLog) {
		// During `trick_resolve` the live trick and the last journal entry are the
		// SAME trick. Dedupe by index or every card of it is counted twice.
		if (t.index === liveIndex) continue;
		for (const p of t.plays) out.push(p.card);
	}
	for (const p of view.trick.plays) out.push(p.card);
	return out;
}

/* ========================================================================== */
/* Hand strength                                                               */
/* ========================================================================== */

/**
 * The structural value of a holding under a given trump, before any positional,
 * auction or scoreboard adjustment.
 *
 * Counts bowers and aces by their real trick-taking value, rewards trump length,
 * punishes a one-trump hand, halves singleton off-suit honours (they get stripped
 * before they cash), and pays for voids **only when there is trump to ruff with**.
 */
export function rawHandValue(hand: readonly CardId[], trump: Suit): number {
	let s = 0;
	let trumpCount = 0;
	const offBySuit = new Map<Suit, CardId[]>();

	for (const c of hand) {
		const es = effectiveSuit(c, trump);
		if (es === trump) {
			trumpCount += 1;
			s += TRUMP_VALUE[trumpRank(c, trump)] ?? 0;
			continue;
		}
		const bucket = offBySuit.get(es);
		if (bucket === undefined) offBySuit.set(es, [c]);
		else bucket.push(c);
	}

	for (const cards of offBySuit.values()) {
		for (const c of cards) {
			const v = OFF_VALUE[rankOf(c)];
			s += cards.length === 1 ? v * 0.5 : v;
		}
	}

	if (trumpCount >= 4) s += 0.45;
	else if (trumpCount === 3) s += 0.15;
	else if (trumpCount <= 1) s -= 0.6;

	// Three off-suits exist; every one we do not hold is a void worth ruffing.
	const voids = 3 - offBySuit.size;
	const voidRate = trumpCount >= 3 ? 0.3 : trumpCount === 2 ? 0.15 : 0;
	s += Math.min(voids, 2) * voidRate;

	return s;
}

/**
 * Scoreboard pressure: the same cards are worth bidding on at 4–9 and worth
 * passing at 9–4.
 */
export function scoreAdjust(ctx: StrengthCtx): number {
	let a = 0;
	if (ctx.oppScore - ctx.myScore >= 4) a += 0.25; // behind: take risks
	if (ctx.myScore >= 8 && ctx.oppScore <= 6) a -= 0.25; // ahead: avoid the euchre
	if (ctx.oppScore === ctx.gameTo - 1) a += 0.35; // they're in the barn: donate
	return a;
}

/**
 * The full bidding evaluation of a holding: structure, plus where the up-card
 * lands, plus the round-2 seat conventions, plus scoreboard pressure.
 */
export function handStrength(hand: readonly CardId[], trump: Suit, ctx: StrengthCtx): number {
	let s = rawHandValue(hand, trump);

	if (ctx.phase === 'round1' && ctx.upCard !== null) {
		if (ctx.seat === ctx.dealerSeat) {
			// We are the one picking it up: value the improvement, not the risk.
			s += pickupDelta(hand, ctx.upCard, trump);
		} else {
			const dealerIsPartner = ctx.dealerSeat === partnerOf(ctx.seat);
			const upBoost = TRUMP_VALUE[trumpRank(ctx.upCard, trump)] ?? 0;
			s += (dealerIsPartner ? 0.35 : -0.45) * (0.5 + upBoost);
		}
	}

	if (ctx.phase === 'round2' && ctx.turnedDownSuit !== null) {
		const isNext = sameColor(trump, ctx.turnedDownSuit) && trump !== ctx.turnedDownSuit;
		const role = seatRole(ctx.seat, ctx.dealerSeat);
		s += isNext
			? role === 1
				? 0.45 // "Next." from first seat is the classic call
				: 0.15
			: role === 0 || role === 2
				? 0.2 // "Crossing the creek." belongs to the dealer's side
				: 0;
	}

	return s + scoreAdjust(ctx);
}

/**
 * The dealer's discard search, shared by {@link bestDiscard} and
 * {@link pickupDelta}: try each of the six cards as the discard, value the
 * remaining five, pay `0.18` for every void created and charge `0.05` for every
 * singleton left behind, take the argmax. Ties break on card id.
 */
function discardSearch(
	six: readonly CardId[],
	trump: Suit
): { readonly card: CardId; readonly value: number } {
	let bestCard: CardId = six[0] ?? ('9C' as CardId);
	let bestValue = Number.NEGATIVE_INFINITY;

	for (let i = 0; i < six.length; i++) {
		const candidate = six[i];
		if (candidate === undefined) continue;
		const remaining = withoutIndex(six, i);
		const v = keepValue(remaining, trump);
		if (v > bestValue || (v === bestValue && candidate < bestCard)) {
			bestValue = v;
			bestCard = candidate;
		}
	}

	return { card: bestCard, value: bestValue };
}

/** A copy of `cards` with exactly the element at `index` removed. */
function withoutIndex(cards: readonly CardId[], index: number): CardId[] {
	const out = [...cards];
	out.splice(index, 1);
	return out;
}

/**
 * What a five-card holding is worth to keep: its structure, plus `0.18` for every
 * void it has, minus `0.05` for every singleton off-suit left behind.
 */
function keepValue(kept: readonly CardId[], trump: Suit): number {
	let v = rawHandValue(kept, trump);
	const bySuit = new Map<Suit, number>();
	for (const c of kept) {
		const es = effectiveSuit(c, trump);
		if (es === trump) continue;
		bySuit.set(es, (bySuit.get(es) ?? 0) + 1);
	}
	v += 0.18 * (3 - bySuit.size);
	for (const n of bySuit.values()) if (n === 1) v -= 0.05;
	return v;
}

/**
 * Which of the dealer's six cards to bury. Never throws; with an empty hand it
 * returns `null`.
 */
export function bestDiscard(six: readonly CardId[], trump: Suit): CardId | null {
	if (six.length === 0) return null;
	try {
		return discardSearch(six, trump).card;
	} catch {
		return six[0] ?? null;
	}
}

/**
 * How much better the dealer's hand gets by taking the up-card and burying their
 * worst card. Never negative: the dealer may always discard the up-card itself.
 */
export function pickupDelta(hand: readonly CardId[], upCard: CardId, trump: Suit): number {
	try {
		const baseline = rawHandValue(hand, trump);
		const best = discardSearch([...hand, upCard], trump).value;
		const d = best - baseline;
		return isFiniteNumber(d) && d > 0 ? d : 0;
	} catch {
		return 0;
	}
}

/** The dealer's five cards after taking the up-card and burying the worst one. */
export function handAfterPickup(hand: readonly CardId[], upCard: CardId, trump: Suit): CardId[] {
	const six = [...hand, upCard];
	const drop = bestDiscard(six, trump);
	if (drop === null) return [...hand];
	const out = [...six];
	const at = out.indexOf(drop);
	if (at >= 0) out.splice(at, 1);
	return out;
}

/**
 * The four classic laydown shapes that justify a loner regardless of the
 * numeric score. A fast-path override, not a replacement for the threshold.
 *
 * 1. both bowers + A trump + two off-aces
 * 2. both bowers + A trump + off-A + off-K
 * 3. both bowers + K trump + two off-aces
 * 4. right + A + K trump + two off-aces
 */
export function loneShortlist(hand: readonly CardId[], trump: Suit): boolean {
	const ranksOfTrump = new Set<number>();
	const offAces: CardId[] = [];
	const offKings: CardId[] = [];

	for (const c of hand) {
		if (isTrumpCard(c, trump)) {
			ranksOfTrump.add(trumpRank(c, trump));
			continue;
		}
		const r = rankOf(c);
		if (r === 'A') offAces.push(c);
		else if (r === 'K') offKings.push(c);
	}

	const right = ranksOfTrump.has(8);
	const left = ranksOfTrump.has(7);
	const aceT = ranksOfTrump.has(6);
	const kingT = ranksOfTrump.has(5);

	if (right && left && aceT && offAces.length >= 2) return true;
	if (right && left && aceT && offAces.length >= 1 && offKings.length >= 1) return true;
	if (right && left && kingT && offAces.length >= 2) return true;
	if (right && aceT && kingT && offAces.length >= 2) return true;
	return false;
}

/* ========================================================================== */
/* Trump census and void reads — public information only                       */
/* ========================================================================== */

interface TrumpCensus {
	/** Trump still in my own hand. */
	readonly mine: readonly CardId[];
	/** Trump that is neither mine, nor played, nor known-buried: still out there. */
	readonly outstanding: readonly CardId[];
}

/**
 * Where the seven trump are, from public information plus this seat's own hand.
 *
 * The up-card is counted as *out of play* when it was turned down, because it is
 * buried in the kitty — which is precisely why `PublicGameView` keeps a
 * turned-down up-card publicly identified (V14). An up-card that was **picked
 * up** stays outstanding: it is in the dealer's hand or was buried as their
 * discard, and this seat cannot tell which.
 */
export function trumpCensus(
	view: PublicGameView,
	hand: readonly CardId[],
	trump: Suit
): TrumpCensus {
	const mine = hand.filter((c) => isTrumpCard(c, trump));
	const accounted = new Set<CardId>(mine);
	for (const c of playedCards(view)) if (isTrumpCard(c, trump)) accounted.add(c);
	if (view.upCard !== null && view.upCardTurnedDown && isTrumpCard(view.upCard, trump)) {
		accounted.add(view.upCard);
	}
	return { mine, outstanding: trumpCards(trump).filter((c) => !accounted.has(c)) };
}

/**
 * Which suits each seat has publicly shown a void in, derived from every failure
 * to follow the led suit. Bower-aware via `effectiveSuit`.
 */
export function voidReads(view: PublicGameView): Readonly<Record<Seat, readonly Suit[]>> {
	const acc: Record<Seat, Suit[]> = { 0: [], 1: [], 2: [], 3: [] };
	const trump = view.trump;
	const liveIndex = view.trick.index;

	const scan = (plays: readonly { seat: Seat; card: CardId }[]): void => {
		const first = plays[0];
		if (first === undefined) return;
		const led = effectiveSuit(first.card, trump);
		for (const p of plays) {
			if (effectiveSuit(p.card, trump) === led) continue;
			if (!acc[p.seat].includes(led)) acc[p.seat].push(led);
		}
	};

	for (const t of view.trickLog) {
		if (t.index === liveIndex) continue;
		scan(t.plays);
	}
	scan(view.trick.plays);
	return acc;
}

/* ========================================================================== */
/* The ranking entry point                                                     */
/* ========================================================================== */

interface Scored {
	readonly id: LegalMoveId;
	readonly score: number;
	readonly why: string;
}

/**
 * Order a legal set from best to worst.
 *
 * **This is the only function the AI seat and the table call.** It takes the
 * redacted view, this seat's own cards, and the legal set the engine computed —
 * nothing else. It cannot see another hand because there is no argument capable
 * of carrying one.
 *
 * Guarantees, all of them load-bearing because this is the fallback rung of the
 * decision ladder:
 *
 * - the result is a permutation of `legal` by `id`: never a superset, never a
 *   synthesised id, never empty when `legal` is non-empty;
 * - it never throws — an internal defect degrades to engine order at score `0`;
 * - it is deterministic, ties broken on `id` ascending.
 *
 * @param view  `project(state, seat)` — the redaction boundary.
 * @param hand  This seat's own cards. Defaults to `view.hand`.
 * @param legal The engine's legal set. Defaults to `view.legal`.
 */
export function rankMoves(
	view: PublicGameView,
	hand: readonly CardId[] = view.hand,
	legal: readonly LegalMove[] = view.legal,
	opts: HeuristicOptions = {}
): RankedMove[] {
	if (legal.length === 0) return [];
	try {
		const scored = scoreLegal(view, hand, legal, opts);
		const byId = new Map<LegalMoveId, Scored>();
		for (const s of scored) if (!byId.has(s.id)) byId.set(s.id, s);

		const out: RankedMove[] = legal.map((m) => {
			const s = byId.get(m.id);
			const score = s !== undefined && isFiniteNumber(s.score) ? s.score : 0;
			return { id: m.id, score, why: s?.why ?? '' };
		});

		out.sort((a, b) => (b.score !== a.score ? b.score - a.score : a.id < b.id ? -1 : 1));
		return out;
	} catch {
		// A defect in judgement must never stall a hand. Engine order, flat scores.
		return legal.map((m) => ({ id: m.id, score: 0, why: '' }));
	}
}

/**
 * The single best legal move id, or `null` when there are none. Never throws.
 * This is what rung 2 of the decision ladder plays.
 */
export function topMove(
	view: PublicGameView,
	hand: readonly CardId[] = view.hand,
	legal: readonly LegalMove[] = view.legal,
	opts: HeuristicOptions = {}
): LegalMoveId | null {
	const ranked = rankMoves(view, hand, legal, opts);
	return ranked[0]?.id ?? legal[0]?.id ?? null;
}

/* ========================================================================== */
/* Phase dispatch                                                              */
/* ========================================================================== */

function scoreLegal(
	view: PublicGameView,
	hand: readonly CardId[],
	legal: readonly LegalMove[],
	opts: HeuristicOptions
): Scored[] {
	switch (view.phase) {
		case 'cutting':
			return scoreCut(legal);
		case 'bid_round_1':
			return scoreRound1(view, hand, legal, opts);
		case 'dealer_discard':
			return scoreDiscard(view, hand, legal);
		case 'bid_round_2':
			return scoreRound2(view, hand, legal, opts);
		case 'trick_play':
			return scorePlays(view, hand, legal);
		default:
			return legal.map((m) => ({ id: m.id, score: 0, why: '' }));
	}
}

/* -------------------------------------------------------------------------- */
/* Cutting                                                                     */
/* -------------------------------------------------------------------------- */

/**
 * The cut is ceremony, not judgement — there is no information in it. A stable
 * mild preference for running them keeps the ranking deterministic without
 * pretending to know anything.
 */
function scoreCut(legal: readonly LegalMove[]): Scored[] {
	return legal.map((m) => ({
		id: m.id,
		score: m.id === 'cut:no' ? 0.01 : 0,
		why: m.id === 'cut:no' ? "run 'em" : 'give it a cut'
	}));
}

/* -------------------------------------------------------------------------- */
/* Bidding                                                                     */
/* -------------------------------------------------------------------------- */

function dials(opts: HeuristicOptions): { readonly agg: number; readonly risk: number } {
	return {
		agg: clamp01(opts.aggression ?? 0.5),
		risk: clamp01(opts.risk ?? 0.5)
	};
}

function ctxFor(
	view: PublicGameView,
	phase: StrengthCtx['phase'],
	opts: HeuristicOptions
): StrengthCtx {
	const me: Team = teamOf(view.you);
	return {
		seat: view.you,
		dealerSeat: view.dealerSeat,
		phase,
		upCard: view.upCard,
		turnedDownSuit: view.turnedDownSuit,
		myScore: view.score[me] ?? 0,
		oppScore: view.score[me === 0 ? 1 : 0] ?? 0,
		gameTo: opts.gameTo ?? DEFAULT_ENGINE_CONFIG.gameTo
	};
}

/**
 * Turn a strength and a pair of thresholds into three comparable scores.
 *
 * `pass` sits at `0`, so a call outranks it exactly when the hand clears the
 * calling threshold. The loner term is `2s − A − O`, which is algebraically
 * greater than the plain call's `s − O` exactly when `s > A` — so one comparison
 * expresses two independent thresholds without a branch, and the ranking stays a
 * pure argmax.
 */
function bidScores(
	strength: number,
	callThreshold: number,
	aloneThreshold: number,
	shortlist: boolean
): { readonly call: number; readonly alone: number } {
	const call = strength - callThreshold;
	const alone = shortlist ? call + 0.5 : 2 * strength - aloneThreshold - callThreshold + EPS;
	return { call, alone };
}

function callThresholdFor(base: number, agg: number): number {
	return base - (agg - 0.5) * AGGRESSION_SWING;
}

function aloneThresholdFor(base: number, risk: number): number {
	return base - (risk - 0.5) * RISK_SWING;
}

function scoreRound1(
	view: PublicGameView,
	hand: readonly CardId[],
	legal: readonly LegalMove[],
	opts: HeuristicOptions
): Scored[] {
	const up = view.upCard;
	if (up === null) return legal.map((m) => ({ id: m.id, score: 0, why: '' }));

	const trump = suitOf(up);
	const role = seatRole(view.you, view.dealerSeat);
	const { agg, risk } = dials(opts);
	const strength = handStrength(hand, trump, ctxFor(view, 'round1', opts));

	// The loner is judged on the hand we would actually hold: the dealer's hand
	// is the six-card holding minus the discard.
	const effective = view.you === view.dealerSeat ? handAfterPickup(hand, up, trump) : hand;
	const shortlist = loneShortlist(effective, trump);

	const { call, alone } = bidScores(
		strength,
		callThresholdFor(ORDER_UP_THRESHOLD[role], agg),
		aloneThresholdFor(ALONE_THRESHOLD[role], risk),
		shortlist
	);

	return legal.map((m) => {
		if (m.id === 'pass') return { id: m.id, score: 0, why: 'not enough here' };
		if (m.id === 'orderUp+alone') {
			return {
				id: m.id,
				score: alone,
				why: shortlist ? 'this one plays itself' : 'I can see three'
			};
		}
		if (m.id === 'orderUp') {
			return {
				id: m.id,
				score: call,
				why: view.you === view.dealerSeat ? 'worth picking up' : 'good enough to call'
			};
		}
		return { id: m.id, score: 0, why: '' };
	});
}

function scoreRound2(
	view: PublicGameView,
	hand: readonly CardId[],
	legal: readonly LegalMove[],
	opts: HeuristicOptions
): Scored[] {
	const role = seatRole(view.you, view.dealerSeat);
	const { agg, risk } = dials(opts);
	const turnedDown = view.turnedDownSuit;
	const ctx = ctxFor(view, 'round2', opts);

	const perSuit = new Map<Suit, { call: number; alone: number; shortlist: boolean }>();
	for (const suit of SUITS) {
		if (turnedDown !== null && suit === turnedDown) continue;
		const strength = handStrength(hand, suit, ctx);
		const isNext = turnedDown !== null && sameColor(suit, turnedDown) && suit !== turnedDown;
		const base = isNext ? NEXT_THRESHOLD[role] : CROSS_THRESHOLD[role];
		const shortlist = loneShortlist(hand, suit);
		const scores = bidScores(
			strength,
			callThresholdFor(base, agg),
			aloneThresholdFor(ALONE_THRESHOLD[role], risk),
			shortlist
		);
		perSuit.set(suit, { call: scores.call, alone: scores.alone, shortlist });
	}

	return legal.map((m) => {
		if (m.id === 'pass') return { id: m.id, score: 0, why: 'nothing to name' };
		if (!m.id.startsWith('call:')) return { id: m.id, score: 0, why: '' };

		const rest = m.id.slice('call:'.length);
		const alone = rest.endsWith('+alone');
		const suitChar = rest.slice(0, 1);
		const suit = SUITS.find((s) => s === suitChar);
		if (suit === undefined) return { id: m.id, score: 0, why: '' };

		const e = perSuit.get(suit);
		if (e === undefined) return { id: m.id, score: 0, why: '' };
		const isNext = turnedDown !== null && sameColor(suit, turnedDown) && suit !== turnedDown;
		return {
			id: m.id,
			score: alone ? e.alone : e.call,
			why: alone
				? e.shortlist
					? 'this one plays itself'
					: 'I can see three'
				: isNext
					? 'next is the call here'
					: 'crossing the creek'
		};
	});
}

function scoreDiscard(
	view: PublicGameView,
	hand: readonly CardId[],
	legal: readonly LegalMove[]
): Scored[] {
	const trump = view.trump;
	if (trump === null) return legal.map((m) => ({ id: m.id, score: 0, why: '' }));

	// `hand` at this point is the dealer's six cards: five dealt plus the up-card.
	return legal.map((m) => {
		if (!m.id.startsWith('discard:')) return { id: m.id, score: 0, why: '' };
		const card = m.id.slice('discard:'.length) as CardId;
		const at = hand.indexOf(card);
		const kept = at < 0 ? [...hand] : withoutIndex(hand, at);
		return {
			id: m.id,
			score: keepValue(kept, trump),
			why: isTrumpCard(card, trump) ? 'hate to break trump' : 'bury the useless one'
		};
	});
}

/* -------------------------------------------------------------------------- */
/* Card play                                                                   */
/* -------------------------------------------------------------------------- */

function scorePlays(
	view: PublicGameView,
	hand: readonly CardId[],
	legal: readonly LegalMove[]
): Scored[] {
	const trump = view.trump;
	if (trump === null) return legal.map((m) => ({ id: m.id, score: 0, why: '' }));

	const me = view.you;
	const myTeam = teamOf(me);
	const partner = partnerOf(me);
	const plays = view.trick.plays;
	const leading = plays.length === 0;
	const seatsAfterMe = Math.max(0, playsPerTrick(view.sittingSeat) - plays.length - 1);
	const census = trumpCensus(view, hand, trump);
	const voids = voidReads(view);
	const trickNo = view.trick.index;

	const makerTeam: Team | null = view.makerSeat === null ? null : teamOf(view.makerSeat);
	const iAmMaker = makerTeam !== null && makerTeam === myTeam;
	const defendingLoner = view.aloneSeat !== null && teamOf(view.aloneSeat) !== myTeam;
	const myTricks = view.tricksWon[myTeam] ?? 0;
	const partnerWinning = currentlyWinning(plays, trump)?.seat === partner;

	// Counts of my own cards by effective suit, for "guarded" reads.
	const bySuit = new Map<Suit, CardId[]>();
	for (const c of hand) {
		const es = effectiveSuit(c, trump);
		const b = bySuit.get(es);
		if (b === undefined) bySuit.set(es, [c]);
		else b.push(c);
	}
	const myTrumpCount = census.mine.length;

	const base = (card: CardId): Scored => {
		const p = power(card, trump);
		const es = effectiveSuit(card, trump);
		const isTrump = es === trump;
		// Conservation compares trump and plain suits on one scale. The old
		// rank/8 vs rank/6 scale could prefer throwing trump over a plain king.
		const cost = isTrump ? (8 + trumpRank(card, trump)) / 16 : plainRank(card) / 16;

		if (leading) {
			// Maker's side, trick 1: draw their trump or cash before the ruff.
			if (iAmMaker && trickNo === 0 && myTrumpCount >= 3 && isTrump) {
				return { id: `play:${card}`, score: 1.6 + p, why: 'pull their trump' };
			}
			if (iAmMaker && trickNo === 0 && myTrumpCount <= 2 && !isTrump && rankOf(card) === 'A') {
				return { id: `play:${card}`, score: 1.5 + p, why: 'cash it before the ruff' };
			}
			// The best trump left in the world is good — take it now.
			const highestOut = census.outstanding[0];
			if (
				isTrump &&
				census.outstanding.length <= 1 &&
				(highestOut === undefined || trumpRank(card, trump) > trumpRank(highestOut, trump))
			) {
				return { id: `play:${card}`, score: 1.5, why: 'this one is good' };
			}
			if (!isTrump && rankOf(card) === 'A' && (bySuit.get(es)?.length ?? 0) >= 2) {
				const risky = [0, 1, 2, 3].some(
					(s) =>
						teamOf(s as Seat) !== myTeam &&
						(voids[s as Seat] ?? []).includes(es) &&
						census.outstanding.length > 0
				);
				return {
					id: `play:${card}`,
					score: (risky ? 1.15 : 1.3) + p,
					why: risky ? 'they may be out of it' : 'the ace is good'
				};
			}
			if (!isTrump) {
				const len = bySuit.get(es)?.length ?? 1;
				return {
					id: `play:${card}`,
					score: 0.5 + 0.12 * len + 0.2 * p,
					why: 'lead from length'
				};
			}
			return { id: `play:${card}`, score: 0.4 + 0.3 * p, why: 'ask for a bower' };
		}

		// Following.
		const first = plays[0];
		const led: Suit = first === undefined ? es : effectiveSuit(first.card, trump);
		const winning = currentlyWinning(plays, trump);
		const canWin =
			winning === null || cardValue(card, led, trump) > cardValue(winning.card, led, trump);

		if (partnerWinning) {
			return seatsAfterMe === 0
				? { id: `play:${card}`, score: 1.4 - cost, why: 'lay off, it is theirs' }
				: { id: `play:${card}`, score: 1.0 - 0.8 * cost, why: 'save the good one' };
		}
		if (canWin) {
			if (seatsAfterMe === 0) {
				return { id: `play:${card}`, score: 1.7 - 0.5 * p, why: 'cheapest card that takes it' };
			}
			if (seatsAfterMe === 1) {
				return { id: `play:${card}`, score: 1.2 + 0.4 * p, why: 'third hand high' };
			}
			return { id: `play:${card}`, score: 0.9 + 0.3 * p, why: 'second hand, only if it matters' };
		}
		return { id: `play:${card}`, score: 0.6 - cost, why: 'cannot win, keep the good ones' };
	};

	return legal.map((m) => {
		if (!m.id.startsWith('play:')) return { id: m.id, score: 0, why: '' };
		const card = m.id.slice('play:'.length) as CardId;
		const s = base(card);
		let score = s.score;
		let why = s.why;

		// Euchre avoidance: two tricks in the bank, guarantee the third.
		if (
			!partnerWinning &&
			iAmMaker &&
			myTricks === 2 &&
			isTrumpCard(card, trump) &&
			trumpRank(card, trump) >= 6
		) {
			score += 0.35;
			why = 'take the third and be done';
		}
		// Stop the four-point sweep; partner's trick already does that job.
		if (
			!partnerWinning &&
			defendingLoner &&
			myTricks === 0 &&
			isTrumpCard(card, trump) &&
			trumpRank(card, trump) >= 7
		) {
			score += 0.4;
			why = 'stop the loner sweep';
		}

		return { id: m.id, score, why };
	});
}
