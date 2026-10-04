import { SUIT_NAME } from '#lib/euchre/index.ts';
import type { CardId, PublicGameView, Seat } from '#lib/protocol/index.ts';

export { TABLE_MOTION, TRICK_PRESENTATION_MS } from '#lib/protocol/pacing.ts';

export function seatName(seat: Seat, you: Seat): string {
	return ['You', 'Left opponent', 'Your partner', 'Right opponent'][(seat - you + 4) % 4]!;
}

export interface HandSlots {
	handNo: number;
	cards: readonly CardId[];
}

/** Retain empty slots after plays. Syncs may remove cards, never move another hit target. */
export function reconcileHandSlots(previous: HandSlots | null, view: PublicGameView): HandSlots {
	if (!previous || previous.handNo !== view.handNo)
		return { handNo: view.handNo, cards: [...view.hand] };
	const added = view.hand.filter((card) => !previous.cards.includes(card));
	return added.length ? { ...previous, cards: [...previous.cards, ...added] } : previous;
}

export function turnInstruction(view: PublicGameView): string {
	if (view.status === 'abandoned')
		return 'This game has ended. Start a new game whenever you’re ready.';
	if (view.status === 'complete')
		return view.winnerTeam === view.you % 2
			? 'Your team wins the match!'
			: 'The other team wins the match.';
	if (view.phase === 'hand_score') return 'Hand complete. Adding the points…';
	if (view.phase === 'trick_resolve')
		return view.trick.winnerSeat === null
			? 'Finishing this trick…'
			: `${seatName(view.trick.winnerSeat, view.you)} ${view.trick.winnerSeat === view.you ? 'win' : 'wins'} the trick.`;
	if (view.sittingSeat === view.you) return 'Your partner is going alone. Enjoy the hand.';
	if (view.turnSeat !== view.you)
		return view.turnSeat === null
			? 'Getting the table ready…'
			: `${seatName(view.turnSeat, view.you)} is ${view.phase.startsWith('bid') ? 'deciding on trump' : 'taking a turn'}…`;
	if (view.phase === 'dealer_discard') return 'You’re the dealer. Select one card to discard.';
	if (view.phase === 'cutting') return 'Your choice: cut the deck, or let the dealer deal.';
	if (view.phase === 'bid_round_1' && view.upCard)
		return `Make ${SUIT_NAME[view.upCard[1] as keyof typeof SUIT_NAME].toLowerCase()} trump, or pass.`;
	if (view.phase === 'bid_round_2') return 'Choose a trump suit, or pass if you can.';
	return view.trick.ledSuit
		? `${SUIT_NAME[view.trick.ledSuit]} were led. Follow suit if you can.`
		: 'You lead. Select any card, then press Play.';
}
