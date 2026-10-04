/** Match the incoming host to a configured origin without trusting forwarded headers. */
export function createPublicOriginResolver(origins: readonly string[]) {
	const byHost = new Map<string, string>();
	for (const origin of origins) {
		const url = new URL(origin);
		if (!['http:', 'https:'].includes(url.protocol) || url.origin !== origin) {
			throw new Error('Public origins must be exact HTTP(S) origins');
		}
		if (byHost.has(url.host) && byHost.get(url.host) !== origin) {
			throw new Error('Each public host must have one configured origin');
		}
		byHost.set(url.host, origin);
	}
	return (url: URL): string | undefined => byHost.get(url.host);
}
