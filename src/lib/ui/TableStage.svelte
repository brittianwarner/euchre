<script lang="ts">
	import { untrack } from 'svelte';
	import { cardName, SUIT_NAME } from '#lib/euchre/index.ts';
	import type { CardId, PublicGameView, Seat } from '#lib/protocol/index.ts';
	import { reducedMotion } from '#lib/three/layout/reducedMotion.svelte.ts';
	import { playPlace, playTrickTake } from './sound';
	import { TABLE_MOTION, seatName } from './table-presentation';

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
	let surface: HTMLElement | undefined;
	function attachElement(node: HTMLElement) {
		surface = node;
		return () => {
			surface = undefined;
		};
	}
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
	const leadSeat = $derived(
		view.trick.plays[0]?.seat ?? (view.phase === 'trick_play' ? view.turnSeat : null)
	);
	function bidFor(seat: Seat) {
		const bids = view.phase === 'bid_round_2' ? view.bids.slice(4) : view.bids;
		return [...bids].reverse().find((b) => b.seat === seat);
	}
	const seen = new Set(
		untrack(() => view.trick.plays.map((play) => `${view.handNo}:${view.trick.index}:${play.card}`))
	);
	const animations = new Set<Animation>();
	const suitGlyph = { H: '♥', D: '♦', S: '♠', C: '♣' };

	function arrive(node: HTMLImageElement) {
		const context = untrack(() => ({ you: view.you, origin, reduced: reducedMotion.enabled }));
		const key = node.dataset.playKey!;
		if (seen.has(key)) return;
		seen.add(key);
		if (seen.size > 40) seen.delete(seen.values().next().value!);
		let disposed = false;
		let animation: Animation | undefined;
		// Preserve the slot but never animate a blank, undecoded card face.
		node.style.visibility = 'hidden';
		void node.decode().then(showCard, showCard);
		function showCard() {
			if (disposed) return;
			node.style.visibility = '';
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
			const anchor = surface
				?.querySelector(`[data-seat-anchor="${seat}"]`)
				?.getBoundingClientRect();
			const dx = fromHand
				? fromHand.left - rect.left
				: anchor
					? anchor.left + anchor.width / 2 - rect.left - rect.width / 2
					: 0;
			const dy = fromHand
				? fromHand.top - rect.top
				: anchor
					? anchor.top + anchor.height / 2 - rect.top - rect.height / 2
					: seat === context.you
						? 130
						: -95;
			const scale = fromHand ? fromHand.width / rect.width : 0.75;
			animation = node.animate(
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
				animations.delete(animation!);
				playPlace();
			};
		}
		return () => {
			disposed = true;
			animation?.cancel();
			if (animation) animations.delete(animation);
		};
	}

	// Only an up card already visible at this table can travel to the dealer.
	// A reconnect in the discard phase does not replay the pickup.
	function pickup(node: HTMLElement, dealt: { handNo: number; dealer: Seat }) {
		const ordered =
			view.handNo === dealt.handNo && view.bids.some((bid) => bid.move.t === 'orderUp');
		const target = surface?.querySelector(`[data-seat-anchor="${dealt.dealer}"]`);
		if (!ordered || !target || reducedMotion.enabled) return { duration: 0 };
		const from = node.getBoundingClientRect();
		const to = target.getBoundingClientRect();
		const dx = to.left + to.width / 2 - from.left - from.width / 2;
		const dy = to.top + to.height / 2 - from.top - from.height / 2;
		return {
			duration: 850,
			css: (t: number) => {
				const progress = 1 - t;
				const eased = progress * progress * (3 - 2 * progress);
				return `position:fixed;left:${from.left}px;top:${from.top}px;width:${from.width}px;height:${from.height}px;max-height:none;margin:0;z-index:20;pointer-events:none;transform:translate(${dx * eased}px,${dy * eased - Math.sin(progress * Math.PI) * 24}px) scale(${1 - eased * 0.72}) rotate(${eased * 8}deg);opacity:${Math.min(1, t * 5)};`;
			}
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
	{@attach attachElement}
	aria-label="The table"
	class:collected={collectedKey === trickKey}
>
	<div class="felt-content">
		<div class="table-status">
			<div class="trump">
				<span class="eyebrow">Hand {view.handNo + 1}</span><strong
					>{#if view.trump}<span aria-hidden="true">{suitGlyph[view.trump]}</span>
						{SUIT_NAME[view.trump]} are trump{:else}Choosing trump{/if}</strong
				>
				{#if view.trick.ledSuit}<span
						class="led-marker"
						aria-label={`${SUIT_NAME[view.trick.ledSuit]} led`}
						>↗ {suitGlyph[view.trick.ledSuit]} <small>Led</small></span
					>{/if}
			</div>
			{#if view.aloneSeat !== null}<span class="loner-marker"
					>★ {seatName(view.aloneSeat, view.you)} alone</span
				>{/if}

			<div class="trick-score">
				<span class="trick-caption">Tricks this hand</span><span class="mobile-led"
					>{#if view.trick.ledSuit}Led {suitGlyph[view.trick.ledSuit]}{:else}Tricks{/if}</span
				><span class="short-trick-caption">Tricks</span><strong
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
						><svg
							class="player-marker"
							viewBox="0 0 32 32"
							width="28"
							height="28"
							aria-hidden="true"
							><circle cx="16" cy="16" r="15" fill="currentColor" opacity=".18" /><circle
								cx="16"
								cy="11"
								r="5"
								fill="currentColor"
							/><path d="M6 27c0-8 4-11 10-11s10 3 10 11" fill="currentColor" /></svg
						><span class="full-seat-name">{seatName(seat, view.you)}</span><span
							class="short-seat-name"
							>{['You', 'Left', 'Partner', 'Right'][(seat - view.you + 4) % 4]}</span
						>
						{#if seat === view.makerSeat && view.trump}<span
								class="maker-marker"
								title="Named trump"
								aria-label="Named trump">{suitGlyph[view.trump]}</span
							>{/if}
						{#if seat === leadSeat}<span title="Leads this trick" aria-label="Leads this trick"
								>↗</span
							>{/if}
						{#if seat === view.dealerSeat}<span class="dealer" title="Dealer"
								>D<span class="sr-only">ealer</span></span
							>{/if}</span
					>
					<span class="seat-detail"
						>{view.sittingSeat === seat
							? 'Sitting out · partner alone'
							: view.turnSeat === seat
								? seat === view.you
									? 'Your turn'
									: 'Thinking…'
								: bidding && bidFor(seat)?.move.t === 'pass'
									? 'Passed'
									: `${view.handCounts[seat]} ${view.handCounts[seat] === 1 ? 'card' : 'cards'}`}</span
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
						out:pickup|global={{ handNo: view.handNo, dealer: view.dealerSeat }}
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
								? `${seatName(view.dealerSeat, view.you)} turned down ${view.upCard ? cardName(view.upCard) : 'the up card'}.`
								: view.phase === 'dealer_discard'
									? `${seatName(view.dealerSeat, view.you)} picked up ${view.upCard ? cardName(view.upCard) : 'the up card'}.`
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
							{/key}<span class="played-owner" aria-hidden="true"
								>{seat === view.you
									? 'You'
									: (seat - view.you + 4) % 4 === 2
										? 'Partner'
										: (seat - view.you + 4) % 4 === 1
											? 'Left'
											: 'Right'}</span
							>{:else}<span class="empty-play"
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
					<span class="next-lead"
						>{view.trick.index < 4
							? view.reviewTricks
								? 'Press Continue before the next lead.'
								: `${seatName(winner, view.you)} ${winner === view.you ? 'lead' : 'leads'} next.`
							: 'That’s the last trick of this hand.'}</span
					></span
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
	.player-marker {
		display: inline-block;
		color: #b9d2c1;
		font-size: 16px;
	}
	.partner .player-marker {
		color: #e4cf91;
	}
	.maker-marker {
		background: #f4ead0;
		color: #243d31;
		border-radius: 50%;
		padding: 2px 6px;
	}
	.loner-marker {
		font-weight: 700;
		color: #f2d887;
		white-space: nowrap;
	}
	.led-marker {
		color: #fff8dc;
		font-size: 26px;
		white-space: nowrap;
	}
	.led-marker small {
		font: 500 13px var(--font-sans);
	}

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
	.short-trick-caption {
		display: none;
	}
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
			position: relative;
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
			display: block;
			font-size: 11px;
			min-height: 14px;
			margin-top: 0;
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
			display: block;
			font-size: 11px;
			min-height: 14px;
			margin-top: 0;
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
	@media (max-width: 700px) {
		.player-marker {
			display: none;
		}
		.loner-marker {
			font-size: 13px;
		}
		.led-marker {
			position: absolute;
			right: 0;
			top: -3px;
			font-size: 20px;
		}
		.led-marker small {
			display: none;
		}
		.table-status:has(.led-marker) .trump {
			padding-right: 28px;
		}
	}
	@media (max-width: 360px) {
		.trump strong {
			font-size: 18px;
		}
		.trick-caption {
			display: none;
		}
		.short-trick-caption {
			display: inline;
		}
		.trick-score {
			gap: 4px;
			font-size: 12px;
		}
		.trick-score strong {
			font-size: 12px;
			gap: 4px;
		}
		.trick-score b {
			font-size: 18px;
		}
		.trick-result {
			font-size: 12px;
		}
	}

	@media (min-width: 701px) and (min-height: 501px) {
		.felt-content {
			grid-template-rows: auto 0 minmax(0, 1fr) auto;
			padding: 18px;
		}
		.table-status {
			flex-wrap: wrap;
			gap: 10px;
			padding-bottom: 12px;
		}
		.trump strong {
			font-size: 26px;
		}
		.seats {
			position: absolute;
			inset: 100px 14px 80px;
			display: block;
			max-width: none;
			pointer-events: none;
		}
		.seat {
			position: absolute;
			width: 160px;
			border: 0;
			background: none;
			padding: 0;
			transform: translateX(-50%);
		}
		.seat:nth-child(1) {
			left: 14%;
			top: 20%;
		}
		.seat:nth-child(2) {
			left: 50%;
			top: 0;
		}
		.seat:nth-child(3) {
			left: 86%;
			top: 20%;
		}
		.seat:nth-child(4) {
			left: 50%;
			bottom: 0;
		}
		.seat-name {
			font-size: 15px;
			flex-wrap: wrap;
			gap: 4px;
		}
		.seat.active .seat-name {
			color: #ffe5a4;
		}
		.seat-detail {
			font-size: 13px;
			min-height: 18px;
			margin: 2px 0;
		}
		.dealer {
			width: 18px;
			height: 18px;
			font-size: 11px;
		}
		.taken-stack {
			right: 4px;
			bottom: -18px;
		}
		.played-cards {
			grid-row: 3;
			display: block;
			position: relative;
			padding: 0;
		}
		.played-slot {
			position: absolute;
			width: 24%;
			height: 38%;
			transform: translate(-50%, -50%);
		}
		.played-slot:nth-child(1) {
			left: 18%;
			top: 60%;
		}
		.played-slot:nth-child(2) {
			left: 50%;
			top: 28%;
		}
		.played-slot:nth-child(3) {
			left: 82%;
			top: 60%;
		}
		.played-slot:nth-child(4) {
			left: 50%;
			top: 65%;
		}
		.bid-table {
			grid-row: 3;
			padding: 60px 0;
			gap: 12px;
			flex-direction: column;
		}
		.bid-table h2 {
			font-size: 23px;
		}
		.bid-table p {
			display: none;
		}
		.trick-result {
			grid-row: 4;
			font-size: 15px;
			min-height: 44px;
		}
	}
	.mobile-led {
		display: none;
	}
	@media (max-width: 700px) {
		.led-marker,
		.trick-caption,
		.short-trick-caption {
			display: none;
		}
		.table-status:has(.led-marker) .trump {
			padding-right: 0;
		}
		.mobile-led {
			display: inline;
			white-space: nowrap;
			font-size: 15px;
		}
	}
	.short-seat-name {
		display: none;
	}
	@media (max-width: 360px) {
		.full-seat-name,
		.next-lead {
			display: none;
		}
		.short-seat-name {
			display: inline;
		}
		.seat-name {
			min-height: 22px;
		}
	}
	/* The page supplies one continuous felt surface beneath table and hand. */
	.felt {
		background: transparent;
		border: 0;
		border-radius: 0;
		box-shadow: none;
	}
	@media (min-width: 701px) and (min-height: 501px) {
		.table-status {
			align-items: flex-start;
			flex-wrap: nowrap;
		}
		.trump {
			display: flex;
			flex-wrap: wrap;
			max-width: 42%;
			align-items: center;
			gap: 6px 12px;
		}
		.trump .eyebrow {
			width: 100%;
			margin: 0;
		}
		.led-marker {
			font-size: 18px;
			padding: 4px 10px;
			border-radius: 20px;
			background: #ffffff0c;
		}
		.seat {
			width: 180px;
		}
		.seat-name {
			font-size: 17px;
		}
		.seat-detail {
			color: #d3decf;
			font-size: 14px;
		}

		.felt-content {
			padding: 8px 20px 0;
		}
		.seats {
			inset: 60px 14px 50px;
		}
		.seat:nth-child(1) {
			left: 13%;
			top: 38%;
		}
		.seat:nth-child(3) {
			left: 87%;
			top: 38%;
		}
		.seat:nth-child(2) {
			top: -36px;
		}
		.seat:nth-child(4) {
			width: 120px;
			bottom: -12px;
		}
		.felt-content:has(.played-cards) .seat:nth-child(4) {
			visibility: hidden;
		}

		.played-slot {
			width: 22%;
			height: 80%;
		}
		.played-slot:nth-child(1) {
			left: 28%;
			top: 50%;
			transform: translate(-50%, -50%) rotate(-7deg);
		}
		.played-slot:nth-child(2) {
			left: 43%;
			top: 35%;
			transform: translate(-50%, -50%) rotate(3deg);
		}
		.played-slot:nth-child(3) {
			left: 72%;
			top: 50%;
			transform: translate(-50%, -50%) rotate(7deg);
		}
		.played-slot:nth-child(4) {
			left: 57%;
			top: 68%;
			transform: translate(-50%, -50%) rotate(-3deg);
		}

		.bid-table {
			flex-direction: row;
			padding: 14px 0;
			gap: 28px;
		}
	}

	@media (orientation: landscape) and (max-height: 500px) {
		.felt-content {
			padding: 2px 10px 0;
		}
		.table-status {
			flex-direction: row;
			align-items: center;
			padding-bottom: 2px;
			gap: 8px;
		}
		.trump {
			gap: 8px;
		}
		.trump strong {
			font-size: 18px;
		}
		.trump .eyebrow {
			display: none;
		}
		.trick-score {
			gap: 8px;
		}
		.mobile-led {
			display: none;
		}
		.trick-score strong {
			font-size: 12px;
		}
		.seat {
			padding: 0 0 2px;
		}
		.seat-name {
			min-height: 18px;
			font-size: 12px;
		}
		.seat-detail {
			font-size: 10px;
			min-height: 12px;
		}
		.played-cards {
			padding-top: 4px;
		}
		.trick-result {
			min-height: 24px;
			margin-top: 2px;
			font-size: 12px;
		}
		.trick-result.won {
			font-size: 12px;
		}
		.trick-result button {
			min-height: 44px;
			padding: 5px 10px;
		}
		.bid-table {
			flex-direction: row;
			gap: 20px;
			padding-top: 4px;
		}
		.bid-table h2 {
			font-size: 18px;
		}
		.bid-table p,
		.bid-table .eyebrow {
			display: none;
		}
	}

	.played-owner {
		display: none;
		position: absolute;
		bottom: -16px;
		left: 50%;
		transform: translateX(-50%);
		font-size: 12px;
		font-weight: 650;
		line-height: 1;
		color: #f3ead4;
		white-space: nowrap;
	}
	@media (max-width: 700px), (max-height: 500px) {
		.played-slot {
			position: relative;
		}
		.played-cards {
			padding-bottom: 16px;
		}
		.played-owner {
			display: block;
			bottom: -13px;
			font-size: 11px;
		}
	}
	@container game (min-width: 701px) and (max-width: 1100px) {
		.trick-score {
			display: block;
			text-align: right;
		}
		.trick-score strong {
			display: block;
			margin-top: 4px;
		}
		.seat:nth-child(2) {
			top: 8px;
		}
	}
</style>
