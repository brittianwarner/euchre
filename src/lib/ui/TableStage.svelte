<script lang="ts">
	import { onMount, untrack, type Component } from 'svelte';
	import { cardName, SUIT_NAME } from '#lib/euchre/index.ts';
	import type { CardId, PublicGameView, Seat } from '#lib/protocol/index.ts';
	import { reducedMotion } from '#lib/three/layout/reducedMotion.svelte.ts';
	import { playPlace, playTrickTake } from './sound';
	import { TABLE_MOTION, seatName } from './table-presentation';
	let FeltLayer = $state<Component | null>(null);
	onMount(() => {
		let active = true;
		void import('#lib/three/TableFelt3D.svelte')
			.then((module) => {
				if (active) FeltLayer = module.default;
			})
			.catch(() => {
				/* The CSS felt remains usable without WebGL. */
			});
		return () => {
			active = false;
		};
	});

	export interface CardOrigin {
		card: CardId;
		rect: DOMRect;
		at: number;
	}
	let {
		view,
		origin = null,
		connected = true,
		onContinue
	}: {
		view: PublicGameView;
		origin?: CardOrigin | null;
		connected?: boolean;
		onContinue: (turnId: string) => Promise<boolean>;
	} = $props();
	let surface = $state<HTMLElement>();
	let collecting = $state(false);
	let collectedKey = $state<string | null>(null);
	const seats = $derived([1, 2, 3, 0].map((offset) => ((view.you + offset) % 4) as Seat));
	const winner = $derived(view.trick.winnerSeat);
	const trickKey = $derived(`${view.handNo}:${view.trick.index}`);
	const winnerKey = $derived(winner === null ? null : trickKey);
	const bidding = $derived(
		['lobby', 'cutting', 'deal', 'bid_round_1', 'bid_round_2', 'dealer_discard'].includes(
			view.phase
		)
	);
	const team = $derived(view.you % 2);
	const seen = new Set(
		untrack(() => view.trick.plays.map((play) => `${view.handNo}:${view.trick.index}:${play.card}`))
	);
	const animations = new Set<Animation>();
	const suitGlyph = { H: '♥', D: '♦', S: '♠', C: '♣' };

	function arrive(node: HTMLElement) {
		const context = untrack(() => ({ you: view.you, origin, reduced: reducedMotion.enabled }));
		const key = node.dataset.playKey!;
		if (seen.has(key)) return;
		seen.add(key);
		if (seen.size > 40) seen.delete(seen.values().next().value!);
		if (context.reduced) return;
		const seat = Number(node.dataset.seat);
		const rect = node.getBoundingClientRect();
		const fromHand =
			seat === context.you &&
			context.origin &&
			context.origin.card === node.dataset.card &&
			performance.now() - context.origin.at < 2500
				? context.origin.rect
				: null;
		const anchor = surface?.querySelector(`[data-seat-anchor="${seat}"]`)?.getBoundingClientRect();
		const dx = fromHand
			? fromHand.left - rect.left
			: anchor
				? anchor.left + anchor.width / 2 - rect.left - rect.width / 2
				: 0;
		const dy = fromHand ? fromHand.top - rect.top : seat === context.you ? 130 : -95;
		const scale = fromHand ? fromHand.width / rect.width : 0.75;
		const animation = node.animate(
			[
				{
					transform: `translate(${dx}px,${dy}px) scale(${scale}) rotate(${seat === context.you ? 0 : -6}deg)`,
					opacity: 0.4
				},
				{ transform: 'translate(0,-8px) scale(1.025) rotate(1deg)', opacity: 1, offset: 0.82 },
				{ transform: 'none', opacity: 1 }
			],
			{ duration: TABLE_MOTION.play, easing: 'cubic-bezier(.22,.7,.25,1)' }
		);
		animations.add(animation);
		animation.onfinish = () => {
			animations.delete(animation);
			playPlace();
		};
		return () => {
			animation.cancel();
			animations.delete(animation);
		};
	}

	async function collect(): Promise<void> {
		if (!surface || winner === null || collecting || collectedKey === trickKey) return;
		const key = trickKey;
		collecting = true;
		const target = surface.querySelector(`[data-stack="${winner}"]`)?.getBoundingClientRect();
		const cards = Array.from(surface.querySelectorAll<HTMLElement>('[data-play-key]'));
		try {
			if (target && !reducedMotion.enabled) {
				await Promise.all(
					cards.map(async (card, index) => {
						const from = card.getBoundingClientRect();
						const dx = target.left + target.width / 2 - from.left - from.width / 2;
						const dy = target.top + target.height / 2 - from.top - from.height / 2;
						const animation = card.animate(
							[
								{ transform: 'none', opacity: 1 },
								{
									transform: `translate(${dx * 0.55}px,${dy * 0.55 - 20}px) scale(.7) rotate(-5deg)`,
									opacity: 1,
									offset: 0.6
								},
								{ transform: `translate(${dx}px,${dy}px) scale(.18) rotateY(90deg)`, opacity: 0 }
							],
							{
								duration: TABLE_MOTION.collect,
								delay: index * TABLE_MOTION.stagger,
								easing: 'cubic-bezier(.4,0,.2,1)',
								fill: 'forwards'
							}
						);
						animations.add(animation);
						try {
							await animation.finished;
						} catch {
							/* A new hand or unmount cancels safely. */
						}
						animations.delete(animation);
					})
				);
			}
			if (key === trickKey) {
				collectedKey = key;
				playTrickTake();
			}
		} finally {
			collecting = false;
		}
	}

	async function continueTrick() {
		const turnId = view.turnId;
		await collect();
		const ok = await onContinue(turnId);
		if (!ok) {
			collectedKey = null;
			for (const card of surface?.querySelectorAll<HTMLElement>('[data-play-key]') ?? [])
				card.getAnimations().forEach((animation) => animation.cancel());
		}
	}

	// Only a completed trick starts the sequence. Ordinary syncs never restart it.
	$effect(() => {
		const key = winnerKey;
		const review = view.reviewTricks;
		if (!key || review) return;
		const timer = setTimeout(() => void collect(), TABLE_MOTION.play + TABLE_MOTION.read);
		return () => clearTimeout(timer);
	});
	$effect(() => () => {
		for (const animation of animations) animation.cancel();
	});
</script>

<section
	class="felt"
	bind:this={surface}
	aria-label="The table"
	class:collected={collectedKey === trickKey}
>
	{#if FeltLayer}<FeltLayer />{/if}
	<div class="felt-content">
		<div class="table-status">
			<div class="trump">
				<span class="eyebrow">Hand {view.handNo + 1}</span><strong
					>{#if view.trump}<span aria-hidden="true">{suitGlyph[view.trump]}</span>
						{SUIT_NAME[view.trump]} are trump{:else}Choosing trump{/if}</strong
				>
			</div>
			<div class="trick-score">
				<span class="trick-caption">Tricks this hand</span><span class="short-trick-caption">Tricks</span><strong
					>Your team <b>{view.tricksWon[team]}</b><i>·</i> Other team
					<b>{view.tricksWon[1 - team]}</b></strong
				>
			</div>
		</div>
		<div class="seats">
			{#each seats as seat (seat)}
				<div
					class="seat"
					class:active={view.turnSeat === seat}
					class:partner={(seat - view.you + 4) % 4 === 2}
					data-seat-anchor={seat}
				>
					<span class="seat-name"
						>{seatName(seat, view.you)}{#if seat === view.dealerSeat}<span
								class="dealer"
								title="Dealer">D<span class="sr-only">ealer</span></span
							>{/if}</span
					>
					<span class="seat-detail"
						>{view.sittingSeat === seat
							? 'Sitting out · partner alone'
							: view.turnSeat === seat
								? seat === view.you
									? 'Your turn'
									: 'Thinking…'
								: `${view.handCounts[seat]} cards`}</span
					>
					<span
						class="taken-stack"
						class:has-tricks={view.trickLog.some((trick) => trick.winnerSeat === seat)}
						data-stack={seat}
						aria-hidden="true"
					></span>
				</div>
			{/each}
		</div>
		{#if bidding}
			<div class="bid-table">
				{#if view.upCard && !view.upCardTurnedDown && view.phase !== 'dealer_discard'}
					<img
						class="upcard"
						src={`/art/cards/${view.upCard}.png`}
						width="635"
						height="889"
						alt={`Up card: ${cardName(view.upCard)}`}
					/>
					<div>
						<span class="eyebrow">The up card</span>
						<h2>{cardName(view.upCard)}</h2>
						<p>
							{seatName(view.dealerSeat, view.you)}
							{view.dealerSeat === view.you ? 'pick' : 'picks'} it up if its suit becomes trump.
						</p>
					</div>
				{:else}<div class="deck" aria-hidden="true">♣</div>
					<div>
						<span class="eyebrow">A seat at the table</span>
						<h2>
							{view.phase === 'bid_round_2'
								? 'A second chance to call.'
								: view.phase === 'dealer_discard'
									? 'Six cards. Keep your best five.'
									: 'A fresh hand awaits.'}
						</h2>
						<p>
							{view.phase === 'bid_round_2' && view.turnedDownSuit
								? `${SUIT_NAME[view.turnedDownSuit]} were turned down. Choose another suit.`
								: 'You and your partner play together. First team to 10 wins.'}
						</p>
					</div>{/if}
			</div>
		{:else}
			<div class="played-cards">
				{#each seats as seat (seat)}
					{@const play = view.trick.plays.find((entry) => entry.seat === seat)}
					<div class="played-slot" class:winner={winner === seat}>
						{#if play}{#key `${trickKey}:${play.card}`}
								<img
									{@attach arrive}
									data-play-key={`${trickKey}:${play.card}`}
									data-card={play.card}
									data-seat={seat}
									class="played-card"
									src={`/art/cards/${play.card}.png`}
									width="635"
									height="889"
									alt={`${seatName(seat, view.you)} played ${cardName(play.card)}${winner === seat ? ' — winning card' : ''}`}
									draggable="false"
								/>
							{/key}{:else}<span class="empty-play"
								>{view.sittingSeat === seat ? 'Sitting out' : ' '}</span
							>{/if}
					</div>
				{/each}
			</div>
		{/if}
		<div class="trick-result" class:won={winner !== null}>
			{#if winner !== null}<span
					><strong
						>{seatName(winner, view.you)} {winner === view.you ? 'win' : 'wins'} the trick.</strong
					>
					{view.trick.index < 4
						? `${seatName(winner, view.you)} ${winner === view.you ? 'lead' : 'leads'} next.`
						: 'That’s the last trick of this hand.'}</span
				>
				{#if view.reviewTricks && view.phase === 'trick_resolve'}<button
						type="button"
						disabled={!view.awaitingTrickReview || collecting || !connected}
						onclick={continueTrick}
						>{collecting ? 'Collecting…' : 'Continue'} <span aria-hidden="true">→</span></button
					>{/if}
			{:else if view.aloneSeat !== null}<span
					>{seatName(view.aloneSeat, view.you)}
					{view.aloneSeat === view.you ? 'are' : 'is'} going alone.</span
				>{:else}<span
					>{bidding
						? 'Take your time. There’s no rush.'
						: `Trick ${view.trick.index + 1} of 5 · The winner leads the next trick.`}</span
				>{/if}
		</div>
	</div>
</section>

<style>
	.felt {
		position: relative;
		isolation: isolate;
		color: #fffdf3;
		background: #285641;
		border: 1px solid #1b3f30;
		border-top: 5px solid #44684e;
		box-shadow:
			inset 0 4px 9px #00180d30,
			0 3px 0 #153c2b;
		border-radius: 18px;
	}
	.felt-content {
		position: relative;
		padding: 24px 32px 16px;
	}
	.table-status {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 20px;
		padding-bottom: 22px;
	}
	.eyebrow {
		display: block;
		font-size: 14px;
		letter-spacing: 0.055em;
		color: #d9e6cb;
		margin-bottom: 4px;
	}
	.trump strong {
		font: 500 clamp(23px, 2.4vw, 30px)/1.25 var(--font-serif);
	}
	.trump strong span {
		color: #e0cf97;
	}
	.trick-score {
		text-align: right;
		font-size: 16px;
	}
	.short-trick-caption { display: none; }
	.trick-score strong {
		display: flex;
		align-items: center;
		gap: 10px;
		font-size: 18px;
		font-weight: 500;
		margin-top: 4px;
	}
	.trick-score b {
		font-size: 25px;
		font-variant-numeric: tabular-nums;
	}
	.trick-score i {
		font-style: normal;
		color: #bdceb2;
		padding-inline: 5px;
	}
	.seats,
	.played-cards {
		display: grid;
		grid-template-columns: repeat(4, minmax(0, 1fr));
		gap: 24px;
		max-width: 850px;
		margin-inline: auto;
	}
	.seat {
		text-align: center;
		position: relative;
		padding: 8px 4px;
		border-bottom: 2px solid #ffffff26;
	}
	.seat.active {
		border-color: #e6d08c;
		background: #ffffff0a;
		border-radius: 7px 7px 0 0;
	}
	.seat-name {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 6px;
		font-size: 18px;
		font-weight: 600;
	}
	.seat-detail {
		display: block;
		font-size: 14px;
		color: #e1e8d9;
		min-height: 22px;
		margin-top: 4px;
	}
	.dealer {
		display: grid;
		place-items: center;
		width: 24px;
		height: 24px;
		flex-shrink: 0;
		border-radius: 50%;
		background: #f4ead0;
		color: #35482f;
		box-shadow: 0 2px 0 #b4a57a;
		font-size: 13px;
	}
	.taken-stack {
		position: absolute;
		top: -9px;
		right: 5px;
		width: 24px;
		height: 16px;
		border-radius: 2px;
		opacity: 0;
		background: repeating-linear-gradient(45deg, #e8e2ca 0 1px, #285743 1px 4px);
		border: 2px solid #f8f0db;
		box-shadow:
			2px 2px #c6c6b5,
			4px 4px #e7e5d8;
	}
	.taken-stack.has-tricks {
		opacity: 0.8;
	}
	.played-cards {
		padding-top: 20px;
	}
	.played-slot {
		display: grid;
		place-items: center;
		min-width: 0;
	}
	.played-card,
	.empty-play {
		width: min(100%, 145px);
		aspect-ratio: 5/7;
		height: auto;
		border-radius: 7px;
	}
	.played-card {
		position: relative;
		box-shadow:
			0 2px 0 #d9d8ca,
			0 4px 0 #abb3a0,
			0 9px 15px #082b2350;
		transform-origin: center;
	}
	.winner .played-card {
		outline: 3px solid #eed591;
		outline-offset: 5px;
	}
	.empty-play {
		border: 0;
		display: grid;
		place-items: center;
		font-size: 15px;
		color: #d9e6cb;
	}
	.collected .played-card {
		visibility: hidden;
	}
	.trick-result {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 20px;
		text-align: center;
		min-height: 62px;
		margin-top: 14px;
		font-size: 16px;
		color: #e0e9d7;
	}
	.trick-result.won {
		color: #ffecb7;
		font-size: 18px;
	}
	.trick-result strong {
		font-weight: 650;
	}
	.trick-result button {
		min-height: 48px;
		padding: 10px 20px;
		background: #f5e8c3;
		color: #233e32;
		border: 0;
		border-radius: 7px;
		font-weight: 650;
		cursor: pointer;
	}
	.trick-result button:disabled {
		opacity: 0.65;
		cursor: wait;
	}
	.bid-table {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 28px;
		padding-top: 20px;
		min-height: 223px;
	}
	.upcard,
	.deck {
		width: 137px;
		height: 192px;
		border-radius: 7px;
		box-shadow:
			0 3px #c6c9ba,
			0 6px #aeb7a6,
			0 12px 20px #0d281b55;
		transform: perspective(900px) rotateX(4deg) rotate(-3deg);
	}
	.deck {
		flex-shrink: 0;
		display: grid;
		place-items: center;
		font-size: 48px;
		color: #f2e8cd;
		border: 6px solid #f4f0df;
		outline: 1px solid #ffffff70;
		background: repeating-linear-gradient(45deg, #3a6448 0 2px, #2a4f3b 2px 5px);
	}
	.bid-table h2 {
		font: 500 clamp(24px, 3vw, 32px)/1.2 var(--font-serif);
		margin: 6px 0 10px;
	}
	.bid-table p {
		max-width: 300px;
		font-size: 17px;
		line-height: 1.5;
		margin: 0;
		color: #e0e9d7;
	}
	@media (min-width: 701px) and (max-height: 850px) {
		.felt-content {
			padding: 12px 24px 4px;
		}
		.table-status {
			padding-bottom: 8px;
		}
		.trump {
			display: flex;
			align-items: center;
			gap: 14px;
		}
		.trump .eyebrow {
			margin: 0;
		}
		.trump strong {
			font-size: 25px;
		}
		.trick-score {
			display: flex;
			align-items: center;
			gap: 14px;
		}
		.trick-score strong {
			margin: 0;
		}
		.seat {
			padding: 5px 4px;
		}
		.seat-name {
			font-size: 17px;
		}
		.seat-detail {
			margin: 1px 0 0;
			font-size: 14px;
		}
		.played-cards {
			padding-top: 12px;
		}
		.played-card,
		.empty-play {
			width: 108px;
		}
		.trick-result {
			min-height: 44px;
			margin-top: 10px;
		}
		.bid-table {
			min-height: 163px;
			padding-top: 12px;
		}
		.upcard,
		.deck {
			width: 105px;
			height: 147px;
		}
		.trick-result button {
			min-height: 42px;
			padding: 8px 16px;
		}
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip-path: inset(50%);
	}
	@media (max-width: 700px) {
		.felt-content {
			padding: 18px 16px 8px;
		}
		.seats,
		.played-cards {
			gap: 12px;
		}
		.seat-name {
			font-size: 15px;
			flex-wrap: wrap;
			gap: 2px 6px;
		}
		.seat-detail {
			font-size: 13px;
		}
		.trick-score strong {
			font-size: 15px;
			gap: 6px;
		}
		.trick-score i {
			padding: 0;
		}
		.dealer {
			width: 20px;
			height: 20px;
		}
	}
	@media (max-width: 480px) {
		.felt {
			border-radius: 12px;
		}
		.table-status {
			flex-direction: column;
			align-items: stretch;
			gap: 12px;
			padding-bottom: 14px;
		}
		.trump {
			display: flex;
			align-items: center;
			justify-content: space-between;
			gap: 8px;
		}
		.trump strong {
			font-size: 23px;
		}
		.trump .eyebrow {
			margin: 0;
			white-space: nowrap;
		}
		.trick-score {
			display: flex;
			justify-content: space-between;
			align-items: center;
			font-size: 14px;
		}
		.trick-score strong {
			margin: 0;
		}
		.trick-score b {
			font-size: 21px;
		}
		.seats,
		.played-cards {
			gap: 10px;
		}
		.seat-name {
			min-height: 42px;
			line-height: 1.2;
			align-content: center;
		}
		.seat-detail {
			min-height: 36px;
			line-height: 1.25;
		}
		.played-cards {
			padding-top: 15px;
		}
		.trick-result {
			font-size: 14px;
			line-height: 1.5;
			gap: 10px;
		}
		.trick-result.won {
			font-size: 16px;
		}
		.trick-result button {
			padding: 10px 12px;
		}
		.bid-table {
			gap: 18px;
			min-height: 183px;
		}
		.upcard,
		.deck {
			width: 100px;
			height: 140px;
		}
		.bid-table h2 {
			font-size: 25px;
		}
		.bid-table p {
			font-size: 15px;
		}
	}
	/* Cards flex into a bounded table; no game state can push the hand off-screen. */
	.felt {
		min-height: 0;
	}
	.felt-content {
		height: 100%;
		min-height: 0;
		padding: 16px 24px 8px;
		display: grid;
		grid-template-rows: auto auto minmax(0, 1fr) auto;
	}
	.table-status {
		padding-bottom: 10px;
	}
	.seats,
	.played-cards {
		width: 100%;
	}
	.played-cards {
		min-height: 0;
		padding-top: 14px;
	}
	.played-slot {
		min-height: 0;
		container-type: size;
	}
	.played-card,
	.empty-play {
		width: min(100cqw, calc((100cqh - 8px) * 5 / 7));
		height: auto;
		max-height: 100%;
	}
	.bid-table {
		min-height: 0;
		padding-top: 12px;
		container-type: size;
	}
	.upcard,
	.deck {
		width: auto;
		height: min(100cqh, 190px);
		aspect-ratio: 5/7;
	}
	.trick-result {
		min-height: 40px;
		margin-top: 8px;
	}
	@media (max-width: 700px) {
		.felt-content {
			padding: 10px 12px 4px;
		}
		.table-status {
			gap: 6px;
			padding-bottom: 6px;
			flex-direction: column;
			align-items: stretch;
		}
		.trump {
			display: flex;
			align-items: center;
			justify-content: space-between;
		}
		.trump strong {
			font-size: 21px;
		}
		.trump .eyebrow {
			margin: 0;
			font-size: 13px;
		}
		.trick-score {
			display: flex;
			align-items: center;
			justify-content: space-between;
			font-size: 13px;
		}
		.trick-score strong {
			font-size: 14px;
			white-space: nowrap;
			margin: 0;
			gap: 6px;
		}
		.trick-score b {
			font-size: 20px;
		}
		.seats,
		.played-cards {
			gap: 10px;
		}
		.seat {
			padding: 3px 0 5px;
		}
		.seat-name {
			font-size: 13px;
			min-height: 32px;
			line-height: 1.2;
			align-content: center;
		}
		.seat-detail {
			display: none;
		}
		.dealer {
			width: 16px;
			height: 16px;
			font-size: 10px;
		}
		.played-cards {
			padding-top: 10px;
		}
		.trick-result {
			min-height: 32px;
			font-size: 13px;
			line-height: 1.3;
			margin-top: 4px;
			gap: 8px;
		}
		.trick-result.won {
			font-size: 14px;
		}
		.trick-result button {
			min-height: 44px;
			font-size: 14px;
			padding: 8px 10px;
		}
		.bid-table {
			gap: 16px;
			padding-top: 8px;
		}
		.bid-table h2 {
			font-size: 22px;
			margin: 4px 0;
		}
		.bid-table p {
			font-size: 14px;
			line-height: 1.3;
		}
		.bid-table .eyebrow {
			display: none;
		}
	}
	@media (min-width: 701px) and (max-height: 850px) {
		.felt-content {
			padding: 10px 24px 4px;
		}
		.trump {
			display: flex;
			gap: 12px;
			align-items: center;
		}
		.trump .eyebrow {
			margin: 0;
		}
		.table-status {
			padding-bottom: 4px;
		}
		.trick-score {
			display: flex;
			gap: 12px;
			align-items: center;
		}
		.trick-score strong {
			margin: 0;
		}
		.seat {
			padding: 4px;
		}
		.trick-result {
			margin-top: 6px;
		}
	}
	@media (min-width: 701px) and (max-height: 500px) {
		.table-status {
			flex-direction: column;
			align-items: stretch;
		}
		.trump strong {
			font-size: 22px;
		}
		.trick-score {
			justify-content: space-between;
		}
		.trick-score strong {
			font-size: 14px;
		}
		.seat-name {
			font-size: 13px;
		}
		.seat-detail {
			display: none;
		}
		.seats,
		.played-cards {
			gap: 10px;
		}
		.trick-result {
			font-size: 14px;
		}
	}
	@media (max-width: 700px) {
		.seat-name {
			display: block;
			padding-inline: 2px;
			min-height: 28px;
		}
		.dealer {
			position: absolute;
			top: -5px;
			right: 0;
		}
		.bid-table h2 {
			font-size: 18px;
			line-height: 1.2;
			margin: 0;
		}
		.bid-table p,
		.felt-content:has(.bid-table) .trick-result {
			display: none;
		}
	}
	@media (max-width: 360px) {
		.trump strong { font-size: 18px; }
		.trick-caption { display: none; }
		.short-trick-caption { display: inline; }
		.trick-score { gap: 4px; font-size: 12px; }
		.trick-score strong { font-size: 12px; gap: 4px; }
		.trick-score b { font-size: 18px; }
		.trick-result { font-size: 12px; }
	}
</style>
