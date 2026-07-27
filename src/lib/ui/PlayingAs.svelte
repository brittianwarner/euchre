<!--
  PlayingAs — the small "you are here" line for /games and /settings: shows
  which email this browser is remembered as, and a one-tap way out. Posts to
  the page's own `?/signout` action.
-->
<script lang="ts">
	let { email }: { email: string } = $props();
</script>

<div class="playing-as">
	<span>Playing as <strong>{email}</strong></span>
	<form method="POST" action="?/signout">
		<button type="submit">Not you? Switch profile</button>
	</form>
</div>

<style>
	.playing-as {
		margin: 0 0 1.5rem;
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.5rem;
		color: #93876f;
		font-size: 0.85rem;
	}
	strong {
		color: #c9b89a;
		font-weight: 600;
	}
	form {
		display: contents;
	}
	button {
		padding: 0.2rem 0;
		border: none;
		background: none;
		color: #e8c27a;
		font: inherit;
		font-size: 0.85rem;
		text-decoration: underline;
		text-underline-offset: 2px;
		cursor: pointer;
	}
	button:hover {
		color: #f2e8d5;
	}
</style>
