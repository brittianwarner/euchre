<!--
  TrickPile.svelte — the cards currently on the table, one per seat that has
  played this trick.

  Unlike Hand/OpponentHand, this does not live inside a seat's `SeatAnchor` —
  it sits at the felt's centre, so its cards use table space directly
  (`trickCardPose` in `layout.ts`): each play is offset toward, and rotated to
  face, the seat that played it. A loner trick shows as few as 0-3 plays; there
  is nothing seat-count-specific to handle, since this simply maps over
  whatever `plays` it is given.

  The winning card is made "readable after the trick resolves" (the brief)
  using `Card.svelte`'s own existing `highlighted` prop — it lifts and
  brightens exactly like a legal card does in `Hand.svelte`. No new mechanism,
  no ownership of the read-pause timing: `winnerSeat` is `null` until the
  server has decided, and this component just reflects whatever it is told.
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import Card from '$lib/three/cards/Card.svelte';
	import { trickCardPose, type TrickOptions } from './layout';
	import type { CardId, Seat } from '$lib/euchre';

	interface Props {
		/** The current trick's plays in order, e.g. `view.trick.plays`. 0-4 entries (0-3 under a loner). */
		plays: readonly { seat: Seat; card: CardId }[];
		/** `view.trick.winnerSeat` — `null` until the trick resolves. */
		winnerSeat?: Seat | null;
		fourColor?: boolean;
		cardHeight?: number;
		/** Metres from the trick's centre to each seat's card slot. */
		radius?: number;
		/** World offset for the whole trick zone, so the composing scene can place it on the felt. */
		position?: readonly [number, number, number];
	}

	let {
		plays,
		winnerSeat = null,
		fourColor = false,
		cardHeight = 0.1,
		radius = 0.055,
		position = [0, 0, 0]
	}: Props = $props();

	const trickOptions = $derived<Partial<TrickOptions>>({ radius });

	function withOrigin(local: readonly [number, number, number]): [number, number, number] {
		return [position[0] + local[0], position[1] + local[1], position[2] + local[2]];
	}
</script>

<T.Group>
	<!-- Keyed on `seat`, not index: within one trick a seat plays exactly one card, so `seat` is real identity. -->
	{#each plays as play, i (play.seat)}
		{@const pose = trickCardPose(play.seat, i, trickOptions)}
		<Card
			id={play.card}
			faceUp
			position={withOrigin(pose.position)}
			rotation={pose.rotation}
			height={cardHeight}
			{fourColor}
			highlighted={winnerSeat === play.seat}
		/>
	{/each}
</T.Group>
