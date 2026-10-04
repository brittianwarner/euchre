<script lang="ts">
	import { Canvas } from '@threlte/core';
	import FeltSurface from './scene/FeltSurface.svelte';
</script>

<!-- Physical felt lighting is decorative. Every card and control remains readable native DOM. -->
<div class="surface" aria-hidden="true">
	<Canvas dpr={1} renderMode="on-demand"><FeltSurface /></Canvas>
</div>

<style>
	.surface {
		position: absolute;
		inset: 0;
		overflow: hidden;
		border-radius: inherit;
		pointer-events: none;
	}
</style>
