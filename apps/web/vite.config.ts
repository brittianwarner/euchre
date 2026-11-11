import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import devtoolsJson from 'vite-plugin-devtools-json';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import { playwright } from '@vitest/browser-playwright';
import { sveltekit } from '@sveltejs/kit/vite';

/**
 * The euchre-server that owns `/api/*`, the `/rivet` gateway and the desktop
 * `/download/*` routes. `vite dev` and `vite preview` proxy all three to it, so
 * the browser stays same-origin, exactly as when the Rust server serves this
 * build itself. The proxy keeps the browser's `Host` (no `changeOrigin`), so the
 * server must list this origin in `ALLOWED_ORIGINS` (see README.md,
 * "Development").
 */
const EUCHRE_SERVER = process.env.EUCHRE_SERVER_URL ?? 'http://127.0.0.1:3000';

const proxy = {
	'/api': { target: EUCHRE_SERVER },
	'/rivet': { target: EUCHRE_SERVER, ws: true },
	'/download': { target: EUCHRE_SERVER }
};

export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			// D4: a static SPA. The Rust server serves `build/client` and answers
			// every extensionless miss with `index.html`. `precompress` writes the
			// `.br` / `.gz` siblings the server sends to clients that accept them,
			// as TS adapter-node did by default.
			adapter: adapter({
				pages: 'build/client',
				assets: 'build/client',
				fallback: 'index.html',
				precompress: true
			}),
			preprocess: vitePreprocess()
		}),
		devtoolsJson()
	],
	server: { proxy },
	preview: { proxy },
	// `@rivetkit/svelte` pulls CJS deps like fast-deep-equal; prebundle them so the
	// browser gets a default export.
	optimizeDeps: {
		include: ['@rivetkit/svelte > fast-deep-equal']
	},
	ssr: {
		noExternal: ['@rivetkit/svelte']
	},
	test: {
		expect: { requireAssertions: true },
		projects: [
			{
				extends: './vite.config.ts',
				test: {
					name: 'client',
					browser: {
						enabled: true,
						provider: playwright(),
						instances: [{ browser: 'chromium', headless: true }]
					},
					include: ['src/**/*.svelte.{test,spec}.{js,ts}']
				}
			},
			{
				extends: './vite.config.ts',
				test: {
					name: 'server',
					environment: 'node',
					include: ['src/**/*.{test,spec}.{js,ts}'],
					exclude: ['src/**/*.svelte.{test,spec}.{js,ts}']
				}
			}
		]
	}
});
