import type { CardId, HandResult, Seat, Suit, Trick } from './index';
/** A separate post-hand contract. Never included in a live view or an AI request. */
export interface CompletedHandReview {
	handNo: number;
	/** Optional for archives created before the review summary was added. */
	dealerSeat?: Seat;
	tricksWon?: readonly [number, number];
	upCard?: CardId | null;
	trump: Suit | null;
	makerSeat: Seat | null;
	aloneSeat: Seat | null;
	tricks: readonly Trick[];
	buried: readonly CardId[];
	unplayed: readonly CardId[];
	result: HandResult | null;
	delta: readonly [number, number] | null;
}
