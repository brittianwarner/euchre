<!--
  Play layout — provides the shared Rivet kit for table pages.
-->
<script lang="ts">
	import { getRivet, rivetContext } from '$lib/client/rivet';

	let { children } = $props();

	rivetContext.set(getRivet());
</script>

{@render children()}
