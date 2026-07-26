<!--
  Live euchre table — DOM M2 client.
  Connects to euchreTable via @rivetkit/svelte and renders HandA11y + bid UI.
-->
<script lang="ts">
	import { withActorParams } from '@rivetkit/svelte';
	import { rivetContext } from '$lib/client/rivet';
	import { TableStore, type TableActorHandle } from '$lib/game/table.svelte';
	import BidPanel from '$lib/ui/BidPanel.svelte';
	import DiscardPanel from '$lib/ui/DiscardPanel.svelte';
	import HandA11y from '$lib/ui/HandA11y.svelte';
	import ScoreBoard from '$lib/ui/ScoreBoard.svelte';
	import TrickView from '$lib/ui/TrickView.svelte';
	import type { LegalMoveId, SyncEvent } from '$lib/protocol';

	let { data } = $props();

	const { useActor } = rivetContext.get();
	const store = new TableStore();

	const table = useActor(
		withActorParams(
			() => ({
				name: 'euchreTable' as const,
				key: ['table', data.gameId]
			}),
			() => ({ token: data.token })
		)
	);

	// onEvent must be registered during component init — not inside $effect.
	table.onEvent('sync', (payload: SyncEvent) => {
		store.applySync(payload);
	});

	store.bind(table as unknown as TableActorHandle);

	$effect(() => {
		if (table.isConnected) {
			void store.resync();
		}
	});

	async function onPlay(moveId: LegalMoveId) {
		await store.play(moveId);
	}
</script>

<svelte:head>
	<title>Euchre — play</title>
</svelte:head>

<main class="table">
	{#if !store.view}
		<p class="loading">
			{table.isConnected ? 'Dealing…' : 'Connecting to the table…'}
		</p>
		{#if table.lastError}
			<p class="err">{String(table.lastError)}</p>
		{/if}
	{:else}
		<ScoreBoard view={store.view} />
		<TrickView view={store.view} />
		{#if store.error}
			<p class="err" role="alert">{store.error}</p>
		{/if}
		<BidPanel view={store.view} disabled={store.submitting} {onPlay} />
		<DiscardPanel view={store.view} disabled={store.submitting} {onPlay} />
		<HandA11y view={store.view} disabled={store.submitting} {onPlay} />
		<p class="footer">
			<a href="/play">Deal a new game</a>
			· game {data.gameId.slice(0, 8)}
		</p>
	{/if}
</main>

<style>
	.table {
		min-height: 100dvh;
		background:
			radial-gradient(ellipse at top, #2a3d22 0%, transparent 55%),
			linear-gradient(165deg, #1a140e 0%, #0f1a12 45%, #1c1610 100%);
		color: #f2e8d5;
		font-family: 'Source Serif 4', 'Iowan Old Style', Georgia, serif;
	}
	.loading,
	.err,
	.footer {
		padding: 1.25rem;
		margin: 0;
	}
	.err {
		color: #e8a090;
	}
	.footer {
		color: #8a7a62;
		font-size: 0.85rem;
	}
	.footer a {
		color: #d4b57a;
	}
</style>
