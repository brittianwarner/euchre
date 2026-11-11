import { DECK } from '#lib/euchre/cards.ts';

let warmed = false;

/** Warm public deck artwork while connecting/bidding, before opponents play. */
export function warmCardArt(): void {
	if (warmed) return;
	warmed = true;
	for (const card of DECK) {
		const image = new Image();
		image.decoding = 'async';
		image.src = `/art/cards/${card}.png`;
		void image.decode().catch(() => {
			// A visible card retries normally if an early background request failed.
		});
	}
}
