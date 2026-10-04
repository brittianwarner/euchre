import { page } from 'vitest/browser';
import { describe, expect, it } from 'vitest';
import { render } from 'vitest-browser-svelte';
import Page from './+page.svelte';

describe('/+page.svelte', () => {
	it('should render the brand and deal CTA', async () => {
		render(Page);

		await expect.element(page.getByRole('link', { name: /Let’s play/ })).toBeInTheDocument();
		await expect
			.element(page.getByRole('heading', { level: 1, name: 'A little nerve. A great hand.' }))
			.toBeInTheDocument();
	});
});
