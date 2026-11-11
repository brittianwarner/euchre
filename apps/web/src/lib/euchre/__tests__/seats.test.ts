/**
 * `seats.test.ts` — seat arithmetic, teams, and loner-aware turn order.
 *
 * Covers `docs/02-GAME-RULES-ENGINE.md` § 3.5 (loner mechanics 4–6), § 9 cases
 * 27–31, and invariant V8 ("`nextActiveSeat` never returns `sittingSeat`").
 *
 * The § 9 loner cases are the reason `firstActiveFrom` exists at all: case 29 is
 * the one the folk rule gets wrong. The spec note is explicit — when the sitting
 * partner *is* eldest, the opening lead passes **clockwise to the next active
 * seat**, which is a defender, and never to the loner.
 */

import { describe, expect, it } from 'vitest';
import {
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
} from '../seats';
import type { Seat, Team } from '../types';

const ALL: readonly Seat[] = [0, 1, 2, 3];
/** Every `sittingSeat` value the engine can present, including "no loner". */
const SITTINGS: readonly (Seat | null)[] = [null, 0, 1, 2, 3];

/* -------------------------------------------------------------------------- */
/* Constants                                                                   */
/* -------------------------------------------------------------------------- */

describe('seat constants', () => {
	it('the human always sits at seat 0 (South)', () => {
		expect(HUMAN_SEAT).toBe(0);
	});

	it('SEATS is 0 1 2 3 clockwise and is frozen', () => {
		expect(SEATS).toEqual([0, 1, 2, 3]);
		expect(Object.isFrozen(SEATS)).toBe(true);
	});
});

/* -------------------------------------------------------------------------- */
/* nextSeat / partnerOf                                                        */
/* -------------------------------------------------------------------------- */

describe('nextSeat', () => {
	it('goes 0→1→2→3→0', () => {
		expect([nextSeat(0), nextSeat(1), nextSeat(2), nextSeat(3)]).toEqual([1, 2, 3, 0]);
	});

	it('returns to the start after exactly four steps, from every seat', () => {
		for (const s of ALL) {
			let cur = s;
			const visited: Seat[] = [];
			for (let i = 0; i < 4; i++) {
				cur = nextSeat(cur);
				visited.push(cur);
			}
			expect(cur).toBe(s);
			expect([...visited].sort()).toEqual([0, 1, 2, 3]);
		}
	});
});

describe('partnerOf', () => {
	it('pairs 0↔2 and 1↔3', () => {
		expect([partnerOf(0), partnerOf(1), partnerOf(2), partnerOf(3)]).toEqual([2, 3, 0, 1]);
	});

	it('is an involution and never self-partners', () => {
		for (const s of ALL) {
			expect(partnerOf(partnerOf(s))).toBe(s);
			expect(partnerOf(s)).not.toBe(s);
		}
	});

	it('a seat and its partner are always the same team', () => {
		for (const s of ALL) expect(teamOf(partnerOf(s))).toBe(teamOf(s));
	});
});

/* -------------------------------------------------------------------------- */
/* Teams                                                                       */
/* -------------------------------------------------------------------------- */

describe('teamOf / opponentTeam', () => {
	it('puts {0,2} on team 0 — always the human’s team — and {1,3} on team 1', () => {
		expect([teamOf(0), teamOf(1), teamOf(2), teamOf(3)]).toEqual([0, 1, 0, 1]);
		expect(teamOf(HUMAN_SEAT)).toBe(0);
	});

	it('opponentTeam flips 0↔1 and is an involution', () => {
		expect(opponentTeam(0)).toBe(1);
		expect(opponentTeam(1)).toBe(0);
		for (const t of [0, 1] as Team[]) expect(opponentTeam(opponentTeam(t))).toBe(t);
	});

	it('adjacent seats are always opponents', () => {
		for (const s of ALL) expect(teamOf(nextSeat(s))).toBe(opponentTeam(teamOf(s)));
	});
});

/* -------------------------------------------------------------------------- */
/* Roles                                                                       */
/* -------------------------------------------------------------------------- */

describe('seatRole', () => {
	it('is 0 dealer · 1 eldest · 2 dealer’s partner · 3 third seat, for every dealer', () => {
		for (const dealer of ALL) {
			expect([dealer, seatRole(dealer, dealer)]).toEqual([dealer, 0]);
			expect([dealer, seatRole(eldestOf(dealer), dealer)]).toEqual([dealer, 1]);
			expect([dealer, seatRole(partnerOf(dealer), dealer)]).toEqual([dealer, 2]);
			expect([dealer, seatRole(cutterOf(dealer), dealer)]).toEqual([dealer, 3]);
		}
	});

	it('matches the spec example: seat 1 is eldest when seat 0 deals', () => {
		expect(seatRole(1, 0)).toBe(1);
	});

	it('is a bijection from seats onto roles, for every dealer', () => {
		for (const dealer of ALL) {
			const roles = ALL.map((s) => seatRole(s, dealer));
			expect([...roles].sort()).toEqual([0, 1, 2, 3]);
		}
	});
});

describe('eldestOf / cutterOf', () => {
	it('eldest is left of the dealer', () => {
		expect([eldestOf(0), eldestOf(1), eldestOf(2), eldestOf(3)]).toEqual([1, 2, 3, 0]);
	});

	it('the cutter is right of the dealer', () => {
		expect([cutterOf(0), cutterOf(1), cutterOf(2), cutterOf(3)]).toEqual([3, 0, 1, 2]);
	});

	it('the cutter is an opponent of the dealer; eldest is too', () => {
		for (const d of ALL) {
			expect(teamOf(cutterOf(d))).toBe(opponentTeam(teamOf(d)));
			expect(teamOf(eldestOf(d))).toBe(opponentTeam(teamOf(d)));
		}
	});
});

describe('biddingOrder', () => {
	it('is eldest → dealer’s partner → third seat → dealer, for every dealer', () => {
		expect(biddingOrder(0)).toEqual([1, 2, 3, 0]);
		expect(biddingOrder(1)).toEqual([2, 3, 0, 1]);
		expect(biddingOrder(2)).toEqual([3, 0, 1, 2]);
		expect(biddingOrder(3)).toEqual([0, 1, 2, 3]);
	});

	it('always seats the dealer last and eldest first', () => {
		for (const d of ALL) {
			const order = biddingOrder(d);
			expect(order).toHaveLength(4);
			expect(order[0]).toBe(eldestOf(d));
			expect(order[3]).toBe(d);
			expect([...order].sort()).toEqual([0, 1, 2, 3]);
		}
	});

	it('returns a fresh array each call', () => {
		const a = biddingOrder(0);
		const b = biddingOrder(0);
		expect(a).not.toBe(b);
		a[0] = 3;
		expect(biddingOrder(0)[0]).toBe(1);
	});
});

/* -------------------------------------------------------------------------- */
/* firstActiveFrom — § 9 cases 27–30, the trick-1 lead under a loner           */
/* -------------------------------------------------------------------------- */

describe('firstActiveFrom — trick-1 lead under a loner', () => {
	/** `sittingSeat` is always `partnerOf(aloneSeat)` (V4). */
	const lead = (dealer: Seat, alone: Seat): Seat =>
		firstActiveFrom(eldestOf(dealer), partnerOf(alone));

	// § 9 case 27
	it('loner is eldest (dealer=0, alone=1, sitting=3) → seat 1 leads', () => {
		expect(partnerOf(1)).toBe(3);
		expect(lead(0, 1)).toBe(1);
	});

	// § 9 case 28
	it('loner is dealer’s partner (dealer=0, alone=2, sitting=0) → seat 1 leads', () => {
		expect(partnerOf(2)).toBe(0);
		expect(lead(0, 2)).toBe(1);
	});

	// § 9 case 29 — the case the folk rule gets wrong.
	it('loner is third seat (dealer=0, alone=3, sitting=1) → seat 2 leads, a DEFENDER', () => {
		expect(partnerOf(3)).toBe(1);
		const l = lead(0, 3);
		expect(l).toBe(2);
		// It is the dealer's partner…
		expect(l).toBe(partnerOf(0));
		// …it is not the loner (the folk rule's answer)…
		expect(l).not.toBe(3);
		// …and it is on the defending team.
		expect(teamOf(l)).toBe(opponentTeam(teamOf(3)));
	});

	// § 9 case 30
	it('loner is the dealer (dealer=0, alone=0, sitting=2) → seat 1 leads', () => {
		expect(partnerOf(0)).toBe(2);
		expect(lead(0, 0)).toBe(1);
	});

	it('never returns the sitting seat, for every (from, sitting) pair', () => {
		for (const s of ALL) {
			for (const sitting of SITTINGS) {
				expect([s, sitting, firstActiveFrom(s, sitting)]).not.toEqual([s, sitting, sitting]);
			}
		}
	});

	it('is the identity whenever the seat is active', () => {
		for (const s of ALL) {
			expect(firstActiveFrom(s, null)).toBe(s);
			for (const sitting of ALL) {
				if (sitting === s) continue;
				expect([s, sitting, firstActiveFrom(s, sitting)]).toEqual([s, sitting, s]);
			}
		}
	});

	it('skips exactly one seat clockwise when the seat is sitting out', () => {
		for (const s of ALL) expect(firstActiveFrom(s, s)).toBe(nextSeat(s));
	});

	it('holds for every dealer, not just dealer 0: the lead is never the sitting seat', () => {
		for (const dealer of ALL) {
			for (const alone of ALL) {
				const sitting = partnerOf(alone);
				const l = lead(dealer, alone);
				expect([dealer, alone, l === sitting]).toEqual([dealer, alone, false]);
				// Eldest leads unless eldest is the one sitting out.
				const want = eldestOf(dealer) === sitting ? nextSeat(eldestOf(dealer)) : eldestOf(dealer);
				expect([dealer, alone, l]).toEqual([dealer, alone, want]);
			}
		}
	});
});

/* -------------------------------------------------------------------------- */
/* nextActiveSeat — V8, and § 9 case 31                                        */
/* -------------------------------------------------------------------------- */

describe('nextActiveSeat', () => {
	it('is plain nextSeat when nobody is sitting out', () => {
		for (const s of ALL) expect(nextActiveSeat(s, null)).toBe(nextSeat(s));
	});

	it('never returns the sitting seat, for every (from, sitting) pair (V8)', () => {
		for (const s of ALL) {
			for (const sitting of SITTINGS) {
				const got = nextActiveSeat(s, sitting);
				expect([s, sitting, got === sitting]).toEqual([s, sitting, false]);
			}
		}
	});

	it('skips exactly one seat when the next seat is sitting out', () => {
		// dealer 0, loner 1, sitting 3: after seat 2 comes seat 0, not seat 3.
		expect(nextActiveSeat(2, 3)).toBe(0);
		expect(nextActiveSeat(0, 1)).toBe(2);
		expect(nextActiveSeat(1, 2)).toBe(3);
		expect(nextActiveSeat(3, 0)).toBe(1);
	});

	it('never skips when the next seat is active', () => {
		for (const s of ALL) {
			for (const sitting of ALL) {
				if (nextSeat(s) === sitting) continue;
				expect([s, sitting, nextActiveSeat(s, sitting)]).toEqual([s, sitting, nextSeat(s)]);
			}
		}
	});

	it('is total even when called from the sitting seat itself', () => {
		for (const s of ALL) expect(nextActiveSeat(s, s)).toBe(nextSeat(s));
	});

	// § 9 case 31: three plays complete a trick, and the rotation is a 3-cycle.
	it('cycles through exactly the 3 active seats under a loner and returns to the lead', () => {
		for (const sitting of ALL) {
			const lead = nextSeat(sitting); // any active seat
			let cur = lead;
			const visited: Seat[] = [lead];
			for (let i = 0; i < 2; i++) {
				cur = nextActiveSeat(cur, sitting);
				visited.push(cur);
			}
			expect(visited).toHaveLength(3);
			expect(new Set(visited).size).toBe(3);
			expect(visited).not.toContain(sitting);
			// One more step closes the cycle.
			expect(nextActiveSeat(cur, sitting)).toBe(lead);
		}
	});

	it('cycles through all 4 seats with no loner', () => {
		let cur: Seat = 0;
		const visited: Seat[] = [0];
		for (let i = 0; i < 3; i++) {
			cur = nextActiveSeat(cur, null);
			visited.push(cur);
		}
		expect(visited).toEqual([0, 1, 2, 3]);
		expect(nextActiveSeat(cur, null)).toBe(0);
	});
});

/* -------------------------------------------------------------------------- */
/* activeSeatCount / isActiveSeat                                              */
/* -------------------------------------------------------------------------- */

describe('activeSeatCount', () => {
	it('is 4 with no loner and 3 with one', () => {
		expect(activeSeatCount(null)).toBe(4);
		for (const s of ALL) expect(activeSeatCount(s)).toBe(3);
	});

	it('equals the number of seats isActiveSeat admits', () => {
		for (const sitting of SITTINGS) {
			const n = ALL.filter((s) => isActiveSeat(s, sitting)).length;
			expect([sitting, n]).toEqual([sitting, activeSeatCount(sitting)]);
		}
	});

	// § 9 case 31 / V10: 20 cards played normally, 15 under one loner.
	it('implies 20 cards played per hand normally and 15 under a loner (V10)', () => {
		expect(activeSeatCount(null) * 5).toBe(20);
		expect(activeSeatCount(2) * 5).toBe(15);
	});
});

describe('isActiveSeat', () => {
	it('is false only for the sitting seat', () => {
		for (const s of ALL) {
			expect(isActiveSeat(s, null)).toBe(true);
			for (const sitting of ALL) {
				expect([s, sitting, isActiveSeat(s, sitting)]).toEqual([s, sitting, s !== sitting]);
			}
		}
	});
});
