import { playView } from './partnership';
import { cardName, type CardId, type LegalMoveId, type PublicGameView } from '../../src/lib/euchre';

function lead(hand: readonly CardId[], defender = false): PublicGameView {
	return playView(hand, [], {
		you: 2,
		turnSeat: 2,
		dealerSeat: 1,
		makerSeat: defender ? 3 : 2,
		upCard: '9D',
		upCardTurnedDown: true,
		turnedDownSuit: 'D'
	});
}
function discard(hand: readonly CardId[], trump: 'H' | 'D'): PublicGameView {
	return {
		...lead(hand),
		phase: 'dealer_discard',
		dealerSeat: 2,
		trump,
		upCard: trump === 'H' ? 'JH' : 'JD',
		upCardTurnedDown: false,
		turnedDownSuit: null,
		legal: hand.map((card) => ({
			id: `discard:${card}`,
			move: { t: 'discard' as const, card },
			label: cardName(card)
		}))
	};
}
/** Allowed sets reflect strategic equivalence, not a claim of a unique optimum.
 * In particular all three highest trump win the same first trick when held
 * together; the player can draw them in any order. Weak plain losers are excluded. */
export const strategyCases: readonly {
	name: string;
	view: PublicGameView;
	expected: readonly LegalMoveId[];
}[] = [
	{
		name: 'Maker chooses an outside winner or plausible trump lead, not a plain loser',
		view: lead(['9H', 'TH', 'AS', '9C', 'TC']),
		expected: ['play:AS', 'play:9H', 'play:TH']
	},
	{
		name: 'Maker draws trump with strong bower control',
		view: lead(['JH', 'JD', 'AH', 'AS', '9C']),
		expected: ['play:JH', 'play:JD', 'play:AH']
	},
	{
		name: 'Defender with both bowers leads a winner or draws with control',
		view: lead(['JH', 'JD', 'AS', '9C', 'TC'], true),
		expected: ['play:AS', 'play:JH', 'play:JD']
	},
	{
		name: 'Outside ace leads before lower cards of its suit',
		view: lead(['AS', 'KS', 'TS', '9C', 'TC']),
		expected: ['play:AS']
	},
	{
		name: 'Dealer keeps both bowers and aces and creates a void',
		view: discard(['JH', 'JD', 'AH', 'KH', 'AS', '9C'], 'H'),
		expected: ['discard:9C']
	},
	{
		name: 'Dealer keeps the left bower in its effective trump suit',
		view: discard(['JD', 'JH', 'AD', 'AS', 'AC', '9H'], 'D'),
		expected: ['discard:9H']
	}
];
