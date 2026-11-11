<!--
  Root layout — global styles, the favicon, the web fonts and the nav. The fonts'
  stylesheet is added here, by script, so it never blocks the first paint
  (`src/app.html` preloads it, and carries the social tags). Rivet context lives
  under /play only.
-->
<script lang="ts">
	import '../app.css';
	import favicon from '#lib/assets/favicon.svg';
	import GlobalNav from '#lib/ui/GlobalNav.svelte';

	let { children } = $props();
</script>

<GlobalNav />

<svelte:head>
	<link rel="icon" href={favicon} />
	<link
		href="https://fonts.googleapis.com/css2?family=DM+Sans:wght@400;500;550;600;650;700&family=Source+Serif+4:ital,opsz,wght@0,8..60,400;0,8..60,500;0,8..60,600;0,8..60,700;1,8..60,400&display=swap"
		rel="stylesheet"
	/>
</svelte:head>

{@render children()}
