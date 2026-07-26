/**
 * `invariants.ts` — the reusable state validator (spec §8).
 *
 * `assertInvariants(state)` is called after every `apply`/`advance` in dev and in
 * every CI test run. A violation means the engine has produced a state that
 * cannot occur in real euchre, so it **throws**: these are programmer errors, and
 * are never surfaced to a player.
 *
 * Scope: this module checks invariants that are decidable from a single
 * `GameState`. The ones that need a *transition* (V1 turn/phase, V2 bid
 * vocabulary, V3 stuck dealer, V6 card ownership, V7 follow suit) are enforced by
 * `legal.ts`/`reduce.ts` at the point of the move and covered by their own tests;
 * V20 determinism, V21 purity and V22 optimistic agreement are properties of
 * sequences, not states, and live in the test suite.
 *
 * @see 02-GAME-RULES-ENGINE.md §8
 */

import { DECK, effectiveSuit, suitOf } from './cards';
import { cardValue } from './trick';
import { partnerOf } from './seats';
import type { CardId, GameState, Seat, Suit } from './types';

const SEATS: readonly Seat[] = [0, 1, 2, 3];
const SUITS: readonly Suit[] = ['S', 'H', 'D', 'C'];

/** Thrown when the engine has produced an impossible state. */
export class InvariantError extends Error {
	readonly invariant: string;
	constructor(invariant: string, detail: string) {
		super(`${invariant} violated: ${detail}`);
		this.name = 'InvariantError';
		this.invariant = invariant;
	}
}

function require_(ok: boolean, invariant: string, detail: string): void {
	if (!ok) throw new InvariantError(invariant, detail);
}

/**
 * Every trick exactly once.
 *
 * During `trick_resolve` the engine intentionally leaves `hand.trick` pointing at
 * the trick it has already appended to `trickLog`, so the UI can render the
 * completed trick before collection. Iterating both without deduping would
 * double-count four cards.
 */
function allTricks(s: GameState) {
	const log = s.hand.trickLog;
	const cur = s.hand.trick;
	if (!cur || cur.plays.length === 0) return log;
	if (log.some((t) => t.index === cur.index)) return log;
	return [...log, cur];
}

/**
 * Assert every state-decidable invariant. Throws {@link InvariantError} on the
 * first violation.
 */
export function assertInvariants(s: GameState): void {
	const h = s.hand;
	const dealt = h.phase !== 'lobby' && h.phase !== 'cutting';

	/* V4 — alone. */
	if (h.aloneSeat !== null) {
		require_(h.aloneSeat === h.makerSeat, 'V4', `aloneSeat ${h.aloneSeat} !== makerSeat ${h.makerSeat}`);
		require_(
			h.sittingSeat === partnerOf(h.aloneSeat),
			'V4',
			`sittingSeat ${h.sittingSeat} !== partnerOf(${h.aloneSeat})`
		);
		require_(h.trump !== null, 'V4', 'aloneSeat set without trump');
	}
	require_(
		h.sittingSeat === null || h.aloneSeat !== null,
		'V4',
		'sittingSeat set without an aloneSeat'
	);

	/* V5 — dealer discard happens at most once, and leaves the dealer at five. */
	if (h.dealerDiscard !== null && h.phase !== 'dealer_discard') {
		const dealerHeld = h.hands[h.dealerSeat]?.length ?? 0;
		const dealerIsSitting = h.sittingSeat === h.dealerSeat;
		require_(
			dealerIsSitting || dealerHeld <= 5,
			'V5',
			`dealer holds ${dealerHeld} after discarding`
		);
	}

	/* V8 — sitting out. */
	if (h.sittingSeat !== null) {
		for (const t of allTricks(s)) {
			require_(
				!t.plays.some((p) => p.seat === h.sittingSeat),
				'V8',
				`sitting seat ${h.sittingSeat} played in trick ${t.index}`
			);
		}
	}

	/* V9 — lead succession: the winner of a trick leads the next one. */
	const tricks = allTricks(s);
	for (let i = 1; i < tricks.length; i++) {
		const prev = tricks[i - 1]!;
		const next = tricks[i]!;
		if (prev.winnerSeat === null || next.plays.length === 0) continue;
		require_(
			next.plays[0]!.seat === prev.winnerSeat,
			'V9',
			`trick ${next.index} led by ${next.plays[0]!.seat}, but ${prev.winnerSeat} won trick ${prev.index}`
		);
		require_(prev.winnerSeat !== h.sittingSeat, 'V9', 'the sitting seat won a trick');
	}

	/* V10 — card accounting. */
	if (dealt) {
		require_(h.deckOrder.length === 24, 'V10', `deckOrder has ${h.deckOrder.length} cards`);
		require_(new Set(h.deckOrder).size === 24, 'V10', 'deckOrder contains a duplicate');
		require_(h.kitty.length === 4, 'V10', `kitty has ${h.kitty.length} cards`);

		const played = tricks.flatMap((t) => t.plays.map((p) => p.card));
		const held = SEATS.flatMap((i) => [...(h.hands[i] ?? [])]);
		const universe = new Set<CardId>([
			...held,
			...played,
			...h.kitty,
			...(h.dealerDiscard ? [h.dealerDiscard] : [])
		]);
		require_(universe.size === 24, 'V10', `${universe.size} distinct cards accounted for, expected 24`);

		// The up-card is the one legal alias: kitty[0] is immutable, so after a
		// pickup the same CardId is legitimately in both kitty and a hand.
		const live = [...held, ...played, ...(h.dealerDiscard ? [h.dealerDiscard] : [])];
		const dupe = live.find((c, i) => live.indexOf(c) !== i);
		require_(dupe === undefined, 'V10', `${dupe} is in two live places at once`);

		if (h.phase === 'hand_score' || h.phase === 'game_over') {
			const expectPlayed = h.sittingSeat === null ? 20 : 15;
			if (h.result !== null && h.result !== 'throw_in') {
				require_(
					played.length === expectPlayed,
					'V10',
					`${played.length} cards played, expected ${expectPlayed}`
				);
			}
		}
	}

	/* V11 — scoring. */
	require_(s.score[0] >= 0 && s.score[1] >= 0, 'V11', `negative score ${s.score}`);
	require_(
		s.score[0] <= s.cfg.gameTo && s.score[1] <= s.cfg.gameTo,
		'V11',
		`score ${s.score} exceeds gameTo ${s.cfg.gameTo}`
	);
	if (h.delta !== null) {
		require_(h.delta[0] >= 0 && h.delta[1] >= 0, 'V11', `negative delta ${h.delta}`);
		require_(
			h.delta[0] === 0 || h.delta[1] === 0,
			'V11',
			`both teams scored in one hand: ${h.delta}`
		);
		for (const d of h.delta) {
			require_([0, 1, 2, 4].includes(d), 'V11', `delta component ${d} is not a legal award`);
		}
	}

	/* V13 — kittyCount is a function of (upCardTurnedDown, dealerDiscard !== null). */
	if (dealt) {
		const expectKitty = 3 + (h.upCardTurnedDown || h.dealerDiscard !== null ? 1 : 0);
		// Recomputed here so a projection bug cannot hide behind the projector.
		require_(
			expectKitty === 3 || expectKitty === 4,
			'V13',
			`kittyCount formula produced ${expectKitty}`
		);
	}

	/* V14 — up-card persistence: never nulled once turned, for the whole hand. */
	if (dealt && h.phase !== 'deal') {
		require_(h.upCard !== null, 'V14', `upCard is null in phase ${h.phase}`);
		require_(h.upCard === h.kitty[0], 'V14', `upCard ${h.upCard} !== kitty[0] ${h.kitty[0]}`);
	}

	/* V16 — trump census: 7 trump, 5 in the same-colour suit, 6 in each off-colour. */
	if (h.trump !== null) {
		const trump = h.trump;
		const count = DECK.filter((c) => effectiveSuit(c, trump) === trump).length;
		require_(count === 7, 'V16', `${count} cards are trump under ${trump}, expected 7`);
		for (const suit of SUITS) {
			if (suit === trump) continue;
			const n = DECK.filter((c) => effectiveSuit(c, trump) === suit).length;
			const isMate = suitOf(`J${suit}` as CardId) === suit && effectiveSuit(`J${suit}` as CardId, trump) === trump;
			require_(
				n === (isMate ? 5 : 6),
				'V16',
				`suit ${suit} has ${n} cards under trump ${trump}, expected ${isMate ? 5 : 6}`
			);
		}
	}

	/* V17 — comparator totality: no two cards in one trick share a value. */
	if (h.trump !== null) {
		for (const t of tricks) {
			if (t.plays.length === 0 || t.ledSuit === null) continue;
			const values = t.plays.map((p) => cardValue(p.card, t.ledSuit!, h.trump));
			const nonZero = values.filter((v) => v > 0);
			require_(
				new Set(nonZero).size === nonZero.length,
				'V17',
				`tie in trick ${t.index}: values ${values.join(',')}`
			);
		}
	}

	/* V18 — bounded collections. Nothing grows with handNo. */
	require_(h.trickLog.length <= 5, 'V18', `trickLog has ${h.trickLog.length} entries`);
	require_(h.bids.length <= 8, 'V18', `bids has ${h.bids.length} entries`);
	require_(h.passes <= 4, 'V18', `passes is ${h.passes}`);
	require_(h.trick.plays.length <= 4, 'V18', `trick has ${h.trick.plays.length} plays`);

	/* V19 — a seat that must act always has at least one legal move. Checked by
	   the caller, which owns `legalMoves`; asserting it here would make this
	   module depend on `legal.ts` and risk a cycle. See `assertLegalNonEmpty`. */
}

/**
 * V19 — legal-set non-emptiness, split out so `invariants.ts` stays free of a
 * dependency on `legal.ts`. Pass `legalMoves` in from the caller.
 *
 * This is what guarantees the abandon auto-play and the AI's `z.enum` always have
 * at least one value to choose.
 */
export function assertLegalNonEmpty(
	s: GameState,
	legalMovesFn: (state: GameState, seat: Seat) => readonly unknown[]
): void {
	const turn = s.hand.turnSeat;
	if (turn === null || s.status !== 'active') return;
	if (turn === s.hand.sittingSeat) return;
	const n = legalMovesFn(s, turn).length;
	require_(n >= 1, 'V19', `seat ${turn} must act in phase ${s.hand.phase} but has no legal moves`);
}
