<script lang="ts">
	import { untrack, onDestroy } from 'svelte';
	import type { PublicGameView, Step } from '#lib/protocol/index.ts';
	import { reducedMotion } from '#lib/three/layout/reducedMotion.svelte.ts';
	import { playTrumpCalled, playMarch, playEuchre, playHandWon, playGameWon } from './sound';
	import { seatName } from './table-presentation';
	let { view, steps }: { view: PublicGameView; steps: readonly Step[] } = $props();
	let notice = $state<{ text: string; celebrate: boolean } | null>(null);
	// Event deduplication is deliberately non-reactive; this is an audio/animation side effect.
	const seen = new Set<string>();
	let timeout: ReturnType<typeof setTimeout> | undefined;
	function announce(message: { text: string; celebrate: boolean }) {
		notice = message;
		clearTimeout(timeout);
		timeout = setTimeout(() => (notice = null), 4500);
	}
	onDestroy(() => clearTimeout(timeout));
	$effect(() => {
		const current = view;
		const events = steps;
		untrack(() => {
			for (const step of events) {
				const key = `${current.handNo}:${step.t}:${JSON.stringify(step)}`;
				if (seen.has(key)) continue;
				seen.add(key);
				if (seen.size > 80) seen.delete(seen.values().next().value!);
				if (step.t === 'trumpSet') {
					playTrumpCalled(step.suit);
					if (step.aloneSeat !== null) {
						playMarch();
						announce({
							text: `${seatName(step.aloneSeat, current.you)} ${step.aloneSeat === current.you ? 'are' : 'is'} going alone!`,
							celebrate: false
						});
					}
				}
				if (step.t === 'handScored') {
					if (step.result === 'lone_march') {
						playMarch();
						announce({ text: 'All five alone! Four points.', celebrate: true });
					} else if (step.result === 'euchre') playEuchre();
					else if (step.result === 'march') playMarch();
					else playHandWon();
				}
				if (step.t === 'gameWon') {
					playGameWon();
					announce({
						text: step.team === current.you % 2 ? 'Your team wins!' : 'A good game. Well played.',
						celebrate: step.team === current.you % 2
					});
				}
			}
		});
	});
</script>

{#if notice}<aside class="moment" role="status">
		<span aria-hidden="true">★</span><strong>{notice.text}</strong>
	</aside>
	{#if notice.celebrate && !reducedMotion.enabled}<div class="confetti" aria-hidden="true">
			{#each Array.from({ length: 24 }, (_, i) => i) as i (i)}<i
					style:--x={`${(i * 37) % 100}%`}
					style:--delay={`${(i % 6) * 70}ms`}
					style:--color={['#e0c579', '#497d69', '#ba5e46'][i % 3]}
				></i>{/each}
		</div>{/if}
{/if}

<style>
	.moment {
		position: fixed;
		z-index: 12;
		top: 104px;
		left: 50%;
		transform: translateX(-50%);
		max-width: calc(100vw - 24px);
		padding: 18px 24px;
		border-radius: 14px;
		background: #fff8df;
		color: #253c2d;
		box-shadow: 0 8px 32px #00180d44;
		font: 600 24px/1.3 var(--font-serif);
		pointer-events: none;
		text-align: center;
	}
	.moment span {
		color: #967629;
		padding-right: 10px;
	}
	.confetti {
		position: fixed;
		inset: 0;
		overflow: hidden;
		pointer-events: none;
		z-index: 11;
	}
	i {
		position: absolute;
		left: var(--x);
		top: -12px;
		width: 8px;
		height: 14px;
		background: var(--color);
		animation: fall 2400ms var(--delay) ease-in both;
	}
	@keyframes fall {
		to {
			transform: translateY(100vh) rotate(540deg);
			opacity: 0;
		}
	}
	@media (max-width: 700px) {
		.moment {
			font-size: 20px;
			top: 96px;
			padding: 14px 18px;
		}
	}
</style>
