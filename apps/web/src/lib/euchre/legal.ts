/**
 * `legal.ts` — what may happen next.
 *
 * Two layers. {@link getLegalPlays} is the card-level follow-suit primitive.
 * {@link legalMoves} is the move-level API every consumer actually calls: the
 * authoritative table (to validate), the AI seats (to build their `z.enum`), the
 * browser (to render and to hit-test), and the coach.
 *
 * Because the legal set is computed here and nowhere else, reneging is
 * structurally impossible: an illegal card is absent from the set, unclickable in
 * the client, and never offered to the model.
 */

import { SUITS, effectiveSuit, isLeftBower, rankOf, suitOf } from './cards';
import { partnerOf } from './seats';
import type { CardId, GameState, LegalMove, Rank, Seat, Suit } from './types';

/* -------------------------------------------------------------------------- */
/* Naming                                                                      */
/* -------------------------------------------------------------------------- */

/** Display name of a suit, e.g. `"Hearts"`. Also the round-2 call label. */
export const SUIT_NAME: Readonly<Record<Suit, string>> = Object.freeze({
	S: 'Spades',
	H: 'Hearts',
	D: 'Diamonds',
	C: 'Clubs'
});

/** Singular, lowercase suit noun, e.g. `"heart"` — for mid-sentence copy. */
export const SUIT_NOUN: Readonly<Record<Suit, string>> = Object.freeze({
	S: 'spade',
	H: 'heart',
	D: 'diamond',
	C: 'club'
});

/** Display name of a rank, e.g. `"Queen"`. */
export const RANK_NAME: Readonly<Record<Rank, string>> = Object.freeze({
	'9': 'Nine',
	T: 'Ten',
	J: 'Jack',
	Q: 'Queen',
	K: 'King',
	A: 'Ace'
});

/**
 * `"Jack of diamonds"` — the spoken/UI label for a card, also read aloud by
 * `HandA11y`. Always names the **printed** card; trump status is surfaced
 * separately so a screen reader hears both facts.
 */
export const cardName = (c: CardId): string =>
	`${RANK_NAME[rankOf(c)]} of ${SUIT_NAME[suitOf(c)].toLowerCase()}`;

/** `"jack of diamonds"` — {@link cardName} lowercased, for mid-sentence copy. */
export const cardNameLower = (c: CardId): string =>
	`${RANK_NAME[rankOf(c)].toLowerCase()} of ${SUIT_NAME[suitOf(c)].toLowerCase()}`;

/**
 * The round-1 label for accepting the up-card. The move id is `orderUp` for all
 * four seats; only the spoken line differs by role.
 */
export function orderLabel(seat: Seat, dealer: Seat): string {
	if (seat === dealer) return 'I take it';
	if (seat === partnerOf(dealer)) return 'I assist';
	return 'Order it up';
}

/** Joins names as `"a"`, `"a and b"`, `"a, b and c"`. */
function joinList(items: readonly string[]): string {
	if (items.length <= 1) return items[0] ?? '';
	return `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]!}`;
}

/* -------------------------------------------------------------------------- */
/* The card-level primitive                                                    */
/* -------------------------------------------------------------------------- */

/**
 * The cards this hand may legally play.
 *
 * A player **must follow the led suit if able**, measured by `effectiveSuit` —
 * so with hearts trump a hand of `JD 9C TC KC AC` is void in diamonds, and `JD`
 * is the only legal play to a heart lead. If void, they may play anything: there
 * is no obligation to trump and none to overtrump.
 *
 * `ledSuit === null` means this seat is on lead, so everything is legal.
 *
 * Returns a new array; never mutates `hand`. Never returns `[]` for a non-empty
 * hand (V19).
 */
export function getLegalPlays(
	hand: readonly CardId[],
	ledSuit: Suit | null,
	trump: Suit | null
): CardId[] {
	if (ledSuit === null) return hand.slice();
	const following = hand.filter((c) => effectiveSuit(c, trump) === ledSuit);
	return following.length > 0 ? following : hand.slice();
}

/**
 * Why a specific card may not be played — the learner-facing copy behind the
 * `Shift+←/→` path and the `IllegalWhy` panel. Returns `null` when the play is
 * legal.
 *
 * The interesting case is the one this text exists for: the led suit is trump and
 * the seat is holding the left bower without realising it is trump.
 *
 * @example whyIllegal(["JD","9C"], "9C", "H", "H")
 *   === "Hearts were led. Your Jack of diamonds is a heart right now — you have to follow."
 */
export function whyIllegal(
	hand: readonly CardId[],
	card: CardId,
	ledSuit: Suit | null,
	trump: Suit | null
): string | null {
	if (!hand.includes(card)) return "That card isn't in your hand.";
	if (ledSuit === null) return null;
	if (effectiveSuit(card, trump) === ledSuit) return null;

	const following = hand.filter((c) => effectiveSuit(c, trump) === ledSuit);
	if (following.length === 0) return null;

	const opener = `${SUIT_NAME[ledSuit]} were led.`;
	const bower = following.find((c) => isLeftBower(c, trump));
	if (bower !== undefined) {
		return `${opener} Your ${cardName(bower)} is a ${SUIT_NOUN[ledSuit]} right now — you have to follow.`;
	}
	const held = joinList(following.map((c) => `the ${cardNameLower(c)}`));
	return `${opener} You still hold ${held} — you have to follow suit.`;
}

/* -------------------------------------------------------------------------- */
/* Stick the dealer                                                            */
/* -------------------------------------------------------------------------- */

/**
 * The stuck-dealer predicate, encoded exactly once (V3): stick-the-dealer is on,
 * we are in round 2, this seat is the dealer, and the other three have passed.
 *
 * When it holds, `pass` is **removed** from the legal set — the UI omits the
 * button rather than disabling it — and a `pass` arriving anyway must be rejected
 * with `RuleCode` `"dealer_must_call"`, never `"bidding_closed"`.
 */
export function isStuckDealer(state: GameState, seat: Seat): boolean {
	const h = state.hand;
	return (
		state.cfg.stickTheDealer && h.phase === 'bid_round_2' && seat === h.dealerSeat && h.passes === 3
	);
}

/* -------------------------------------------------------------------------- */
/* The move-level API                                                          */
/* -------------------------------------------------------------------------- */

/**
 * The single source of truth for what `seat` may do right now.
 *
 * Returns `[]` for any seat that is not to act — which is exactly what
 * `PublicGameView.legal` needs, since that field is empty unless
 * `turnSeat === you` — and `[]` for the loner's sitting partner in every phase.
 *
 * In any phase with a non-null `turnSeat`, the acting seat's set is **never
 * empty** (V19). That guarantee is what makes the AI's `z.enum(legalMoveIds)`
 * always constructible and the 240 s abandon auto-play always able to move.
 *
 * Pure: never mutates `state`, and every returned array and object is fresh.
 */
export function legalMoves(state: GameState, seat: Seat): LegalMove[] {
	const h = state.hand;
	if (state.status !== 'active') return [];
	if (seat === h.sittingSeat) return [];
	if (h.turnSeat !== seat) return [];

	switch (h.phase) {
		case 'cutting':
			return [
				{ id: 'cut:yes', move: { t: 'cut', cut: true }, label: 'Bump' },
				{ id: 'cut:no', move: { t: 'cut', cut: false }, label: "Run 'em" }
			];

		case 'bid_round_1': {
			const order = orderLabel(seat, h.dealerSeat);
			if (
				state.cfg.requireNaturalTrump &&
				h.upCard &&
				seat !== h.dealerSeat &&
				!h.hands[seat].some((c) => c[1] === h.upCard![1])
			)
				return [{ id: 'pass', move: { t: 'pass' }, label: 'Pass' }];
			return [
				{ id: 'pass', move: { t: 'pass' }, label: 'Pass' },
				{ id: 'orderUp', move: { t: 'orderUp', alone: false }, label: order },
				{ id: 'orderUp+alone', move: { t: 'orderUp', alone: true }, label: `${order}, alone` }
			];
		}

		case 'dealer_discard':
			// Six cards after the pickup; discarding the up-card itself is legal.
			return h.hands[seat].map((c): LegalMove => ({
				id: `discard:${c}`,
				move: { t: 'discard', card: c },
				label: cardName(c)
			}));

		case 'bid_round_2': {
			const out: LegalMove[] = isStuckDealer(state, seat)
				? []
				: [{ id: 'pass', move: { t: 'pass' }, label: 'Pass' }];
			for (const s of SUITS) {
				if (s === h.turnedDownSuit) continue;
				if (
					state.cfg.requireNaturalTrump &&
					!isStuckDealer(state, seat) &&
					!h.hands[seat].some((c) => c[1] === s)
				)
					continue;
				out.push({
					id: `call:${s}`,
					move: { t: 'call', suit: s, alone: false },
					label: SUIT_NAME[s]
				});
				out.push({
					id: `call:${s}+alone`,
					move: { t: 'call', suit: s, alone: true },
					label: `${SUIT_NAME[s]}, alone`
				});
			}
			return out;
		}

		case 'trick_play':
			return getLegalPlays(h.hands[seat], h.trick.ledSuit, h.trump).map((c): LegalMove => ({
				id: `play:${c}`,
				move: { t: 'play', card: c },
				label: cardName(c)
			}));

		default:
			// lobby, deal, trick_resolve, hand_score, game_over — engine-only phases.
			return [];
	}
}

/**
 * The legal move ids for `seat`, in the same order as {@link legalMoves}. This is
 * the exact vocabulary the AI is constrained to and the exact whitelist the
 * client hit-tests against.
 */
export function legalMoveIds(state: GameState, seat: Seat): string[] {
	return legalMoves(state, seat).map((m) => m.id);
}

/**
 * Looks up one offered move by id, or `null` if it is not legal for `seat` right
 * now. The reducer's single lookup point, so "is this legal" and "what is this"
 * can never disagree.
 */
export function findLegalMove(state: GameState, seat: Seat, id: string): LegalMove | null {
	return legalMoves(state, seat).find((m) => m.id === id) ?? null;
}
