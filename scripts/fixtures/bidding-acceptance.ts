/** Separate acceptance positions, added after the reported-hand prompt development. */
import { bidView } from './bidding';
import type { PublicGameView } from '../../src/lib/euchre/index';

export const acceptanceBiddingCases: readonly {
	name: string;
	view: PublicGameView;
	expected: readonly string[];
}[] = [
	{
		name: 'No heart support, ace goes to opponent dealer',
		view: bidView(['JS', 'QS', '9S', '9D', 'TC'], 'AH'),
		expected: ['pass']
	},
	{
		name: 'Left bower only, no outside ace, low up-card to partner',
		view: bidView(['JD', '9C', 'TC', 'KS', '9S'], 'TH', { dealerSeat: 3 }),
		expected: ['pass']
	},
	{
		name: 'Two low trump and no aces, give opponent the right',
		view: bidView(['9H', 'TH', '9S', 'QC', 'KD'], 'JH'),
		expected: ['pass']
	},
	{
		name: 'Right, ace, small trump and outside ace',
		view: bidView(['JH', 'AH', '9H', 'AC', 'TC'], 'QH'),
		expected: ['orderUp']
	},
	{
		name: 'Assist with left, ace, king; exposed outside losers',
		view: bidView(['JD', 'AH', 'KH', 'QD', 'TS'], '9H', { dealerSeat: 3 }),
		expected: ['orderUp']
	},
	{
		name: 'Both black bowers and small trump, outside loser needs partner',
		view: bidView(['JS', 'JC', '9S', 'AD', '9C'], 'AS'),
		expected: ['orderUp']
	},
	{
		name: 'Dealer pickup makes three high trump but outside loser remains',
		view: bidView(['AH', 'KH', 'AS', '9S', '9D'], 'JH', { dealerSeat: 1 }),
		expected: ['orderUp']
	},
	{
		name: 'Dealer pickup makes right and small trump plus two aces',
		view: bidView(['9H', 'AS', 'AC', 'TC', '9D'], 'JH', { dealerSeat: 1 }),
		expected: ['orderUp']
	},
	{
		name: 'Three plain suits, nothing worth a second-round voluntary call',
		view: bidView(['9H', 'QH', 'TC', '9C', 'TS'], 'AD', { phase: 'bid_round_2' }),
		expected: ['pass']
	},
	{
		name: 'Second round right, ace, king, outside ace; keep partner for loser',
		view: bidView(['JC', 'AC', 'KC', 'AS', '9D'], 'TH', { phase: 'bid_round_2' }),
		expected: ['call:C']
	},
	{
		name: 'Second round five trump, both bowers',
		view: bidView(['JD', 'JH', 'AD', 'KD', '9D'], 'TS', { phase: 'bid_round_2' }),
		expected: ['call:D+alone']
	},
	{
		name: 'Dealer before pickup looks weak, afterwards has five dominant trump',
		view: bidView(['JD', 'AH', 'KH', '9H', '9C'], 'JH', { dealerSeat: 1 }),
		expected: ['orderUp+alone']
	},
	{
		name: 'At nine, dominant dealer pickup still keeps partner',
		view: bidView(['JD', 'AH', 'KH', 'AS', '9C'], 'JH', { dealerSeat: 1, score: [4, 9] }),
		expected: ['orderUp']
	},
	{
		name: 'Opponent at nine is no reason to donate a hopeless hand',
		view: bidView(['JC', '9C', 'TS', 'KS', 'TD'], 'TH', { dealerSeat: 3, score: [9, 4] }),
		expected: ['pass']
	},
	{
		name: 'Forced dealer has four high clubs but an exposed outside nine: keep partner',
		view: bidView(['JC', 'JS', 'AC', 'KC', '9D'], 'TH', {
			phase: 'bid_round_2',
			dealerSeat: 1,
			passes: 3,
			rules: { stickTheDealer: true, requireNaturalTrump: true }
		}),
		expected: ['call:C']
	}
];
