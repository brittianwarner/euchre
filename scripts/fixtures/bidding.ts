import {
	createGame,
	legalMoves,
	project,
	suitOf,
	type CardId,
	type PublicGameView
} from '../../src/lib/euchre/index';

/** Synthetic seat-visible auctions. Legal options come from the actual rule engine. */
export function bidView(
	hand: readonly CardId[],
	upCard: CardId,
	overrides: Partial<PublicGameView> = {}
): PublicGameView {
	const you = overrides.you ?? 1;
	const base = createGame({ gameId: 'bidding-check', seed: 'bidding-check' });
	const round2 = overrides.phase === 'bid_round_2';
	const state = {
		...base,
		cfg: { ...base.cfg, ...overrides.rules },
		status: 'active' as const,
		hand: {
			...base.hand,
			phase: round2 ? ('bid_round_2' as const) : ('bid_round_1' as const),
			dealerSeat: overrides.dealerSeat ?? 0,
			turnSeat: you,
			upCard,
			upCardTurnedDown: round2,
			turnedDownSuit: round2 ? suitOf(upCard) : null,
			passes: overrides.passes ?? 0,
			hands: { ...base.hand.hands, [you]: [...hand] }
		}
	};
	return {
		...project(state, you),
		handCounts: [5, 5, 5, 5],
		...overrides,
		legal: legalMoves(state, you)
	};
}

export const biddingCases: readonly {
	name: string;
	view: PublicGameView;
	expected: readonly string[];
}[] = [
	{
		name: 'Reported Rita hand 2: no hearts to assist a partner with a ten',
		view: bidView(['JC', '9C', 'TS', 'AS', 'TD'], 'TH', {
			dealerSeat: 3,
			score: [0, 1],
			passes: 1,
			bids: [{ seat: 0, move: { t: 'pass' }, say: 'Pass.' }]
		}),
		expected: ['pass']
	},
	{
		name: 'Reported Rita hand 3: lone ace of trump, opponent picks up queen',
		view: bidView(['TH', '9C', 'KC', 'JD', 'AS'], 'QS', { score: [2, 1] }),
		expected: ['pass']
	},
	{
		name: 'Both bowers, ace and king of trump: take the loner',
		view: bidView(['JH', 'JD', 'AH', 'KH', 'AC'], '9H'),
		expected: ['orderUp+alone']
	},
	{
		name: 'Three trump with the right and an outside ace: sound team call',
		view: bidView(['JH', 'KH', '9H', 'AS', 'TC'], 'QH'),
		expected: ['orderUp']
	},
	{
		name: 'Dealer gains right bower: evaluate pickup, not just current hand',
		view: bidView(['JD', 'AH', 'KH', 'AS', '9C'], 'JH', { dealerSeat: 1 }),
		expected: ['orderUp+alone']
	},
	{
		name: 'Zero printed hearts but left bower and three outside aces',
		view: bidView(['JD', 'AS', 'AC', 'AD', '9C'], 'JH', { dealerSeat: 3 }),
		expected: ['orderUp']
	},
	{
		name: 'Round two: do not blindly call next with no trump or aces',
		view: bidView(['9S', 'TS', '9C', 'TC', 'QH'], 'AD', {
			phase: 'bid_round_2',
			dealerSeat: 0,
			passes: 0
		}),
		expected: ['pass']
	},
	{
		name: 'Round two: four clubs, both bowers, outside ace',
		view: bidView(['JC', 'JS', 'AC', '9C', 'AH'], 'TD', { phase: 'bid_round_2' }),
		expected: ['call:C+alone']
	},
	{
		name: 'Stuck dealer: best available trump, pass is unavailable',
		view: bidView(['JH', 'AH', '9H', '9C', 'TS'], 'QD', {
			phase: 'bid_round_2',
			dealerSeat: 1,
			passes: 3,
			rules: { stickTheDealer: true, requireNaturalTrump: true }
		}),
		expected: ['call:H']
	},
	{
		name: 'At nine: keep partner for the winning point',
		view: bidView(['JH', 'JD', 'AH', 'KH', 'AC'], '9H', { score: [3, 9] }),
		expected: ['orderUp']
	}
];
