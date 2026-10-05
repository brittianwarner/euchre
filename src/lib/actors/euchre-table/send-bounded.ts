/** Queue delivery can wait for a cold actor. Never park the table's watchdog behind it. */
export async function sendBounded(
	handle: {
		send(name: string, body: unknown, options?: { signal: AbortSignal }): Promise<unknown>;
	},
	name: string,
	body: unknown,
	lifecycle: AbortSignal | undefined,
	budgetMs = 1000
): Promise<void> {
	const controller = new AbortController();
	let timer: ReturnType<typeof setTimeout> | undefined;
	let onAbort: () => void = () => {};
	const stopped = new Promise<never>((_, reject) => {
		onAbort = () => {
			controller.abort();
			reject(new Error('Actor queue delivery interrupted'));
		};
		if (lifecycle?.aborted) onAbort();
		else lifecycle?.addEventListener('abort', onAbort, { once: true });
		timer = setTimeout(onAbort, budgetMs);
	});
	try {
		if (lifecycle?.aborted) await stopped;
		await Promise.race([handle.send(name, body, { signal: controller.signal }), stopped]);
	} finally {
		clearTimeout(timer);
		lifecycle?.removeEventListener('abort', onAbort);
	}
}
