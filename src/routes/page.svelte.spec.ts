import { page } from 'vitest/browser';
import { describe, expect, it } from 'vitest';
import { render } from 'vitest-browser-svelte';
import Page from './+page.svelte';

describe('/+page.svelte', () => {
	it('should render the brand and deal CTA', async () => {
		render(Page);

		await expect.element(page.getByRole('link', { name: 'Deal a hand' })).toBeInTheDocument();
		await expect.element(page.getByText('Kitchen-table euchre against three sharp seats.')).toBeInTheDocument();
	});
});
