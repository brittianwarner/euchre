<!--
  Landing — brand-first entry into a local M2 game (heuristic opponents).
-->
<script lang="ts">
</script>

<svelte:head>
	<title>Euchre</title>
</svelte:head>

<main class="hero">
	<div class="copy">
		<p class="brand">Euchre</p>
		<h1>Kitchen-table euchre against three sharp seats.</h1>
		<p class="lede">Bid, bury, and march to ten. Local play uses heuristic opponents — no account needed yet.</p>
		<p class="cta">
			<a href="/play">Deal a hand</a>
		</p>
	</div>
</main>

<style>
	.hero {
		min-height: 100dvh;
		display: grid;
		place-items: center;
		padding: 2rem;
		background:
			radial-gradient(ellipse at 20% 10%, rgba(80, 120, 60, 0.35), transparent 50%),
			radial-gradient(ellipse at 90% 80%, rgba(90, 60, 30, 0.35), transparent 45%),
			linear-gradient(160deg, #1a140e, #102016 55%, #1c1610);
		color: #f2e8d5;
		font-family: 'Source Serif 4', 'Iowan Old Style', Georgia, serif;
	}
	.copy {
		max-width: 34rem;
	}
	.brand {
		margin: 0 0 0.75rem;
		font-size: clamp(3.2rem, 10vw, 5rem);
		line-height: 0.95;
		letter-spacing: -0.03em;
	}
	h1 {
		margin: 0 0 0.75rem;
		font-size: clamp(1.35rem, 3.5vw, 1.75rem);
		font-weight: 500;
		color: #e8dcc6;
	}
	.lede {
		margin: 0 0 1.5rem;
		color: #c9b89a;
		line-height: 1.45;
	}
	.cta a {
		display: inline-block;
		padding: 0.8rem 1.25rem;
		border-radius: 0.45rem;
		background: #3d6b38;
		color: #f4f7e8;
		text-decoration: none;
		font-weight: 600;
		transition: transform 160ms ease, background 160ms ease;
	}
	.cta a:hover {
		transform: translateY(-2px);
		background: #4a7f44;
	}
</style>
