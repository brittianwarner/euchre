/**
 * Dev-only Vite middleware that serves `/api/rivet/*` with a raw Node body.
 *
 * Vite + SvelteKit was delivering `Content-Length: N` with an empty
 * `request.arrayBuffer()` for Rivet's binary `/start` POSTs (no Content-Type),
 * which surfaces as `serverless start payload too short` and
 * `actor_wake_retries_exceeded`. Handling the path before SvelteKit preserves
 * the bytes. Production (Vercel) uses the SvelteKit route as usual.
 */

import { resolve } from 'node:path';
import type { Plugin, ViteDevServer } from 'vite';
import type { IncomingMessage, ServerResponse } from 'node:http';

async function readBody(req: IncomingMessage): Promise<Buffer> {
	const chunks: Buffer[] = [];
	for await (const chunk of req) {
		chunks.push(typeof chunk === 'string' ? Buffer.from(chunk) : chunk);
	}
	return Buffer.concat(chunks);
}

function nodeHeadersToFetch(req: IncomingMessage): Headers {
	const headers = new Headers();
	for (const [key, value] of Object.entries(req.headers)) {
		if (value === undefined) continue;
		if (Array.isArray(value)) {
			for (const v of value) headers.append(key, v);
		} else {
			headers.set(key, value);
		}
	}
	return headers;
}

async function handleRivet(
	server: ViteDevServer,
	req: IncomingMessage,
	res: ServerResponse
): Promise<void> {
	const { registry } = (await server.ssrLoadModule('/src/lib/actors/registry.ts')) as {
		registry: { handler: (request: Request) => Promise<Response> };
	};

	const body = await readBody(req);
	const host = req.headers.host ?? '127.0.0.1:5173';
	const url = new URL(req.url ?? '/api/rivet', `http://${host}`);
	const method = req.method ?? 'GET';
	const init: RequestInit & { duplex?: 'half' } = {
		method,
		headers: nodeHeadersToFetch(req)
	};
	if (method !== 'GET' && method !== 'HEAD') {
		// Buffer is not a BodyInit; a Uint8Array view over the same bytes is.
		init.body = new Uint8Array(body);
		init.duplex = 'half';
	}

	const response = await registry.handler(new Request(url, init));
	res.statusCode = response.status;
	response.headers.forEach((value, key) => {
		// Node disallows setting hop-by-hop headers this way in some cases.
		if (key.toLowerCase() === 'transfer-encoding') return;
		res.setHeader(key, value);
	});
	const out = Buffer.from(await response.arrayBuffer());
	res.end(out);
}

/** Vite plugin: intercept `/api/rivet` in `configureServer` only. */
export function rivetDevMiddleware(): Plugin {
	return {
		name: 'rivet-dev-middleware',
		configureServer(server) {
			// Dev-only path stays out of production dependency tracing.
			if (!process.env.RIVET_ENDPOINT) process.env.RIVETKIT_STORAGE_PATH ??= resolve('.rivetkit');
			server.middlewares.use((req, res, next) => {
				if (!req.url?.startsWith('/api/rivet')) {
					next();
					return;
				}
				handleRivet(server, req, res).catch((err) => {
					console.error('[rivet-dev-middleware]', err);
					res.statusCode = 500;
					res.end(String(err));
				});
			});
		}
	};
}
