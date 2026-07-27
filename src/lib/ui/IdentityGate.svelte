<!--
  IdentityGate — the whole of "sign-in" for /games and /settings: no password,
  no magic link, just "whose kitchen table is this?" so a scrapbook has
  somewhere to live. Posts to the page's own `?/identify` action.
-->
<script lang="ts">
	let {
		heading,
		lede,
		errorMessage = null
	}: {
		heading: string;
		lede: string;
		errorMessage?: string | null;
	} = $props();
</script>

<section class="gate">
	<h1>{heading}</h1>
	<p class="lede">{lede}</p>
	<form method="POST" action="?/identify">
		<label for="gate-email">Your email</label>
		<input
			id="gate-email"
			name="email"
			type="email"
			inputmode="email"
			autocomplete="email"
			placeholder="you@example.com"
			required
		/>
		<button type="submit">Remember me here</button>
	</form>
	{#if errorMessage}
		<p class="error" role="alert">{errorMessage}</p>
	{/if}
	<p class="fine-print">
		No password, no verification email — this just labels the games this browser remembers as
		yours. Nothing is sent anywhere; it only tells this app which scrapbook to open.
	</p>
</section>

<style>
	.gate {
		max-width: 28rem;
		margin: 3.5rem auto;
		padding: 1.75rem;
		border-radius: 0.85rem;
		background: rgba(15, 20, 14, 0.6);
		border: 1px solid rgba(232, 194, 122, 0.22);
	}
	h1 {
		margin: 0 0 0.5rem;
		font-size: clamp(1.4rem, 4vw, 1.8rem);
		color: #f2e8d5;
	}
	.lede {
		margin: 0 0 1.25rem;
		color: #c9b89a;
		line-height: 1.5;
	}
	form {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
	}
	label {
		width: 100%;
		font-size: 0.85rem;
		color: #c9b89a;
	}
	input {
		flex: 1 1 14rem;
		min-height: 44px;
		padding: 0 0.85rem;
		border-radius: 0.5rem;
		border: 1px solid #6a5638;
		background: #241b10;
		color: #f2e8d5;
		font: inherit;
		font-size: 1rem;
	}
	input:focus-visible {
		outline: 2px solid #e8c27a;
		outline-offset: 1px;
	}
	button {
		min-height: 44px;
		padding: 0 1.1rem;
		border-radius: 0.5rem;
		border: 1px solid #4a7f44;
		background: #3d6b38;
		color: #f4f7e8;
		font: inherit;
		font-weight: 600;
		cursor: pointer;
	}
	button:hover {
		background: #4a7f44;
	}
	.error {
		margin: 0.75rem 0 0;
		color: #e0a06e;
		font-size: 0.9rem;
	}
	.fine-print {
		margin: 1.1rem 0 0;
		color: #93876f;
		font-size: 0.8rem;
		line-height: 1.5;
	}
</style>
