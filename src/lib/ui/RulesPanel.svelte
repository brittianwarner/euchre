<!--
  RulesPanel — the persistent "?". Always available, never a route change, so
  table state is never lost while someone looks something up
  (docs/04-FRONTEND-UX.md §15).

  Context-first: the rule for whatever phase is live right now is written out
  in full at the top, expanded, before anything else. Below it, the rest of
  the rulebook is progressive disclosure — closed `<details>` sections, short
  and scannable, plus a tiny search box that opens whichever ones match.

  Every fact below is sourced from `$lib/euchre` (bower pairing via
  `bowersOf`, scoring via the docs' own table) rather than re-typed from
  memory, so this can't drift from what the engine actually enforces — see
  docs/02-GAME-RULES-ENGINE.md §5 and §10 for the normative source.
-->
<script lang="ts">
	import { SUIT_NAME, bowersOf, cardNameLower, suitOf } from '#lib/euchre/index.ts';
	import type { GamePhase, PublicGameView, Suit } from '#lib/protocol/index.ts';

	interface Props {
		view: PublicGameView | null;
		/** "Take the tour again" inside the panel re-opens `Walkthrough.svelte`. */
		onReplayTour?: () => void;
	}

	let { view, onReplayTour }: Props = $props();

	let dialogEl = $state<HTMLDialogElement | undefined>(undefined);
	let query = $state('');

	function open(): void {
		dialogEl?.showModal();
	}
	function close(): void {
		dialogEl?.close();
	}

	/** The one rule that matters for the phase on screen right now, spoken plainly. */
	function contextNote(v: PublicGameView | null): { title: string; body: string } | null {
		if (v === null || v.status !== 'active') return null;
		const yourTurn = v.turnSeat === v.you;
		const phase: GamePhase = v.phase;

		switch (phase) {
			case 'cutting':
				return {
					title: 'Right now: cutting for the deal',
					body: 'Whoever cuts either "bumps" the deck (cuts it) or says "run \'em" and skips it. It only decides who deals first.'
				};
			case 'bid_round_1': {
				const upSuit = v.upCard !== null ? SUIT_NAME[suitOf(v.upCard)] : 'that suit';
				return {
					title: 'Right now: the up-card is showing',
					body: `${
						yourTurn ? 'You may' : 'Each player in turn may'
					} "order it up" — ${upSuit} become trump, the dealer picks the card into hand and buries one of their own — or pass. Ordering it up alone leaves your partner out of the hand for a bigger score if you win it all.`
				};
			}
			case 'dealer_discard':
				return {
					title: 'Right now: the dealer discards',
					body: 'The dealer has 6 cards after picking up the up-card. One goes face down, unseen, for the rest of the hand — back down to 5.'
				};
			case 'bid_round_2':
				return {
					title: 'Right now: round two',
					body: `The turned-down suit (${v.turnedDownSuit ? SUIT_NAME[v.turnedDownSuit] : 'that suit'}) may not be named again. Each player may name any of the other three suits as trump, or pass. If it comes all the way back around to the dealer with everyone else passed, the dealer must name a suit — no more passing.`
				};
			case 'trick_play': {
				const leftBowerNote =
					v.trump !== null
						? ` ${SUIT_NAME[v.trump]} are trump — the ${cardNameLower(bowersOf(v.trump)[1])} counts as ${SUIT_NAME[v.trump].toLowerCase()} right now, not its printed suit.`
						: '';
				return {
					title: 'Right now: playing a trick',
					body:
						v.trick.ledSuit === null
							? `You're on lead — play anything. Whatever suit you lead, everyone else must follow if they can.${leftBowerNote}`
							: `${SUIT_NAME[v.trick.ledSuit]} was led. Follow it if you're holding one.${leftBowerNote}`
				};
			}
			case 'trick_resolve':
				return {
					title: 'Right now: the trick just finished',
					body: 'Watch the felt — the winner leads next.'
				};
			case 'hand_score':
				return { title: 'Right now: scoring the hand', body: 'The next hand deals in a moment.' };
			case 'game_over':
				return { title: 'Game over', body: 'First to 10 points takes the match.' };
			default:
				return null;
		}
	}

	interface Section {
		readonly id: string;
		readonly title: string;
		readonly body: string;
	}

	/** Live bower example: whatever trump actually is right now, else the classic hearts/diamonds pair. */
	const bowerSuit = $derived<Suit>(view?.trump ?? 'H');
	const bowerPair = $derived(bowersOf(bowerSuit));
	const bowerIsLive = $derived(view?.trump != null);

	const sections = $derived.by((): readonly Section[] => [
		{
			id: 'deck',
			title: 'The deck',
			body: '24 cards: nine, ten, jack, queen, king and ace, in all four suits. Nothing lower than a nine is used.'
		},
		{
			id: 'bowers',
			title: 'Trump and the bowers',
			body:
				`The jack of the trump suit — the right bower — is the single best card in the game. ` +
				`The jack of the OTHER suit that's the same colour becomes trump too, and outranks every ` +
				`other trump except the right bower — that's the left bower. ${
					bowerIsLive
						? `Right now ${SUIT_NAME[bowerSuit]} are trump, so the ${cardNameLower(bowerPair[1])} counts as ${SUIT_NAME[bowerSuit].toLowerCase()}, not its printed suit, for the rest of this hand.`
						: `For example: if ${SUIT_NAME[bowerSuit]} were trump, the ${cardNameLower(bowerPair[1])} would count as ${SUIT_NAME[bowerSuit].toLowerCase()}, not its printed suit, for the rest of that hand.`
				}`
		},
		{
			id: 'follow',
			title: 'Following suit',
			body:
				'If you can play a card of the suit that was led, you must — including the left bower, ' +
				"which counts as trump, not its printed suit, once trump is set. If you're void in the led suit, play anything."
		},
		{
			id: 'bid1',
			title: 'Bidding, round one',
			body:
				'One card is turned face up. Starting left of the dealer, each player may "order it up" — making ' +
				"its suit trump for the hand — or pass. If the dealer's side orders it up, the dealer picks the " +
				'card into hand and discards one card face down.'
		},
		{
			id: 'bid2',
			title: 'Bidding, round two',
			body:
				"If everyone passes, the up-card turns face down and can't be named. Starting left of the dealer " +
				'again, each player may name any of the other three suits as trump, or pass. If it reaches the ' +
				'dealer with all three others having passed, the dealer must name a suit — "stuck".'
		},
		{
			id: 'alone',
			title: 'Going alone',
			body:
				'Whoever calls trump may go alone: their partner sits out entirely and they play all five ' +
				"tricks solo against both opponents. Win all 5 alone and it's worth 4 points instead of 2."
		},
		{
			id: 'scoring',
			title: 'Scoring',
			body:
				'3 or 4 tricks: 1 point. All 5 (a march): 2 points. Alone and all 5: 4 points. The calling team ' +
				'wins fewer than 3 tricks — euchred — and the other team scores 2 instead. First to 10 wins the game.'
		},
		{
			id: 'talk',
			title: "What you'll hear at the table",
			body:
				'"Order it up." / "I assist." (dealer\'s partner) / "I take it." or "Turn it down." (dealer) / ' +
				'"Next." or "crossing the creek" (round two) / "You\'re stuck" (dealer forced to call) / "March!" / "Euchre!"'
		}
	]);

	const filtered = $derived.by((): readonly Section[] => {
		const q = query.trim().toLowerCase();
		if (q.length === 0) return sections;
		return sections.filter(
			(s) => s.title.toLowerCase().includes(q) || s.body.toLowerCase().includes(q)
		);
	});

	const note = $derived(contextNote(view));
</script>

<button
	type="button"
	class="trigger"
	aria-haspopup="dialog"
	aria-label="Rules and help"
	onclick={open}
>
	?
</button>

<dialog
	bind:this={dialogEl}
	class="rules"
	aria-labelledby="rules-title"
	onclick={(e) => {
		if (e.target === dialogEl) close();
	}}
>
	<div class="sheet">
		<header>
			<h2 id="rules-title">How to play</h2>
			<button type="button" class="close" aria-label="Close rules" onclick={close}>×</button>
		</header>

		{#if note !== null}
			<section class="context" aria-live="off">
				<p class="eyebrow">{note.title}</p>
				<p>{note.body}</p>
			</section>
		{/if}

		<label class="search">
			<span class="sr-only">Search the rules</span>
			<input type="search" placeholder="Search the rules…" bind:value={query} />
		</label>

		<div class="sections">
			{#each filtered as s (s.id)}
				<details open={query.trim().length > 0}>
					<summary>{s.title}</summary>
					<p>{s.body}</p>
				</details>
			{:else}
				<p class="empty">No section matches "{query}".</p>
			{/each}
		</div>

		{#if onReplayTour}
			<button
				type="button"
				class="replay"
				onclick={() => {
					close();
					onReplayTour?.();
				}}
			>
				New to euchre? Take the quick tour again
			</button>
		{/if}
	</div>
</dialog>

<style>
	.trigger {
		min-width: 44px;
		min-height: 44px;
		border-radius: 999px;
		border: 1px solid rgba(232, 194, 122, 0.55);
		background: rgba(20, 26, 16, 0.85);
		color: #f4ecd8;
		font: inherit;
		font-weight: 700;
		font-size: 1.1rem;
		cursor: pointer;
		box-shadow: 0 2px 10px rgba(0, 0, 0, 0.4);
	}
	.trigger:hover {
		background: rgba(74, 58, 28, 0.9);
	}

	.rules {
		border: none;
		padding: 0;
		background: transparent;
		max-width: min(34rem, 92vw);
		width: 100%;
		max-height: min(85vh, 44rem);
		color: #f2e8d5;
		font-family: 'Source Serif 4', 'Iowan Old Style', Georgia, serif;
	}
	.rules::backdrop {
		background: rgba(6, 5, 3, 0.68);
	}
	.sheet {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		max-height: min(85vh, 44rem);
		padding: 1.25rem 1.35rem;
		border-radius: 0.9rem;
		border: 1px solid rgba(232, 194, 122, 0.35);
		background: #17130d;
		box-shadow: 0 12px 40px rgba(0, 0, 0, 0.55);
		overflow-y: auto;
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
	}
	h2 {
		margin: 0;
		font-size: 1.3rem;
		color: #f7f1e4;
	}
	.close {
		min-width: 44px;
		min-height: 44px;
		border: none;
		background: transparent;
		color: #c9b89a;
		font-size: 1.4rem;
		line-height: 1;
		cursor: pointer;
	}
	.context {
		margin: 0;
		padding: 0.75rem 0.9rem;
		border-radius: 0.6rem;
		border: 1px solid rgba(232, 194, 122, 0.45);
		background: rgba(232, 194, 122, 0.1);
	}
	.context .eyebrow {
		margin: 0 0 0.3rem;
		font-size: 0.75rem;
		letter-spacing: 0.05em;
		text-transform: uppercase;
		color: #e8c27a;
		font-weight: 700;
	}
	.context p:last-child {
		margin: 0;
		line-height: 1.45;
	}
	.search input {
		width: 100%;
		min-height: 44px;
		padding: 0.5rem 0.7rem;
		border-radius: 0.5rem;
		border: 1px solid rgba(232, 194, 122, 0.3);
		background: #0f0b07;
		color: #f2e8d5;
		font: inherit;
	}
	.sections {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
	}
	details {
		border-radius: 0.5rem;
		border: 1px solid rgba(232, 194, 122, 0.2);
		background: rgba(0, 0, 0, 0.22);
		padding: 0.15rem 0.75rem;
	}
	summary {
		padding: 0.6rem 0;
		font-weight: 600;
		cursor: pointer;
		min-height: 44px;
		display: flex;
		align-items: center;
	}
	details p {
		margin: 0 0 0.75rem;
		line-height: 1.5;
		color: #e8dcc6;
	}
	.empty {
		margin: 0;
		color: #9aab8a;
	}
	.replay {
		min-height: 44px;
		border: 1px dashed rgba(232, 194, 122, 0.4);
		border-radius: 0.5rem;
		background: transparent;
		color: #d8b464;
		font: inherit;
		cursor: pointer;
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		white-space: nowrap;
		border: 0;
	}
</style>
