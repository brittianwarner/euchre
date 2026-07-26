/**
 * `optimistic.ts` — the browser's local move preview.
 *
 * The client must show your card leaving your hand the instant you tap it, not
 * after a round trip to Rivet Cloud. `applyOptimistic` produces the view the
 * server is *about* to send, computed locally from the view you already hold.
 *
 * ## The contract (V22)
 *
 * For every state reachable by a seat and every `LegalMove` in that seat's set:
 *
 * ```
 * applyOptimistic(project(s, seat), move)  ≡  project(apply(s, seat, move).state, seat)
 * ```
 *
 * on every field listed in {@link OPTIMISTIC_FIELDS}. Fields outside that set are
 * server-authoritative and are deliberately left at their previous value — the
 * next `sync` event overwrites the whole view anyway, so a stale field costs one
 * frame, whereas a *wrong* field would render a lie.
 *
 * ## Why this is not a second rulebook
 *
 * This file computes no legality and decides no transition. It reuses
 * {@link effectiveSuit} and {@link trickWinner} from the engine for the two
 * derivations it cannot avoid, and otherwise only moves a card from one array to
 * another. The authoritative reducer remains the single source of truth: if this
 * function and `apply` ever disagree, the `sync` event wins, and the V22 test
 * fails loudly in CI.
 *
 * @see 02-GAME-RULES-ENGINE.md §6.5 and invariant V22
 */

import { effectiveSuit } from './cards';
import { trickWinner } from './trick';
import { partnerOf } from './seats';
import type { CardId, Move, PublicGameView, Seat, Suit, Trick } from './types';

/**
 * The fields {@link applyOptimistic} is contractually required to get exactly
 * right. Everything else in `PublicGameView` is left untouched and refreshed by
 * the next server `sync`.
 */
export const OPTIMISTIC_FIELDS = Object.freeze([
	'hand',
	'handCounts',
	'trick',
	'kittyCount',
	'passes',
	'trump',
	'makerSeat',
	'aloneSeat',
	'sittingSeat',
	'upCardTurnedDown',
	'turnedDownSuit'
] as const);

/**
 * `legal` is deliberately NOT in the contract.
 *
 * Reproducing the server's legal set would mean reproducing its move labels, i.e.
 * a second rulebook — exactly what binding rule 1 forbids. Instead this module
 * always clears `legal` after your move: you have acted, so nothing is offered
 * until the server says otherwise. That errs in the safe direction (the UI never
 * offers MORE than the server would allow) and the next `sync` restores the real
 * set a few milliseconds later.
 */

export type OptimisticField = (typeof OPTIMISTIC_FIELDS)[number];

/** Remove the first occurrence of `card`, returning a new array. */
function without(hand: readonly CardId[], card: CardId): readonly CardId[] {
	const i = hand.indexOf(card);
	if (i < 0) return hand;
	return [...hand.slice(0, i), ...hand.slice(i + 1)];
}

function decrement(
	counts: readonly [number, number, number, number],
	seat: Seat
): readonly [number, number, number, number] {
	const next: [number, number, number, number] = [...counts] as [number, number, number, number];
	next[seat] = Math.max(0, next[seat] - 1);
	return next;
}

/**
 * Append a play to the current trick, deriving `ledSuit` on the opening card and
 * `winnerSeat` once the trick is full.
 *
 * "Full" is 4 plays normally and 3 when someone is playing alone, which is why
 * `sittingSeat` is a parameter rather than an assumption.
 */
function appendPlay(
	trick: Trick,
	seat: Seat,
	card: CardId,
	trump: Suit | null,
	sittingSeat: Seat | null
): Trick {
	const plays = [...trick.plays, { seat, card }];
	const ledSuit = trick.plays.length === 0 ? effectiveSuit(card, trump) : trick.ledSuit;
	const expected = sittingSeat === null ? 4 : 3;
	const complete = plays.length >= expected && ledSuit !== null && trump !== null;
	return {
		index: trick.index,
		ledSuit,
		plays,
		winnerSeat: complete ? trickWinner(plays, ledSuit, trump) : trick.winnerSeat
	};
}

/**
 * Compute the view the server is about to produce for this seat, locally.
 *
 * `move` **must** come from `view.legal` — this function assumes the move is
 * already legal and does not re-check it. Passing an arbitrary move produces a
 * meaningless view; the server would reject it anyway, and the resulting `sync`
 * would correct the display.
 *
 * Returns `view` unchanged (identity) when the move is one whose effects this
 * function does not model, so a caller can always use the result unconditionally.
 */
export function applyOptimistic(view: PublicGameView, move: Move): PublicGameView {
	const you = view.you;

	switch (move.t) {
		case 'play': {
			if (!view.hand.includes(move.card)) return view;
			return {
				...view,
				hand: without(view.hand, move.card),
				handCounts: decrement(view.handCounts, you),
				trick: appendPlay(view.trick, you, move.card, view.trump, view.sittingSeat),
				legal: []
			};
		}

		case 'discard': {
			if (!view.hand.includes(move.card)) return view;
			// kittyCount is `3 + (upCardTurnedDown || dealerDiscard !== null ? 1 : 0)`.
			// A discard is exactly the event that makes `dealerDiscard` non-null.
			return {
				...view,
				hand: without(view.hand, move.card),
				handCounts: decrement(view.handCounts, you),
				kittyCount: view.upCardTurnedDown ? view.kittyCount : 4,
				legal: []
			};
		}

		case 'pass': {
			const passes = view.passes + 1;

			// The fourth pass in round one is the dealer turning it down: the up-card
			// is buried (so kittyCount reaches 4), its suit becomes uncallable, and
			// the pass counter resets for the second round.
			if (passes >= 4 && view.phase === 'bid_round_1' && view.upCard) {
				return {
					...view,
					passes: 0,
					kittyCount: 4,
					upCardTurnedDown: true,
					turnedDownSuit: view.upCard[1] as Suit,
					legal: []
				};
			}

			return { ...view, passes, legal: [] };
		}

		case 'orderUp': {
			// Accepting the up-card makes its PRINTED suit trump and hands it to the
			// dealer in the same move — the dealer is at six cards until they discard.
			const trump = view.upCard ? (view.upCard[1] as Suit) : view.trump;
			const sitting = move.alone ? partnerOf(you) : null;
			const counts: [number, number, number, number] = [...view.handCounts] as [
				number,
				number,
				number,
				number
			];

			// The edge case: when a loner's sitting partner IS the dealer, the dealer
			// never picks up and never discards — a mandatory `dealer_discard` would
			// deadlock, since a sitting seat's legal set is empty by rule (V8). The
			// reducer collapses pickup+discard to a no-op, which sets `dealerDiscard`
			// to the up-card immediately and therefore pushes `kittyCount` to 4.
			const dealerSitsOut = sitting !== null && sitting === view.dealerSeat;
			if (!dealerSitsOut) counts[view.dealerSeat] = counts[view.dealerSeat] + 1;
			if (sitting !== null) counts[sitting] = 0;

			// When YOU are the dealer, the up-card lands in the hand you can see.
			const hand =
				you === view.dealerSeat && !dealerSitsOut && view.upCard
					? [...view.hand, view.upCard]
					: view.hand;

			return {
				...view,
				hand,
				trump,
				makerSeat: you,
				aloneSeat: move.alone ? you : null,
				sittingSeat: sitting,
				handCounts: counts,
				kittyCount: dealerSitsOut ? 4 : view.kittyCount,
				legal: []
			};
		}

		case 'call': {
			// Round two names a suit outright: no up-card changes hands, so the only
			// count that moves is a sitting partner's, which drops to zero.
			const sitting = move.alone ? partnerOf(you) : null;
			const counts: [number, number, number, number] = [...view.handCounts] as [
				number,
				number,
				number,
				number
			];
			if (sitting !== null) counts[sitting] = 0;

			return {
				...view,
				trump: move.suit,
				makerSeat: you,
				aloneSeat: move.alone ? you : null,
				sittingSeat: sitting,
				handCounts: counts,
				legal: []
			};
		}

		// `cut` is deliberately OUTSIDE the optimistic contract: answering the cut
		// triggers the deal, and the deal comes from the server-held seeded shuffle.
		// Nothing about it is locally derivable, so previewing it would be inventing
		// cards. The client shows a dealing animation and waits for the `sync`.
		case 'cut':
			return view;
	}
}
