<script lang="ts">
	import type { PublicGameView } from '#lib/protocol/index.ts';
	import type { CompletedHandReview } from '#lib/protocol/hand-review.ts';
	import { cardName } from '#lib/euchre/index.ts';
	import { seatName } from './table-presentation';
	let {
		view,
		load
	}: {
		view: PublicGameView;
		load: (handNo: number) => Promise<CompletedHandReview | null | undefined>;
	} = $props();
	let dialog: HTMLDialogElement | undefined;
	function attachDialog(node: HTMLDialogElement) {
		dialog = node;
		return () => {
			dialog = undefined;
		};
	}
	let handNo = $state(0);
	let archive = $state.raw<CompletedHandReview | null>(null);
	let busy = $state(false);
	let error = $state('');
	let requestNo = 0;
	const tricks = $derived(archive?.tricks ?? (handNo === view.handNo ? view.trickLog : []));
	async function choose(value: number) {
		handNo = value;
		archive = null;
		error = '';
		busy = true;
		const request = ++requestNo;
		try {
			const result = await load(value);
			if (request !== requestNo) return;
			archive = result ?? null;
			if (!result && value !== view.handNo)
				error = 'This older hand was not saved by the previous version.';
		} catch {
			if (request === requestNo) error = 'Could not load this hand. Please try again.';
		} finally {
			if (request === requestNo) busy = false;
		}
	}
	function open() {
		dialog?.showModal();
		void choose(view.handNo);
	}
</script>

<button type="button" onclick={open}>↶ Review tricks</button>
<dialog {@attach attachDialog} aria-labelledby="review-title">
	<header>
		<h2 id="review-title">The cards on the table</h2>
		<button type="button" aria-label="Close trick review" onclick={() => dialog?.close()}>×</button>
	</header>
	<label
		>Hand <select value={handNo} onchange={(e) => void choose(Number(e.currentTarget.value))}
			>{#each Array.from({ length: view.handNo + 1 }, (_, i) => i) as number (number)}<option
					value={number}>{number + 1}</option
				>{/each}</select
		></label
	>
	{#if busy}<p role="status">Opening the hand…</p>{/if}
	{#if error}<p role="alert">{error}</p>{/if}
	{#each tricks as trick (trick.index)}
		<section>
			<h3>
				Trick {trick.index + 1} · {trick.winnerSeat !== null
					? seatName(trick.winnerSeat, view.you)
					: ''} won
			</h3>
			<div class="cards">
				{#each trick.plays as play, i (play.seat)}<figure>
						<img src={`/art/cards/${play.card}.png`} alt={cardName(play.card)} />
						<figcaption>{i + 1}. {seatName(play.seat, view.you)}</figcaption>
					</figure>{/each}
			</div>
		</section>
	{:else}<p>No completed tricks in this hand yet.</p>{/each}
	{#if archive}<section>
			<h3>Buried cards</h3>
			<div class="cards">
				{#each archive.buried as card (card)}<img
						src={`/art/cards/${card}.png`}
						alt={cardName(card)}
					/>{/each}
			</div>
		</section>
		{#if archive.unplayed.length}<section>
				<h3>Cards that stayed in hand</h3>
				<div class="cards">
					{#each archive.unplayed as card (card)}<img
							src={`/art/cards/${card}.png`}
							alt={cardName(card)}
						/>{/each}
				</div>
			</section>{/if}
	{:else}<p>The buried cards are revealed only after this hand is finished.</p>{/if}
</dialog>

<style>
	button,
	select {
		min-height: 44px;
		font: 600 16px var(--font-sans);
		padding: 8px 12px;
		background: #fffdf6;
		color: #233e32;
		border: 1px solid #bbc9ae;
		border-radius: 8px;
		cursor: pointer;
	}
	dialog {
		width: min(720px, calc(100vw - 24px));
		max-height: calc(100dvh - 24px);
		overflow-y: auto;
		padding: 24px;
		border: 0;
		border-radius: 16px;
		background: #f4f3e9;
		color: #233e32;
	}
	dialog::backdrop {
		background: #10251dcc;
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}
	h2 {
		font: 500 28px var(--font-serif);
	}
	h3 {
		font-size: 18px;
		margin-block: 24px 12px;
	}
	.cards {
		display: flex;
		gap: 12px;
		flex-wrap: wrap;
	}
	figure {
		margin: 0;
		width: calc((100% - 36px) / 4);
	}
	img {
		width: 100%;
		border-radius: 6px;
		box-shadow: 0 3px 6px #0002;
	}
	.cards > img {
		width: calc((100% - 36px) / 4);
	}
	figcaption {
		font-size: 14px;
		padding-top: 8px;
	}
	p {
		font-size: 17px;
		line-height: 1.4;
	}
</style>
