import devtoolsJson from 'vite-plugin-devtools-json';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import { playwright } from '@vitest/browser-playwright';
import { sveltekit } from '@sveltejs/kit/vite';
import { rivetDevMiddleware } from './vite-plugin-rivet-dev';

export default defineConfig({
	plugins: [
		// Before sveltekit so /api/rivet keeps Rivet's raw start payload in dev.
		rivetDevMiddleware(),
		tailwindcss(),
		sveltekit(),
		devtoolsJson()
	],
	// `@rivetkit/svelte` (file: dist) pulls CJS deps like fast-deep-equal; prebundle
	// them so the browser gets a default export. Keep the package itself external
	// to SSR so Vite serves its prebuilt .js rather than re-parsing sources.
	optimizeDeps: {
		include: ['fast-deep-equal', '@rivetkit/framework-base']
	},
	ssr: {
		noExternal: ['@rivetkit/svelte', '@rivetkit/framework-base']
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
					include: ['src/**/*.svelte.{test,spec}.{js,ts}'],
					exclude: ['src/lib/server/**']
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
