import { describe, expect, it } from 'vitest';
import { advance, apply, createGame, legalMoves, type GameState } from '../index';

function bidding(): GameState {
	let state = advance(
		createGame({
			gameId: 'rules',
			seed: 'rules',
			firstDealer: 0,
			cfg: { requireNaturalTrump: true }
		})
	).state;
	state = apply(state, state.hand.turnSeat!, { t: 'cut', cut: false }).state;
	state = advance(state).state;
	return {
		...state,
		hand: {
			...state.hand,
			upCard: '9H',
			hands: [
				state.hand.hands[0],
				['JD', '9C', 'TC', 'QC', 'AC'],
				state.hand.hands[2],
				state.hand.hands[3]
			]
		}
	};
}
describe('natural trump house rule', () => {
	it('does not let the left bower alone order up trump', () => {
		const s = bidding();
		expect(legalMoves(s, 1).map((m) => m.id)).toEqual(['pass']);
	});
	it('does not let the left bower alone name that suit in round two', () => {
		const s = bidding();
		const next = {
			...s,
			hand: { ...s.hand, phase: 'bid_round_2' as const, turnedDownSuit: 'S' as const }
		};
		expect(legalMoves(next, 1).map((m) => m.id)).not.toContain('call:H');
		expect(legalMoves(next, 1).map((m) => m.id)).toContain('call:C');
	});
	it('allows the dealer to count the natural trump up-card', () => {
		const s = bidding();
		const next = { ...s, hand: { ...s.hand, turnSeat: 0 as const } };
		expect(legalMoves(next, 0).map((m) => m.id)).toContain('orderUp');
	});
	it('lets a stuck dealer choose rather than creating a dead end', () => {
		const s = bidding();
		const next = {
			...s,
			hand: {
				...s.hand,
				phase: 'bid_round_2' as const,
				turnSeat: 0 as const,
				passes: 3,
				turnedDownSuit: 'S' as const
			}
		};
		expect(legalMoves(next, 0).map((m) => m.id)).toEqual([
			'call:H',
			'call:H+alone',
			'call:D',
			'call:D+alone',
			'call:C',
			'call:C+alone'
		]);
	});
	it.each([true, false])('finishes matches with natural trump and stick=%s', (stickTheDealer) => {
		for (let seed = 0; seed < 20; seed++) {
			let s = createGame({
				gameId: `house-${seed}`,
				seed: `house-${seed}`,
				cfg: { stickTheDealer, requireNaturalTrump: true }
			});
			for (let step = 0; step < 3000 && s.status === 'active'; step++) {
				const seat = s.hand.turnSeat;
				if (seat === null) s = advance(s).state;
				else {
					const moves = legalMoves(s, seat);
					expect(moves.length).toBeGreaterThan(0);
					s = apply(
						s,
						seat,
						(moves.find((m) => m.move.t === 'call' || m.move.t === 'orderUp') ??
							moves[(seed + step) % moves.length])!.move
					).state;
				}
			}
			expect(s.status).toBe('complete');
		}
	});
});
