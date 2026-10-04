import { defineConfig } from 'vitest/config';

// Keep the adapter's tests independent of the consuming app's Vite projects.
export default defineConfig({
	test: {
		environment: 'jsdom',
		include: ['src/lib/__tests__/*.test.ts']
	}
});
