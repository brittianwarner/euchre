<!--
  Root layout — global styles and favicon. Rivet context lives under /play only
  so `@rivetkit/svelte` is never evaluated during SSR of marketing pages.
-->
<script lang="ts">
	import '../app.css';
	import favicon from '$lib/assets/favicon.svg';

	let { children } = $props();
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<link rel="preconnect" href="https://fonts.googleapis.com" />
	<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin="anonymous" />
	<link
		href="https://fonts.googleapis.com/css2?family=Source+Serif+4:opsz,wght@8..60,500;8..60,600;8..60,700&display=swap"
		rel="stylesheet"
	/>
</svelte:head>

{@render children()}
