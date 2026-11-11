<!--
  Play layout — provides the shared Rivet kit for table pages.
-->
<script lang="ts">
	import { page } from '$app/state';

	let { children } = $props();
</script>

{#key page.params.gameId}
	{@render children()}
{/key}
