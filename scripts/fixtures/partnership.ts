import {
	cardName,
	createGame,
	effectiveSuit,
	getLegalPlays,
	project,
	SEATS,
	type CardId,
	type LegalMoveId,
	type Play,
	type PublicGameView
} from '../../src/lib/euchre/index';

/** Seat-visible situations shared by offline regressions and opt-in live Jev checks. */
export function playView(
	hand: readonly CardId[],
	plays: readonly Play[],
	overrides: Partial<PublicGameView> = {}
): PublicGameView {
	const base = project(createGame({ gameId: 'partnership-check', seed: 'partnership-check' }), 2);
	const trump = overrides.trump ?? 'H';
	const ledSuit = plays[0] ? effectiveSuit(plays[0].card, trump) : null;
	return {
		...base,
		phase: 'trick_play',
		status: 'active',
		turnSeat: 2,
		trump,
		makerSeat: 0,
		hand,
		handCounts: SEATS.map(
			(seat) => hand.length - Number(plays.some((play) => play.seat === seat))
		) as [number, number, number, number],
		trick: { index: 0, ledSuit, plays, winnerSeat: null },
		...overrides,
		legal: getLegalPlays(hand, ledSuit, trump).map((card) => ({
			id: `play:${card}`,
			move: { t: 'play', card },
			label: cardName(card)
		}))
	};
}

export const partnershipCases: readonly {
	name: string;
	view: PublicGameView;
	expected: LegalMoveId;
}[] = [
	{
		name: 'Save the ace when partner has the last-seat winner',
		view: playView(
			['AS', 'TS', '9C', 'KC', 'AD'],
			[
				{ seat: 3, card: '9S' },
				{ seat: 0, card: 'KS' },
				{ seat: 1, card: 'QS' }
			]
		),
		expected: 'play:TS'
	},
	{
		name: 'Discard a loser instead of overtaking partner with a bower',
		view: playView(
			['JH', '9C', 'KC', 'AD', 'QD'],
			[
				{ seat: 3, card: 'KS' },
				{ seat: 0, card: 'AH' },
				{ seat: 1, card: 'AS' }
			]
		),
		expected: 'play:9C'
	},
	{
		name: 'Partner already has our third trick; save the left bower',
		view: playView(
			['KH', 'JD'],
			[
				{ seat: 3, card: '9H' },
				{ seat: 0, card: 'AH' },
				{ seat: 1, card: 'QH' }
			],
			{
				tricksWon: [2, 1],
				handCounts: [1, 1, 2, 1],
				trick: {
					index: 3,
					ledSuit: 'H',
					winnerSeat: null,
					plays: [
						{ seat: 3, card: '9H' },
						{ seat: 0, card: 'AH' },
						{ seat: 1, card: 'QH' }
					]
				},
				trickLog: [
					{
						index: 0,
						ledSuit: 'S',
						winnerSeat: 0,
						plays: [
							{ seat: 0, card: 'AS' },
							{ seat: 1, card: '9S' },
							{ seat: 2, card: 'KS' },
							{ seat: 3, card: 'TS' }
						]
					},
					{
						index: 1,
						ledSuit: 'D',
						winnerSeat: 0,
						plays: [
							{ seat: 0, card: 'AD' },
							{ seat: 1, card: '9D' },
							{ seat: 2, card: 'KD' },
							{ seat: 3, card: 'TD' }
						]
					},
					{
						index: 2,
						ledSuit: 'C',
						winnerSeat: 3,
						plays: [
							{ seat: 0, card: '9C' },
							{ seat: 1, card: 'TC' },
							{ seat: 2, card: 'QC' },
							{ seat: 3, card: 'AC' }
						]
					}
				]
			}
		),
		expected: 'play:KH'
	},
	{
		name: 'Partner has the right bower even though an opponent still plays',
		view: playView(
			['JD', '9H', '9C', 'KC', 'AD'],
			[
				{ seat: 0, card: 'JH' },
				{ seat: 1, card: 'QH' }
			]
		),
		expected: 'play:9H'
	},
	{
		name: 'Protect a vulnerable partner with the unbeatable right bower',
		view: playView(
			['JH', '9H', '9C', 'KC', 'AD'],
			[
				{ seat: 0, card: 'KH' },
				{ seat: 1, card: 'QH' }
			]
		),
		expected: 'play:JH'
	},
	{
		name: 'Use the cheapest winner when the opponent is winning',
		view: playView(
			['AS', 'KS', '9C', 'KC', 'AD'],
			[
				{ seat: 3, card: '9S' },
				{ seat: 0, card: 'TS' },
				{ seat: 1, card: 'QS' }
			]
		),
		expected: 'play:KS'
	},
	{
		name: 'Hand already made; save the left bower to finish the sweep',
		view: playView(
			['9H', 'JD'],
			[
				{ seat: 0, card: 'AH' },
				{ seat: 1, card: 'QH' }
			],
			{
				tricksWon: [3, 0],
				handCounts: [1, 1, 2, 2],
				trick: {
					index: 3,
					ledSuit: 'H',
					winnerSeat: null,
					plays: [
						{ seat: 0, card: 'AH' },
						{ seat: 1, card: 'QH' }
					]
				},
				trickLog: [
					{
						index: 0,
						ledSuit: 'S',
						winnerSeat: 0,
						plays: [
							{ seat: 0, card: 'AS' },
							{ seat: 1, card: '9S' },
							{ seat: 2, card: 'KS' },
							{ seat: 3, card: 'TS' }
						]
					},
					{
						index: 1,
						ledSuit: 'D',
						winnerSeat: 0,
						plays: [
							{ seat: 0, card: 'AD' },
							{ seat: 1, card: '9D' },
							{ seat: 2, card: 'KD' },
							{ seat: 3, card: 'TD' }
						]
					},
					{
						index: 2,
						ledSuit: 'H',
						winnerSeat: 0,
						plays: [
							{ seat: 0, card: 'JH' },
							{ seat: 1, card: 'TH' },
							{ seat: 2, card: 'KH' },
							{ seat: 3, card: 'QC' }
						]
					}
				]
			}
		),
		expected: 'play:9H'
	}
];
