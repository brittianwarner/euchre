/** Possible remaining deals constrained by public cards/counts/voids. These are
 * hypothetical reference continuations; Jev still chooses every actual card.
 */
import {
	DECK,
	advance,
	apply,
	createGame,
	effectiveSuit,
	project,
	teamOf,
	type CardId,
	type GameState,
	type LegalMove,
	type PublicGameView,
	type Seat
} from '#lib/euchre/index.ts';
import { topMove, voidReads } from './heuristic';
import { randomFor } from './bid-rollouts';

export const PLAY_ROLLOUT_SAMPLES = 24;

export function possibleRemainingDeals(
	view: PublicGameView,
	targetSamples = PLAY_ROLLOUT_SAMPLES
): GameState[] {
	if (view.phase !== 'trick_play' || !view.trump || !view.upCard || view.makerSeat === null)
		return [];
	const tricks = [...view.trickLog, view.trick];
	const played = tricks.flatMap((trick) => trick.plays.map((play) => play.card));
	// Incomplete synthetic fixtures do not describe possible remaining deals.
	if (
		view.trick.index !== view.trickLog.length ||
		view.handCounts[view.you] !== view.hand.length ||
		[0, 1, 2, 3].some(
			(seat) =>
				seat !== view.sittingSeat &&
				view.handCounts[seat] !==
					5 - tricks.flatMap((trick) => trick.plays).filter((play) => play.seat === seat).length
		)
	)
		return [];
	const known = new Set<CardId>([...view.hand, ...played]);
	if (known.size !== view.hand.length + played.length) return [];
	if (view.upCardTurnedDown) known.add(view.upCard);
	const unknown = DECK.filter((card) => !known.has(card));
	const voids = voidReads(view);
	const rng = randomFor(
		JSON.stringify([
			view.you,
			[...view.hand].sort(),
			view.trump,
			view.trick,
			view.trickLog,
			view.handCounts
		])
	);
	const base = createGame({
		gameId: 'play-estimate',
		seed: 'possible-deals-only',
		cfg: view.rules
	});
	const deals: GameState[] = [];
	for (let attempt = 0; deals.length < targetSamples && attempt < 512; attempt++) {
		const shuffled = [...unknown];
		for (let i = shuffled.length - 1; i > 0; i--) {
			const j = Math.floor(rng() * (i + 1));
			[shuffled[i], shuffled[j]] = [shuffled[j]!, shuffled[i]!];
		}
		const hands = { ...base.hand.hands, [view.you]: [...view.hand] };
		let offset = 0;
		let valid = true;
		for (const seat of [0, 1, 2, 3] as Seat[]) {
			if (seat === view.you) continue;
			const count = seat === view.sittingSeat ? 5 : view.handCounts[seat];
			hands[seat] = shuffled.slice(offset, offset + count);
			offset += count;
			if (
				seat !== view.sittingSeat &&
				hands[seat].some((card) => voids[seat].includes(effectiveSuit(card, view.trump)))
			)
				valid = false;
			// An accepted up-card can only remain with the dealer, or be discarded.
			if (
				!view.upCardTurnedDown &&
				(seat !== view.dealerSeat || seat === view.sittingSeat) &&
				hands[seat].includes(view.upCard)
			)
				valid = false;
		}
		if (!valid) continue;
		const buried = shuffled.slice(offset);
		const dealerDiscard = view.upCardTurnedDown
			? null
			: buried.includes(view.upCard)
				? view.upCard
				: buried[0]!;
		const kittyRest = buried.filter((card) => card !== view.upCard && card !== dealerDiscard);
		if (kittyRest.length !== 3) continue;
		deals.push({
			...base,
			hand: {
				...base.hand,
				phase: 'trick_play',
				deckOrder: DECK,
				cut: false,
				dealerSeat: view.dealerSeat,
				turnSeat: view.you,
				trump: view.trump,
				makerSeat: view.makerSeat,
				aloneSeat: view.aloneSeat,
				sittingSeat: view.sittingSeat,
				hands,
				upCard: view.upCard,
				upCardTurnedDown: view.upCardTurnedDown,
				turnedDownSuit: view.turnedDownSuit,
				dealerDiscard,
				trick: view.trick,
				trickLog: view.trickLog,
				tricksWon: view.tricksWon,
				kitty: [view.upCard, kittyRest[0]!, kittyRest[1]!, kittyRest[2]!]
			}
		});
	}
	return deals;
}

export function playRollouts(
	view: PublicGameView,
	legal: readonly LegalMove[],
	targetSamples = PLAY_ROLLOUT_SAMPLES
) {
	const plays = legal.filter((move) => move.move.t === 'play');
	if (plays.length < 2) return {};
	const deals = possibleRemainingDeals(view, targetSamples);
	if (!deals.length) return {};
	const counters = Object.fromEntries(
		plays.map((play) => [play.id, { points: 0, tricks: 0, made: 0, swept: 0 }])
	);
	for (const initial of deals) {
		for (const play of plays) {
			let state = apply(initial, view.you, play.move).state;
			for (let step = 0; state.hand.phase !== 'hand_score' && step < 40; step++) {
				const seat = state.hand.turnSeat;
				if (seat === null) {
					state = advance(state).state;
					continue;
				}
				const player = project(state, seat);
				const id = topMove(player, player.hand, player.legal);
				const move = player.legal.find((candidate) => candidate.id === id);
				if (!move) throw new Error('Invalid play rollout move');
				state = apply(state, seat, move.move).state;
			}
			if (state.hand.phase !== 'hand_score') throw new Error('Play rollout did not score');
			const makerTeam = teamOf(view.makerSeat!);
			const count = counters[play.id]!;
			const result = state.hand.tricksWon;
			const makerPoints =
				result[makerTeam] < 3 ? -2 : result[makerTeam] < 5 ? 1 : view.aloneSeat !== null ? 4 : 2;
			count.points += teamOf(view.you) === makerTeam ? makerPoints : -makerPoints;
			count.tricks += result[teamOf(view.you)];
			count.made += Number(result[makerTeam] >= 3);
			count.swept += Number(result[makerTeam] === 5);
		}
	}
	return Object.fromEntries(
		plays.map((play) => {
			const count = counters[play.id]!;
			return [
				play.id,
				{
					sampledDeals: deals.length,
					meanTeamNetPoints: Math.round((count.points / deals.length) * 100) / 100,
					meanTeamTricks: Math.round((count.tricks / deals.length) * 100) / 100,
					makerMadeHands: count.made,
					makerSweptHands: count.swept,
					limitation:
						'Possible remaining hands consistent with public cards, counts, dealer pickup and proved voids; reference heuristic continuation. Small-sample estimate, not actual holdings or a guarantee. Use exact partnership facts for equivalent outcomes.'
				}
			];
		})
	);
}
