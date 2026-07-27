<!--
  SoundToggle.svelte — the one, obvious mute switch (docs/04-FRONTEND-UX.md §11:
  "every cue is duckable to zero; sound is never the only channel for
  anything"). Muted by default; this is how a player opts in.

  44px minimum tap target (ground-truth design floor), regardless of the glyph
  inside it — the hit area is the button's own padding/min-size, not the icon.
-->
<script lang="ts">
	import { soundSettings } from './sound.svelte';
</script>

<button
	type="button"
	class="sound-toggle"
	aria-pressed={soundSettings.enabled}
	aria-label={soundSettings.enabled ? 'Mute table sound' : 'Unmute table sound'}
	title={soundSettings.enabled ? 'Sound on' : 'Sound off'}
	onclick={() => soundSettings.toggle()}
>
	{#if soundSettings.enabled}
		<!-- speaker, sound waves -->
		<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
			<path
				fill="currentColor"
				d="M4 9v6h4l5 5V4L8 9H4Zm12.5 3a4.5 4.5 0 0 0-2.5-4.03v8.06A4.5 4.5 0 0 0 16.5 12Zm-2.5-8.77v2.06a7 7 0 0 1 0 13.42v2.06a9 9 0 0 0 0-17.54Z"
			/>
		</svg>
	{:else}
		<!-- speaker, muted (X) -->
		<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
			<path
				fill="currentColor"
				d="M4 9v6h4l5 5V4L8 9H4Zm11.59 3 2.71 2.71-1.41 1.41L14.18 13.4l-2.71 2.72-1.41-1.41L12.77 12l-2.71-2.71 1.41-1.41L14.18 10.6l2.71-2.72 1.41 1.41L15.59 12Z"
			/>
		</svg>
	{/if}
</button>

<style>
	.sound-toggle {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 44px;
		height: 44px;
		border-radius: 999px;
		border: 1px solid rgba(232, 194, 122, 0.35);
		background: rgba(15, 20, 14, 0.72);
		color: #e8c27a;
		cursor: pointer;
		transition: background-color 120ms ease;
	}
	.sound-toggle:hover {
		background: rgba(15, 20, 14, 0.9);
	}
	.sound-toggle:focus-visible {
		outline: 2px solid #e8c27a;
		outline-offset: 2px;
	}
	@media (prefers-reduced-motion: reduce) {
		.sound-toggle {
			transition-duration: 0.001ms;
		}
	}
</style>
