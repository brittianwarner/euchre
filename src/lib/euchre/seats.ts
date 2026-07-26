/**
 * `seats.ts` — seat arithmetic, team membership, and loner-aware turn order.
 *
 * Seats are chairs, fixed for the life of the match; roles rotate with the
 * dealer. Every function here is total, allocation-free, and pure.
 */

import type { Seat, SeatRole, Team } from './types';

/** The chair the human always occupies. Seat `0`, South. */
export const HUMAN_SEAT: Seat = 0;

/** Every seat in clockwise order. */
export const SEATS: readonly Seat[] = Object.freeze([0, 1, 2, 3] as const);

/**
 * The seat one place clockwise — the next player to act in an unimpeded rotation.
 * Loner-unaware; use {@link nextActiveSeat} once a seat may be sitting out.
 */
export const nextSeat = (s: Seat): Seat => ((s + 1) & 3) as Seat;

/** The seat opposite `s`: their partner, and the other member of their team. */
export const partnerOf = (s: Seat): Seat => ((s + 2) & 3) as Seat;

/**
 * The team a seat plays for. Team `0` is `{0, 2}` (the human and North), team `1`
 * is `{1, 3}`. `score` and `tricksWon` are indexed by this, so `score[0]` is
 * always the human's team.
 */
export const teamOf = (s: Seat): Team => (s & 1) as Team;

/** The opposing team. */
export const opponentTeam = (t: Team): Team => ((1 - t) | 0) as Team;

/**
 * A seat's role relative to `dealer`: `0` dealer · `1` eldest · `2` dealer's
 * partner · `3` third seat (the cutter).
 * @example seatRole(1, 0) === 1  // seat 1 is eldest when seat 0 deals
 */
export const seatRole = (s: Seat, dealer: Seat): SeatRole => ((s - dealer) & 3) as SeatRole;

/**
 * Eldest hand: the seat left of the dealer. Bids first in both rounds and leads
 * trick 1 unless they are sitting out under a loner.
 */
export const eldestOf = (dealer: Seat): Seat => nextSeat(dealer);

/** Third seat — right of the dealer. The seat offered the cut. */
export const cutterOf = (dealer: Seat): Seat => ((dealer + 3) & 3) as Seat;

/**
 * The bidding order for one round: eldest, dealer's partner, third seat, dealer.
 * Returns a fresh array; the dealer always acts last.
 */
export function biddingOrder(dealer: Seat): Seat[] {
	const order: Seat[] = [];
	let s = eldestOf(dealer);
	for (let i = 0; i < 4; i++) {
		order.push(s);
		s = nextSeat(s);
	}
	return order;
}

/**
 * `s` if it is active, otherwise the next seat clockwise. Used to resolve the
 * trick-1 lead: when the loner's sitting partner *is* eldest, the opening lead
 * passes clockwise to the next active seat — the dealer's partner, a defender —
 * and never to the loner.
 */
export const firstActiveFrom = (s: Seat, sittingSeat: Seat | null): Seat =>
	s === sittingSeat ? nextSeat(s) : s;

/**
 * The next seat to act after `s`, skipping the loner's sitting partner. At most
 * one seat can ever be sitting out, so one skip is always enough. Never returns
 * `sittingSeat` (V8).
 */
export const nextActiveSeat = (s: Seat, sittingSeat: Seat | null): Seat => {
	const n = nextSeat(s);
	return n === sittingSeat ? nextSeat(n) : n;
};

/**
 * How many seats are in play: `4` normally, `3` under a loner. This is the number
 * of plays that completes a trick.
 */
export const activeSeatCount = (sittingSeat: Seat | null): number => (sittingSeat === null ? 4 : 3);

/** Whether a seat may act at all this hand — `false` only for the sitting partner. */
export const isActiveSeat = (s: Seat, sittingSeat: Seat | null): boolean => s !== sittingSeat;
