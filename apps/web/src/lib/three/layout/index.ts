/**
 * `$lib/three/layout` — the euchre table's card arrangements.
 *
 * Pure geometry (`layout.ts`) plus the four components that compose it with
 * `Card.svelte`: `Hand`, `OpponentHand`, `TrickPile`, `Kitty`. See `layout.ts`'s
 * module doc for the coordinate conventions every one of them assumes.
 */

export {
	CARD_ASPECT,
	DEFAULT_FAN_OPTIONS,
	DEFAULT_STACK_OPTIONS,
	DEFAULT_TRICK_OPTIONS,
	MIN_STRIP_H,
	MIN_STRIP_W,
	SEAT_DIRECTION,
	SEAT_FACING_RAD,
	fanPositions,
	pxPerMetreAt,
	sizeFan,
	stackPositions,
	trickCardPose
} from './layout';
export type {
	CardPose,
	FanOptions,
	FanSizing,
	KittyStage,
	StackOptions,
	TrickOptions
} from './layout';

export { default as Hand } from './Hand.svelte';
export { default as OpponentHand } from './OpponentHand.svelte';
export { default as TrickPile } from './TrickPile.svelte';
export { default as Kitty } from './Kitty.svelte';
