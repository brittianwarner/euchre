/** Cheap, deterministic sampling of possible deals, using ONLY the bidder's view.
 * Jev still makes every real choice. These are approximate outcomes under the
 * existing reference play policy, not solver truth or calibrated win probabilities.
 */
import {
	DECK,
	advance,
	apply,
	createGame,
	project,
	teamOf,
	type Bid,
	type GameState,
	type LegalMove,
	type PublicGameView,
	type Seat
} from '#lib/euchre/index.ts';
import { topMove } from './heuristic';

export const BID_ROLLOUT_SAMPLES = 32;

export function randomFor(text: string) {
	let seed = 2166136261;
	for (const c of text) seed = Math.imul(seed ^ c.charCodeAt(0), 16777619);
	return () => {
		seed |= 0;
		seed = (seed + 0x6d2b79f5) | 0;
		let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

export function bidRollouts(
	view: PublicGameView,
	legal: readonly LegalMove[],
	samples = BID_ROLLOUT_SAMPLES
) {
	const bids = legal.filter(
		(move): move is LegalMove & { move: Exclude<Bid, { t: 'pass' }> } =>
			move.move.t === 'orderUp' || move.move.t === 'call'
	);
	if (
		!bids.length ||
		!view.upCard ||
		view.hand.length !== 5 ||
		!['bid_round_1', 'bid_round_2'].includes(view.phase)
	)
		return {};
	const rng = randomFor(
		JSON.stringify([view.you, view.dealerSeat, view.phase, [...view.hand].sort(), view.upCard])
	);
	const known = new Set([...view.hand, view.upCard]);
	const unseen = DECK.filter((card) => !known.has(card));
	const base = createGame({ gameId: 'bid-estimate', seed: 'possible-deals-only', cfg: view.rules });
	const counts = Object.fromEntries(bids.map((bid) => [bid.id, { made: 0, swept: 0, points: 0 }]));
	for (let sample = 0; sample < samples; sample++) {
		const shuffled = [...unseen];
		for (let i = shuffled.length - 1; i > 0; i--) {
			const j = Math.floor(rng() * (i + 1));
			[shuffled[i], shuffled[j]] = [shuffled[j]!, shuffled[i]!];
		}
		let offset = 0;
		const hands = { ...base.hand.hands, [view.you]: [...view.hand] };
		for (const seat of [0, 1, 2, 3] as Seat[])
			if (seat !== view.you) {
				hands[seat] = shuffled.slice(offset, offset + 5);
				offset += 5;
			}
		const initial: GameState = {
			...base,
			hand: {
				...base.hand,
				phase: view.phase as 'bid_round_1' | 'bid_round_2',
				dealerSeat: view.dealerSeat,
				turnSeat: view.you,
				hands,
				upCard: view.upCard,
				upCardTurnedDown: view.upCardTurnedDown,
				turnedDownSuit: view.turnedDownSuit,
				passes: view.passes,
				bids: view.bids,
				kitty: [view.upCard, shuffled[offset]!, shuffled[offset + 1]!, shuffled[offset + 2]!]
			}
		};
		for (const bid of bids) {
			let state = apply(initial, view.you, bid.move).state;
			for (let step = 0; state.hand.phase !== 'hand_score' && step < 40; step++) {
				const seat = state.hand.turnSeat;
				if (seat === null) {
					state = advance(state).state;
					continue;
				}
				const player = project(state, seat);
				const id = topMove(player, player.hand, player.legal);
				const move = player.legal.find((candidate) => candidate.id === id);
				if (!move) throw new Error('Invalid reference rollout move');
				state = apply(state, seat, move.move).state;
			}
			if (state.hand.phase !== 'hand_score') throw new Error('Reference rollout did not finish');
			const count = counts[bid.id]!;
			const tricks = state.hand.tricksWon[teamOf(view.you)];
			count.made += Number(tricks >= 3);
			count.swept += Number(tricks === 5);
			count.points += tricks < 3 ? -2 : tricks < 5 ? 1 : bid.move.alone ? 4 : 2;
		}
	}
	return Object.fromEntries(
		bids.map((bid) => {
			const count = counts[bid.id]!;
			return [
				bid.id,
				{
					sampledDeals: samples,
					madeHands: count.made,
					sweptHands: count.swept,
					meanNetPoints: Math.round((count.points / samples) * 100) / 100,
					assessment:
						count.made / samples < 0.5
							? 'poor: euchred in most reference deals'
							: count.made / samples < 0.7
								? 'marginal: considerable euchre risk'
								: count.made / samples < 0.85
									? 'supported: made in most reference deals'
									: 'strong: made in almost all reference deals',
					limitation:
						'Uniform possible unseen deals, not actual hidden cards. Reference heuristic play, small sample, ignores uncertain inferences from auction passes. This is an estimate, not a guarantee or Jev confidence.'
				}
			];
		})
	);
}
