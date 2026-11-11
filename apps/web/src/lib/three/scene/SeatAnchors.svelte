<!--
  SeatAnchors.svelte — four named, empty `<T.Group>`s at the four chairs.

  This component owns no card, no hand, no kitty — it owns *where things
  go*. The card layer composes it and fills each seat with a snippet:

  ```svelte
  <SeatAnchors>
    {#snippet seat0(t)}
      <PlayerHand ... />
    {/snippet}
    {#snippet seat2(t)}
      <OpponentHand ... />
    {/snippet}
  </SeatAnchors>
  ```

  Every snippet receives the seat's `SeatTransform` (seat index, compass
  label, world position, rotation) even though the group already applied
  that transform — useful when a caller needs the numbers directly, e.g. to
  billboard a name tag toward the camera instead of toward table-local +Z.

  The local-space convention (documented in full in `seatLayout.ts`): a
  seat's `<T.Group>` local **+Z axis always points from that seat toward the
  table centre**. A hand fan authored once in that local frame is correct at
  all four seats — this is exactly why per-card maths never needs to know
  which seat it's in, only that it's "a seat."

  Radius is responsive on its own — §14 shrinks the seat ring from 0.30 m to
  0.26 m in portrait — so a caller normally passes nothing and gets the
  right ring for the current canvas aspect. An explicit `radius` prop is
  still honoured, for a caller (or a future spectator/replay layout) that
  needs to override it.
-->
<script lang="ts">
	import { T, useThrelte } from '@threlte/core';
	import type { Snippet } from 'svelte';
	import { isPortrait } from './breakpoints';
	import {
		seatLayout,
		SEAT_RADIUS_LANDSCAPE,
		SEAT_RADIUS_PORTRAIT,
		type SeatTransform
	} from './seatLayout';

	interface Props {
		/** Distance from centre to each anchor, world metres. Defaults to the responsive standard. */
		radius?: number;
		/** Height of every anchor pivot above the felt surface, world metres. */
		y?: number;
		seat0?: Snippet<[SeatTransform]>;
		seat1?: Snippet<[SeatTransform]>;
		seat2?: Snippet<[SeatTransform]>;
		seat3?: Snippet<[SeatTransform]>;
	}

	let { radius, y = 0, seat0, seat1, seat2, seat3 }: Props = $props();

	const { size } = useThrelte();
	const aspect = $derived($size.width / Math.max(1, $size.height));
	const responsiveRadius = $derived(
		isPortrait(aspect) ? SEAT_RADIUS_PORTRAIT : SEAT_RADIUS_LANDSCAPE
	);
	const effectiveRadius = $derived(radius ?? responsiveRadius);

	const layout = $derived(seatLayout({ radius: effectiveRadius, y }));
	const south = $derived(layout[0]);
	const west = $derived(layout[1]);
	const north = $derived(layout[2]);
	const east = $derived(layout[3]);
</script>

<!-- `T.Group`'s position/rotation props want mutable tuples; `SeatTransform`'s
     fields are `readonly` on purpose (shared, cached data), so each is
     spread into a fresh mutable tuple here rather than losing the
     `readonly` on the shared type. -->
<T.Group name="seat-0-south" position={[...south.position]} rotation={[...south.rotation]}>
	{#if seat0}{@render seat0(south)}{/if}
</T.Group>

<T.Group name="seat-1-west" position={[...west.position]} rotation={[...west.rotation]}>
	{#if seat1}{@render seat1(west)}{/if}
</T.Group>

<T.Group name="seat-2-north" position={[...north.position]} rotation={[...north.rotation]}>
	{#if seat2}{@render seat2(north)}{/if}
</T.Group>

<T.Group name="seat-3-east" position={[...east.position]} rotation={[...east.rotation]}>
	{#if seat3}{@render seat3(east)}{/if}
</T.Group>
