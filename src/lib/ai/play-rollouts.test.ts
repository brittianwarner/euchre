import { describe, expect, it } from 'vitest';
import {
	advance,
	apply,
	assertInvariants,
	createGame,
	effectiveSuit,
	project,
	type PublicGameView,
	type Seat
} from '../euchre';
import { topMove, voidReads } from './heuristic';
import { playRollouts, possibleRemainingDeals } from './play-rollouts';

function positions(): PublicGameView[] {
	const views: PublicGameView[] = [];
	for (let seed = 0; seed < 12; seed++) {
		let state = createGame({
			gameId: 'eval',
			seed: `rollout-constraint-${seed}`,
			firstDealer: (seed % 4) as Seat
		});
		for (let step = 0; step < 150 && state.hand.handNo < 2; step++) {
			const seat = state.hand.turnSeat;
			if (seat === null) {
				state = advance(state).state;
				continue;
			}
			const view = project(state, seat);
			if (view.phase === 'trick_play' && view.legal.length > 1) views.push(view);
			const move = view.legal.find(
				(candidate) =>
					candidate.id ===
					(view.phase === 'cutting' ? 'cut:no' : topMove(view, view.hand, view.legal))
			)!;
			state = apply(state, seat, move.move).state;
		}
	}
	return views;
}
const views = positions();

describe('public-evidence card continuations', () => {
	it('accounts for every card and respects proved voids and dealer pickup across real positions', () => {
		expect(views.length).toBeGreaterThan(80);
		let sampled = 0;
		for (const view of views.filter((_, i) => i % 3 === 0)) {
			const voids = voidReads(view);
			for (const deal of possibleRemainingDeals(view, 4)) {
				sampled++;
				assertInvariants(deal);
				expect(deal.hand.hands[view.you]).toEqual(view.hand);
				for (const seat of [0, 1, 2, 3] as Seat[]) {
					if (seat === view.sittingSeat) continue;
					expect(
						deal.hand.hands[seat].some((card) =>
							voids[seat].includes(effectiveSuit(card, view.trump))
						)
					).toBe(false);
				}
				if (!view.upCardTurnedDown && view.upCard) {
					const owner = ([0, 1, 2, 3] as Seat[]).find((seat) =>
						deal.hand.hands[seat].includes(view.upCard!)
					);
					expect(owner === undefined || owner === view.dealerSeat).toBe(true);
				}
			}
		}
		expect(sampled).toBeGreaterThan(80);
	});
	it('is independent of option order and cannot read private cards or seeds', () => {
		const view = views[0]!;
		const expected = playRollouts(view, view.legal, 6);
		expect(Object.keys(expected)).toHaveLength(view.legal.length);
		expect(playRollouts(view, [...view.legal].reverse(), 6)).toEqual(expected);
		expect(
			playRollouts(
				{ ...view, seed: 'secret', hands: { 0: ['JH'] }, dealerDiscard: 'AH' } as PublicGameView,
				view.legal,
				6
			)
		).toEqual(expected);
	});
	it('does not invent reference evidence for incomplete fixtures or forced moves', () => {
		const view = views[0]!;
		expect(possibleRemainingDeals({ ...view, handCounts: [0, 0, 0, 0] })).toEqual([]);
		expect(playRollouts(view, view.legal.slice(0, 1))).toEqual({});
	});
	it('buries the accepted up-card when the dealer sits out on partner’s loner', () => {
		let state = createGame({ gameId: 'sitting-dealer', seed: 'sitting-dealer', firstDealer: 2 });
		state = advance(state).state;
		state = apply(state, 1, { t: 'cut', cut: false }).state;
		state = advance(state).state;
		state = apply(state, 3, { t: 'pass' }).state;
		state = apply(state, 0, { t: 'orderUp', alone: true }).state;
		const view = project(state, 3);
		expect(view).toMatchObject({ phase: 'trick_play', sittingSeat: 2, upCardTurnedDown: false });
		const deals = possibleRemainingDeals(view, 8);
		expect(deals).toHaveLength(8);
		for (const deal of deals) {
			assertInvariants(deal);
			expect(deal.hand.dealerDiscard).toBe(view.upCard);
			expect(deal.hand.hands[2]).not.toContain(view.upCard);
		}
	});
	it('gives bounded estimates and never changes the view', () => {
		const view = views.find((view) => view.trick.index === 2)!;
		const before = JSON.stringify(view);
		const result = playRollouts(view, view.legal, 8);
		expect(Object.values(result).length).toBeGreaterThan(1);
		for (const estimate of Object.values(result)) {
			expect(estimate.sampledDeals).toBeGreaterThan(0);
			expect(estimate.sampledDeals).toBeLessThanOrEqual(8);
			expect(estimate.meanTeamNetPoints).toBeGreaterThanOrEqual(-4);
			expect(estimate.meanTeamNetPoints).toBeLessThanOrEqual(4);
			expect(estimate.meanTeamTricks).toBeGreaterThanOrEqual(0);
			expect(estimate.meanTeamTricks).toBeLessThanOrEqual(5);
		}
		expect(JSON.stringify(view)).toBe(before);
	});
});
