/** Evidence a person in this chair could remember. Never reads privileged game state. */
import {
	DECK,
	SUITS,
	effectiveSuit,
	partnerOf,
	sameColor,
	teamOf,
	trumpRank
} from '#lib/euchre/index.ts';
import type { CardId, PublicGameView, Seat } from '#lib/protocol/index.ts';
import { voidReads } from './heuristic';
import { splitTricks } from './notation';

export function tableContext(view: PublicGameView) {
	const { current, past } = splitTricks(view);
	const played = [...past, ...(current ? [current] : [])].flatMap((trick) => trick.plays);
	const known = new Set<CardId>([...view.hand, ...played.map((play) => play.card)]);
	if (view.upCardTurnedDown && view.upCard) known.add(view.upCard);
	const unseen = DECK.filter((card) => !known.has(card));
	const voids = voidReads(view);
	const team = teamOf(view.you);
	const makerTeam = view.makerSeat === null ? null : teamOf(view.makerSeat);
	const suitCounts = Object.fromEntries(
		SUITS.map((suit) => [
			suit,
			view.hand.filter((card) => effectiveSuit(card, view.trump) === suit).length
		])
	);
	const trump = view.trump;
	const outstandingTrump =
		trump === null
			? []
			: unseen
					.filter((card) => effectiveSuit(card, trump) === trump)
					.sort((a, b) => trumpRank(b, trump) - trumpRank(a, trump));
	return {
		partnerSeat: partnerOf(view.you),
		yourRole: makerTeam === null ? 'auction' : makerTeam === team ? 'maker' : 'defender',
		objective:
			makerTeam === null
				? 'Choose a bid for expected partnership points; account for who deals.'
				: makerTeam === team
					? view.tricksWon[team] >= 3
						? view.tricksWon[1 - team] === 0
							? 'Hand made; pursue the sweep for the extra points.'
							: 'Hand made; conserve resources and finish soundly.'
						: 'Take three tricks to make the hand; keep a route to the next lead.'
					: view.aloneSeat !== null && view.tricksWon[team] === 0
						? 'Stop the four-point loner sweep; then pursue three tricks for a euchre.'
						: 'Take three tricks to euchre; partner winners count equally.',
		pointsNeededToTen: [Math.max(0, 10 - view.score[team]), Math.max(0, 10 - view.score[1 - team])],
		cardsInEachEffectiveSuit: suitCounts,
		yourVoids: SUITS.filter((suit) => suitCounts[suit] === 0),
		unseenTrump: outstandingTrump,
		unseenTrumpWarning:
			'Possible cards in opponents, partner, or buried kitty/discard. Not known holdings or calibrated probabilities.',
		publicReads: ([0, 1, 2, 3] as Seat[])
			.filter((seat) => seat !== view.you)
			.map((seat) => ({
				seat,
				relation: seat === partnerOf(view.you) ? 'partner' : 'opponent',
				provedVoids: voids[seat],
				cardsPlayed: played.filter((play) => play.seat === seat).map((play) => play.card),
				tricksTaken: past.filter((trick) => trick.winnerSeat === seat).length,
				remainingCards: view.handCounts[seat]
			})),
		auction: {
			dealerIsPartner: view.dealerSeat === partnerOf(view.you),
			dealerIsYou: view.dealerSeat === view.you,
			upCard: view.upCard,
			upCardLocation: view.upCardTurnedDown
				? 'out of play'
				: view.makerSeat === null
					? 'on the table'
					: view.turnedDownSuit === null
						? 'dealer picked it up; it may subsequently have been discarded or played'
						: 'out of play',
			nextSuit:
				view.turnedDownSuit === null
					? null
					: SUITS.find(
							(suit) => suit !== view.turnedDownSuit && sameColor(suit, view.turnedDownSuit!)
						),
			calls: view.bids.map((bid, index) => ({
				round: index < 4 ? 1 : 2,
				seat: bid.seat,
				move: bid.move
			})),
			rules: view.rules ?? { stickTheDealer: false, requireNaturalTrump: false }
		},
		observedActions: (view.observations ?? []).filter((o) => o.handNo === view.handNo).slice(-32),
		timingCaution:
			'Elapsed time is server-observed, includes network and distraction. Computer timing is deliberately paced, not a tell. Human timing is weak context only: never infer a specific card, override card evidence, delay deliberately to signal, or punish slow players.'
	};
}
