<script lang="ts">
	let stick = $state(false);
	let natural = $state(false);
</script>

<svelte:head><title>Choose your table · Euchre</title></svelte:head>
<main>
	<a href="/games">← Your saved games</a>
	<h1>Your table. Your rules.</h1>
	<p>Traditional 24-card euchre. You and your partner play to 10.</p>
	<form action="/play" method="GET">
		<label
			><input type="checkbox" bind:checked={stick} /> Stick the dealer
			<small>After three passes in round two, the dealer must name trump.</small></label
		>
		<label
			><input type="checkbox" bind:checked={natural} /> Require a natural trump to call
			<small
				>The left bower alone does not qualify. The dealer may count the up-card when picking it up.</small
			></label
		>
		{#if stick && natural}<p>
				The stuck dealer may call without a natural trump, so the hand can always proceed.
			</p>{/if}
		<input type="hidden" name="stick" value={String(stick)} />
		<input type="hidden" name="natural" value={String(natural)} />
		<button>Deal me in →</button>
	</form>
	<p>Take your time. Your turn waits for you, and your table is saved in this browser.</p>
</main>

<style>
	main {
		max-width: 640px;
		margin: auto;
		padding: 48px 24px;
		color: #233e32;
		background: #f4f3e9;
		border-radius: 16px;
	}
	h1 {
		font: 500 38px/1.2 var(--font-serif);
		margin-block: 24px;
	}
	p {
		font-size: 18px;
		line-height: 1.5;
	}
	label {
		display: grid;
		grid-template-columns: 28px 1fr;
		gap: 12px;
		padding: 20px 0;
		font-size: 20px;
		cursor: pointer;
	}
	input[type='checkbox'] {
		width: 24px;
		height: 24px;
		accent-color: #285641;
	}
	small {
		grid-column: 2;
		font-size: 16px;
		line-height: 1.5;
	}
	button {
		width: 100%;
		min-height: 56px;
		background: #285641;
		color: white;
		border: 0;
		border-radius: 8px;
		font: 600 20px var(--font-sans);
		cursor: pointer;
	}
</style>
