/** Railway entry point: SvelteKit, authenticated Rivet gateway, and the actor runner. */
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import {
	createRivetGatewayProxy,
	deriveTrustedClientIp,
	type WsConnData
} from './rivet-gateway-proxy.ts';
import { assertRivetEnvSafety, ensureRivetNamespace } from './bootstrap-rivet-namespace.ts';
import { verifyGuest } from '../src/lib/actors/auth/guest.ts';
import { createPublicOriginResolver } from './public-origin.ts';

const env = process.env;
assertRivetEnvSafety({ ...env, selfhosted: true });
const origin = new URL(env.APP_URL ?? '').origin;
if (
	(env.EUCHRE_ACTOR_JWT_SECRET?.length ?? 0) < 32 ||
	(env.EUCHRE_INTERNAL_TOKEN?.length ?? 0) < 32
) {
	throw new Error('Deployment requires independent guest and internal signing secrets');
}
if (!env.RIVET_ENVOY_VERSION) env.RIVET_ENVOY_VERSION = String(Date.now());
const endpoint = env.RIVET_ENDPOINT!;
const namespace = env.RIVET_NAMESPACE!;
const allowedOrigins = (env.ALLOWED_ORIGINS ?? '')
	.split(',')
	.map((s) => s.trim())
	.filter(Boolean);
if (!allowedOrigins.includes(origin)) throw new Error('APP_URL must be in ALLOWED_ORIGINS');
const publicOriginFor = createPublicOriginResolver(allowedOrigins);
await ensureRivetNamespace({
	endpoint,
	token: env.RIVET_TOKEN!,
	namespace,
	runnerPool: env.RIVET_POOL ?? 'default'
});
const { registry } = await import('../src/lib/actors/registry.ts');
await registry.startAndWait();
const { server: kit } = await import(pathToFileURL(resolve('build/server/server.js')).href);
await kit.init({ env, read: (file: string) => Bun.file(resolve('build/client', file)).stream() });

const assets = new Map<string, string>();
for await (const name of new Bun.Glob('**/*').scan({ cwd: 'build/client', onlyFiles: true })) {
	if (!name.endsWith('.map')) assets.set(`/${name}`, resolve('build/client', name));
}
const proxy = createRivetGatewayProxy({ upstreamHttp: endpoint, allowedOrigins });
let draining = false;
const app = Bun.serve<WsConnData>({
	hostname: '::',
	port: Number(env.PORT ?? 3000),
	maxRequestBodySize: 1024 * 1024,
	idleTimeout: 120,
	websocket: proxy.websocket,
	async fetch(request, server) {
		const url = new URL(request.url);
		if (url.pathname === '/health') {
			return draining ? new Response('draining', { status: 503 }) : registry.routes.health();
		}
		const publicOrigin = publicOriginFor(url);
		if (!publicOrigin) return new Response('Unknown host', { status: 421 });
		if (url.pathname === '/api/rivet/metadata') {
			return Response.json(
				{ clientEndpoint: `${publicOrigin}/rivet`, clientNamespace: namespace },
				{ headers: { 'Cache-Control': 'no-store' } }
			);
		}
		// The private envoy owns actors; no public serverless/admin/inspector surfaces.
		if (url.pathname.startsWith('/api/rivet')) return new Response('Not found', { status: 404 });
		if (url.pathname.startsWith('/rivet')) {
			if (url.pathname === '/rivet/metadata') return proxy.handleRequest(request, server);
			// Our browser only connects to a table the server has already created.
			// Never permit gateway create/getOrCreate or arbitrary namespaces/actors.
			const protocols =
				request.headers
					.get('sec-websocket-protocol')
					?.split(',')
					.map((p) => p.trim()) ?? [];
			if (
				url.pathname !== '/rivet/connect' ||
				url.search !== '' ||
				!protocols.includes('rivet_target.actor') ||
				!protocols.some((p) => /^rivet_actor\.[a-z0-9_-]{20,64}$/.test(p)) ||
				!protocols.some((p) => /^rivet_token\.[A-Za-z0-9_.-]+$/.test(p))
			) {
				return new Response('Not found', { status: 404 });
			}
			const cookie = request.headers
				.get('cookie')
				?.split(';')
				.map((s) => s.trim())
				.find((s) => s.startsWith('euchre_guest='))
				?.slice(13);
			try {
				await verifyGuest(cookie ?? '', 'euchre-session');
			} catch {
				return new Response('Unauthorized', { status: 401 });
			}
			// Actor connection params independently enforce the table's owner.
			return proxy.handleRequest(request, server);
		}
		if (request.method === 'GET' || request.method === 'HEAD') {
			let pathname: string;
			try {
				pathname = decodeURIComponent(url.pathname);
			} catch {
				return new Response('Bad path', { status: 400 });
			}
			const asset = assets.get(pathname);
			if (asset) {
				const headers = new Headers({
					'Cache-Control': pathname.includes('/immutable/')
						? 'public,max-age=31536000,immutable'
						: 'public,max-age=86400',
					'Content-Type': Bun.file(asset).type
				});
				const accepted = request.headers.get('accept-encoding') ?? '';
				const encoding =
					accepted.includes('br') && assets.has(`${pathname}.br`)
						? 'br'
						: accepted.includes('gzip') && assets.has(`${pathname}.gz`)
							? 'gzip'
							: null;
				if (encoding) {
					headers.set('Content-Encoding', encoding);
					headers.set('Vary', 'Accept-Encoding');
				}
				const file = Bun.file(encoding ? `${asset}.${encoding === 'gzip' ? 'gz' : 'br'}` : asset);
				return new Response(request.method === 'HEAD' ? null : file, { headers });
			}
		}
		const publicRequest = new Request(`${publicOrigin}${url.pathname}${url.search}`, request);
		return kit.respond(publicRequest, {
			getClientAddress: () =>
				deriveTrustedClientIp(
					request.headers.get('x-forwarded-for'),
					server.requestIP(request)?.address ?? null
				) ?? '127.0.0.1'
		});
	},
	error(error) {
		console.error('Application request failed:', error.message);
		return new Response('Unable to load the table. Please retry.', { status: 500 });
	}
});
console.log(`Euchre ready on port ${app.port}; private Rivet namespace ${namespace}`);
async function shutdown() {
	if (draining) return;
	draining = true;
	await registry.shutdown();
	await app.stop(true);
	process.exit(0);
}
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);
