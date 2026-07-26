import adapter from '@sveltejs/adapter-vercel';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	// Consult https://svelte.dev/docs/kit/integrations
	// for more information about preprocessors
	preprocess: vitePreprocess(),

	kit: {
		// Deployed to Vercel. The Rivet registry is mounted inside this same
		// deployment at /api/rivet/* (see src/routes/api/rivet/[...all]/+server.ts),
		// so there is exactly one deployable — no separate backend process.
		//
		// Note: this app never terminates a WebSocket. The browser opens its
		// WebSocket against Rivet Cloud; Rivet Cloud then makes ordinary HTTPS
		// requests back to /api/rivet/* here. Nothing about the adapter needs to
		// support connection upgrades.
		adapter: adapter()
	}
};

export default config;
