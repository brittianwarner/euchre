import { describe, expect, it } from 'vitest';
import { bidView, biddingCases } from '../../../scripts/fixtures/bidding';
import { bidRollouts } from './bid-rollouts';

describe('seat-visible bidding rollouts', () => {
	it('is reproducible and independent of option order', () => {
		const view = biddingCases[1].view;
		expect(bidRollouts(view, view.legal)).toEqual(bidRollouts(view, [...view.legal].reverse()));
	});
	it('does not depend on unknown deck order, private seed or another hand', () => {
		const view = biddingCases[0].view;
		const poisoned = {
			...view,
			seed: 'completely different',
			deckOrder: ['AH'],
			hands: { 0: ['JH'] },
			kitty: ['JD'],
			internalToken: 'never-read'
		};
		expect(bidRollouts(poisoned, view.legal)).toEqual(bidRollouts(view, view.legal));
	});
	it('rejects the reported poor bids but recognizes a strong team call', () => {
		for (const scenario of biddingCases.slice(0, 2)) {
			const result = bidRollouts(scenario.view, scenario.view.legal).orderUp!;
			expect(result.meanNetPoints).toBeLessThan(0);
			expect(result.assessment).toContain('poor');
		}
		const good = biddingCases[3];
		expect(bidRollouts(good.view, good.view.legal).orderUp!.meanNetPoints).toBeGreaterThan(0);
	});
	it('handles partner alone sitting out, left bowers, and natural-trump restrictions', () => {
		const view = biddingCases[5].view;
		const result = bidRollouts(view, view.legal);
		expect(result.orderUp!.madeHands).toBeGreaterThan(result['orderUp+alone']!.madeHands);
		const restricted = bidView(view.hand, 'JH', {
			dealerSeat: 3,
			rules: { stickTheDealer: true, requireNaturalTrump: true }
		});
		expect(bidRollouts(restricted, restricted.legal)).toEqual({});
	});
	it('keeps forced dealer options available and the turned-down suit out', () => {
		const view = biddingCases[8].view;
		const result = bidRollouts(view, view.legal);
		expect(result['call:H']).toBeDefined();
		expect(result['call:D']).toBeUndefined();
		expect(result.pass).toBeUndefined();
	});
});
