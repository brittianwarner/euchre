/** Public, deterministic facts for Jev; this module never chooses a move. */
import {
	DECK,
	cardName,
	cardValue,
	currentlyWinning,
	effectiveSuit,
	nextActiveSeat,
	partnerOf,
	plainRank,
	playsPerTrick,
	teamOf,
	trumpRank
} from '#lib/euchre/index.ts';
import type { CardId, LegalMove, PublicGameView, Seat } from '#lib/protocol/index.ts';
import { voidReads } from './heuristic';

export function analyzePartnership(view: PublicGameView, legal: readonly LegalMove[]) {
	const trump = view.trump;
	if (view.phase !== 'trick_play' || trump === null) return null;
	const team = teamOf(view.you);
	const partner = partnerOf(view.you);
	const plays = view.trick.plays;
	const winner = currentlyWinning(plays, trump);
	const seatsAfterYou: Seat[] = [];
	let next = view.you;
	for (let i = plays.length + 1; i < playsPerTrick(view.sittingSeat); i++) {
		next = nextActiveSeat(next, view.sittingSeat);
		seatsAfterYou.push(next);
	}
	const opponentsAfterYou = seatsAfterYou.filter((seat) => teamOf(seat) !== team);

	// An unseen card is only a possibility, never evidence of a particular holding.
	// A picked-up up-card may still be in the dealer's hand, so only exclude it
	// when it was turned down. The buried kitty/discard are deliberately unknown.
	const known = new Set<CardId>(view.hand);
	for (const trick of [...view.trickLog, view.trick]) {
		for (const play of trick.plays) known.add(play.card);
	}
	if (view.upCardTurnedDown && view.upCard !== null) known.add(view.upCard);
	const unseen = DECK.filter((card) => !known.has(card));
	const voids = voidReads(view);
	const possibleBeaters = (card: CardId, led: CardId): CardId[] => {
		const suit = effectiveSuit(led, trump);
		return unseen.filter(
			(candidate) =>
				cardValue(candidate, suit, trump) > cardValue(card, suit, trump) &&
				opponentsAfterYou.some((seat) => !voids[seat].includes(effectiveSuit(candidate, trump)))
		);
	};
	const first = plays[0];
	const partnerWinning = winner?.seat === partner;
	const threats = winner && first ? possibleBeaters(winner.card, first.card) : [];
	const makerTeam = view.makerSeat === null ? null : teamOf(view.makerSeat);
	const context = {
		yourTeam: team,
		partnerSeat: partner,
		partnerSittingOut: partner === view.sittingSeat,
		currentWinner: winner,
		partnerWinning,
		seatsAfterYou,
		opponentsAfterYou,
		// Even with opponents left, the right bower (or highest remaining trump)
		// can make a partner's lead unbeatable. All other uncertainty stays explicit.
		partnerTrickSecured: partnerWinning && threats.length === 0,
		possibleOpponentBeaters: threats,
		unseenCardsArePossibilitiesNotKnownHoldings: true,
		yourTeamTricks: view.tricksWon[team],
		opponentTricks: view.tricksWon[1 - team],
		yourTeamHasWonHand: view.tricksWon[team] >= 3,
		makerTeam,
		makersHaveThreeTricks: makerTeam !== null && view.tricksWon[makerTeam] >= 3,
		makersCanStillSweep: makerTeam !== null && view.tricksWon[1 - makerTeam] === 0,
		lonerSweepStopped:
			view.aloneSeat !== null && makerTeam !== null && view.tricksWon[1 - makerTeam] > 0
	};
	const options = legal.flatMap((move) => {
		if (move.move.t !== 'play') return [];
		const card = move.move.card;
		const after = currentlyWinning([...plays, { seat: view.you, card }], trump)!;
		const isTrump = effectiveSuit(card, trump) === trump;
		const rank = isTrump ? trumpRank(card, trump) : plainRank(card);
		return [
			{
				id: move.id,
				card,
				effectiveSuit: effectiveSuit(card, trump),
				rank,
				resource: isTrump
					? rank === 8
						? 'right bower, highest trump'
						: rank === 7
							? 'left bower, second-highest trump'
							: rank >= 5
								? 'high trump'
								: 'low trump'
					: rank === 6
						? 'plain ace'
						: rank <= 3
							? 'low plain card'
							: 'plain court card',
				winnerAfterPlay: after,
				overtakesPartner: partnerWinning && after.seat === view.you,
				possibleOpponentBeatersAfterPlay: possibleBeaters(after.card, first?.card ?? card),
				remainingHand: view.hand.filter((held) => held !== card)
			}
		];
	});
	return {
		context,
		options: options.map((option) => {
			// Same suit, same winning seat and same remaining threats: spending the
			// higher card gains neither protection nor the lead. Compare facts here,
			// not in the model, but leave every option available for Jev's judgment.
			const cheaperEquivalentCards = options
				.filter(
					(other) =>
						other.effectiveSuit === option.effectiveSuit &&
						other.rank < option.rank &&
						other.winnerAfterPlay.seat === option.winnerAfterPlay.seat &&
						other.possibleOpponentBeatersAfterPlay.join(',') ===
							option.possibleOpponentBeatersAfterPlay.join(',')
				)
				.map((other) => other.card);
			const teamWinning = teamOf(option.winnerAfterPlay.seat) === team;
			const secured = teamWinning && option.possibleOpponentBeatersAfterPlay.length === 0;
			const description = [
				`Play ${cardName(option.card)} (${option.resource}).`,
				secured
					? 'Our team is guaranteed this trick against the remaining opponents.'
					: teamWinning
						? 'Our team leads, but a remaining opponent could still beat it.'
						: 'An opponent remains ahead.',
				option.overtakesPartner
					? context.partnerTrickSecured
						? 'Overtakes an already secured partner winner without improving protection; only a justified transfer of the next lead could help.'
						: secured
							? 'Protects our vulnerable partner by removing every possible opponent beater.'
							: 'Takes the lead away from our partner, but opponents can still beat it.'
					: option.winnerAfterPlay.seat === partner
						? secured
							? 'Saves our stronger cards while our partner wins the trick.'
							: `Leaves our partner exposed to ${option.possibleOpponentBeatersAfterPlay.map(cardName).join(', ')} from a remaining opponent.`
						: '',
				cheaperEquivalentCards.length
					? `Unnecessary expenditure: ${cheaperEquivalentCards.map(cardName).join(' or ')} keeps the same winning seat and protection while saving this stronger card.`
					: '',
				`Keeps ${option.remainingHand.map(cardName).join(', ')} for later tricks.`
			]
				.filter(Boolean)
				.join(' ');
			return { ...option, cheaperEquivalentCards, description };
		})
	};
}
