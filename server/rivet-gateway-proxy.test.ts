/**
 * Tests for the Rivet gateway proxy. Classifier/origin tests are pure;
 * relay tests spin a mock upstream engine (Bun.serve WS echo + HTTP
 * recorder) and the proxy on ephemeral ports. Live protocol verification
 * against the real engine is scripts/rivet-selfhost-verify.ts's job.
 */
import { afterAll, describe, expect, test } from 'bun:test';
import { createClient } from 'rivetkit/client';
import {
	capWsCloseReason,
	classifyProxyPath,
	createRivetGatewayProxy,
	deriveTrustedClientIp,
	isOriginAllowed,
	toWebSocketBase,
	type WsConnData
} from './rivet-gateway-proxy.js';

const quietLog = { info() {}, warn() {}, error() {} };

// ---------------------------------------------------------------------------
// Pure: guard rate-limit key derivation
// ---------------------------------------------------------------------------

describe('deriveTrustedClientIp', () => {
	test('takes the LAST X-Forwarded-For entry (nearest trusted hop)', () => {
		// A client-forged first entry must never win — the Guard would key its
		// rate-limit bucket on it.
		expect(deriveTrustedClientIp('6.6.6.6, 203.0.113.9', '10.0.0.1')).toBe('203.0.113.9');
		expect(deriveTrustedClientIp('203.0.113.9', '10.0.0.1')).toBe('203.0.113.9');
		expect(deriveTrustedClientIp('2001:db8::1, 2001:db8::2', '10.0.0.1')).toBe('2001:db8::2');
	});

	test('falls back to the socket IP when the header is absent or garbage', () => {
		expect(deriveTrustedClientIp(null, '10.0.0.1')).toBe('10.0.0.1');
		expect(deriveTrustedClientIp('not-an-ip', '10.0.0.1')).toBe('10.0.0.1');
		expect(deriveTrustedClientIp('a, b, still-not-an-ip', '10.0.0.1')).toBe('10.0.0.1');
		expect(deriveTrustedClientIp('', '10.0.0.1')).toBe('10.0.0.1');
	});

	test('null when nothing trustworthy exists', () => {
		expect(deriveTrustedClientIp(null, null)).toBeNull();
		expect(deriveTrustedClientIp('garbage', 'also-garbage')).toBeNull();
	});

	test('tolerates ip:port and [v6]:port entry shapes (edge format drift)', () => {
		// If an edge ever appends port-suffixed entries, naive isIP fails and
		// EVERY browser silently collapses into the shared socket-IP bucket —
		// the pre-incident topology. Port-strip first.
		expect(deriveTrustedClientIp('6.6.6.6, 203.0.113.9:8080', '10.0.0.1')).toBe('203.0.113.9');
		expect(deriveTrustedClientIp('6.6.6.6, [2001:db8::2]:443', '10.0.0.1')).toBe('2001:db8::2');
		expect(deriveTrustedClientIp('[2001:db8::7]', '10.0.0.1')).toBe('2001:db8::7');
		// A bare IPv6 has colons but is NOT ip:port — must not be mangled.
		expect(deriveTrustedClientIp('2001:db8::9', '10.0.0.1')).toBe('2001:db8::9');
		// Unresolvable even after stripping → socket fallback, not a mangled IP.
		expect(deriveTrustedClientIp('[not-an-ip]:443', '10.0.0.1')).toBe('10.0.0.1');
	});
});

describe('capWsCloseReason', () => {
	test('caps by UTF-8 BYTES, never splitting a multibyte character', () => {
		// 107 'é' = 123 UTF-16 units but 230 UTF-8 bytes — a plain
		// .slice(0, 123) passes it through and the close frame byte-truncates
		// mid-character (client sees 1007 "invalid UTF8", no reason) or throws
		// on the client-WebSocket leg (verified Bun 1.4.0).
		const multibyte = 'é'.repeat(107);
		const capped = capWsCloseReason(multibyte);
		expect(new TextEncoder().encode(capped).length).toBeLessThanOrEqual(123);
		// No partial trailing sequence — round-trips cleanly.
		expect(capped).toBe('é'.repeat(61));
	});

	test('ASCII within the limit passes through untouched', () => {
		expect(capWsCloseReason('upstream error: guard.rate_limit#abc')).toBe(
			'upstream error: guard.rate_limit#abc'
		);
		const long = 'x'.repeat(200);
		expect(capWsCloseReason(long)).toBe('x'.repeat(123));
	});
});

// ---------------------------------------------------------------------------
// Pure: allowlist classifier
// ---------------------------------------------------------------------------

describe('classifyProxyPath', () => {
	const ws = (p: string, m = 'GET', up = true) => classifyProxyPath(p, m, up).kind;
	const http = (p: string, m = 'GET') => classifyProxyPath(p, m, false).kind;

	test('allows the client wire contract', () => {
		expect(http('/rivet/metadata')).toBe('metadata');
		expect(ws('/rivet/connect')).toBe('ws');
		expect(http('/rivet/connect')).toBe('deny');
		expect(ws('/rivet/gateway/user/connect')).toBe('ws');
		expect(ws('/rivet/gateway/actor-abc@tok123/connect')).toBe('ws');
		expect(ws('/rivet/gateway/user/websocket')).toBe('ws');
		expect(ws('/rivet/gateway/user/websocket/custom/path')).toBe('ws');
		expect(http('/rivet/gateway/user/action/getProfile', 'POST')).toBe('http');
		expect(http('/rivet/gateway/user/queue/jobs', 'POST')).toBe('http');
		expect(http('/rivet/gateway/user/request', 'POST')).toBe('http');
		expect(http('/rivet/gateway/user/request/custom/path', 'GET')).toBe('http');
		// CORS preflight on allowlisted paths rides through to the engine.
		expect(http('/rivet/gateway/user/action/getProfile', 'OPTIONS')).toBe('http');
	});

	test('denies every admin surface that shares the Guard port', () => {
		expect(http('/rivet/namespaces')).toBe('deny');
		expect(http('/rivet/namespaces', 'POST')).toBe('deny');
		expect(http('/rivet/runner-configs/default', 'PUT')).toBe('deny');
		expect(http('/rivet/runner-configs', 'GET')).toBe('deny');
		expect(http('/rivet/datacenters')).toBe('deny');
		expect(http('/rivet/')).toBe('deny');
		expect(http('/rivet')).toBe('deny');
		expect(http('/rivet/ui')).toBe('deny'); // bundled dashboard
		expect(http('/rivet/gateway/actor-1/inspector/state')).toBe('deny');
		expect(http('/rivet/gateway/actor-1/health')).toBe('deny');
		expect(http('/rivet/actors/actor-1/kv/keys/k')).toBe('deny');
		expect(http('/rivet/actors', 'GET')).toBe('deny');
		expect(http('/rivet/actors', 'PUT')).toBe('deny');
		expect(http('/rivet/actors', 'POST')).toBe('deny');
		expect(http('/rivet/actors/actor-123', 'DELETE')).toBe('deny');
		expect(http('/rivet/gateway/user/dynamic/reload', 'PUT')).toBe('deny');
	});

	test('denies malformed/edge paths', () => {
		expect(http('/other/actors')).toBe('deny'); // outside mount prefix
		expect(http('/rivet/actors/a/b')).toBe('deny'); // too deep
		expect(http('/rivet/metadata', 'POST')).toBe('deny');
		expect(http('/rivet/gateway/user/connect')).toBe('deny'); // WS path, no upgrade
		expect(ws('/rivet/gateway/user/action/x', 'POST', true)).toBe('deny'); // upgrade to http path
		expect(http('/rivet/gateway/user/connect/extra')).toBe('deny');
	});

	test('strips the prefix for the upstream path', () => {
		const route = classifyProxyPath('/rivet/gateway/user/connect', 'GET', true);
		expect(route.upstreamPath).toBe('/gateway/user/connect');
	});
});

describe('isOriginAllowed', () => {
	test('absent origin passes (non-browser clients)', () => {
		expect(isOriginAllowed(null, ['https://layerr.ai'])).toBe(true);
	});
	test('listed origin passes, unlisted rejects, wildcard passes', () => {
		expect(isOriginAllowed('https://layerr.ai', ['https://layerr.ai'])).toBe(true);
		expect(isOriginAllowed('https://evil.com', ['https://layerr.ai'])).toBe(false);
		expect(isOriginAllowed('https://anything.com', '*')).toBe(true);
	});
});

describe('toWebSocketBase', () => {
	expect(toWebSocketBase('http://engine:6420')).toBe('ws://engine:6420');
	expect(toWebSocketBase('https://engine:6420')).toBe('wss://engine:6420');
});

describe('Bun-native WebSocket admission', () => {
	test('uses explicit payload and backpressure ceilings without compression', () => {
		const maxBufferedBytes = 2 * 1024 * 1024;
		const proxy = createRivetGatewayProxy({
			upstreamHttp: 'http://127.0.0.1:6420',
			allowedOrigins: ['http://localhost:4000'],
			maxBufferedBytes,
			log: quietLog
		});
		expect(proxy.websocket.maxPayloadLength).toBe(16 * 1024 * 1024);
		expect(proxy.websocket.backpressureLimit).toBe(maxBufferedBytes);
		expect(proxy.websocket.closeOnBackpressureLimit).toBe(true);
		expect(proxy.websocket.sendPings).toBe(true);
		expect(proxy.websocket.perMessageDeflate).toBe(false);
	});

	test('queued frames stay FIFO when a leg becomes writable before drain', async () => {
		const OriginalWebSocket = globalThis.WebSocket;
		class FakeClientWebSocket {
			static latest: FakeClientWebSocket | null = null;
			bufferedAmount = 0;
			binaryType = 'arraybuffer';
			sent: Array<string | Uint8Array> = [];
			private listeners = new Map<string, Array<(event: any) => void>>();

			constructor() {
				FakeClientWebSocket.latest = this;
			}

			addEventListener(name: string, listener: (event: any) => void) {
				const listeners = this.listeners.get(name) ?? [];
				listeners.push(listener);
				this.listeners.set(name, listeners);
			}

			emit(name: string, event: any = {}) {
				for (const listener of this.listeners.get(name) ?? []) listener(event);
			}

			send(payload: string | Uint8Array) {
				this.sent.push(payload);
			}

			close() {}
			terminate() {}
		}

		Object.defineProperty(globalThis, 'WebSocket', {
			configurable: true,
			writable: true,
			value: FakeClientWebSocket
		});

		const browserSent: Array<string | Uint8Array> = [];
		const data: WsConnData = {
			upstreamUrl: 'ws://127.0.0.1:6420/gateway/user/connect',
			protocols: ['rivet'],
			origin: 'http://localhost:4000',
			clientIp: '127.0.0.1',
			upstream: null,
			upstreamOpen: false,
			downQueue: [],
			downQueuedBytes: 0,
			upQueue: [],
			upQueuedBytes: 0,
			closed: false,
			dialTimer: null,
			drainTimer: null
		};
		const browser = {
			data,
			getBufferedAmount: () => 0,
			send(payload: string | Uint8Array) {
				browserSent.push(payload);
				return typeof payload === 'string' ? Buffer.byteLength(payload) : payload.byteLength;
			},
			cork(callback: () => void) {
				callback();
			},
			close() {}
		};

		try {
			const proxy = createRivetGatewayProxy({
				upstreamHttp: 'http://127.0.0.1:6420',
				allowedOrigins: ['http://localhost:4000'],
				log: quietLog
			});
			proxy.websocket.open?.(browser as never);
			const upstream = FakeClientWebSocket.latest;
			expect(upstream).not.toBeNull();
			upstream?.emit('open');

			data.downQueue.push('down-A');
			data.downQueuedBytes = Buffer.byteLength('down-A');
			proxy.websocket.message?.(browser as never, 'down-B');
			await waitFor(() => (upstream?.sent.length ?? 0) === 2);
			expect(upstream?.sent).toEqual(['down-A', 'down-B']);

			data.upQueue.push('up-A');
			data.upQueuedBytes = Buffer.byteLength('up-A');
			upstream?.emit('message', { data: 'up-B' });
			expect(browserSent).toEqual([]);
			proxy.websocket.drain?.(browser as never);
			expect(browserSent).toEqual(['up-A', 'up-B']);

			proxy.websocket.close?.(browser as never, 1000, 'done');
		} finally {
			Object.defineProperty(globalThis, 'WebSocket', {
				configurable: true,
				writable: true,
				value: OriginalWebSocket
			});
		}
	});
});

// ---------------------------------------------------------------------------
// Integration: mock upstream engine + proxy over real sockets
// ---------------------------------------------------------------------------

interface RecordedUpgrade {
	protocols: string[];
	origin: string | null;
	path: string;
	xff: string | null;
	extensions: string | null;
}
const upgrades: RecordedUpgrade[] = [];
const queryActions: Array<{
	path: string;
	connParams: string | null;
}> = [];
let upstreamClose: { code: number; reason: string } | null = null;
let holdUpstreamClose: ((code: number, reason: string) => void) | null = null;

const upstreamServer = Bun.serve({
	port: 0,
	hostname: '127.0.0.1',
	fetch(req, server) {
		const url = new URL(req.url);
		if (
			url.pathname.startsWith('/gateway/') &&
			req.headers.get('upgrade')?.toLowerCase() === 'websocket'
		) {
			upgrades.push({
				protocols: (req.headers.get('sec-websocket-protocol') ?? '')
					.split(',')
					.map((p) => p.trim())
					.filter(Boolean),
				origin: req.headers.get('origin'),
				path: url.pathname + url.search,
				xff: req.headers.get('x-forwarded-for'),
				extensions: req.headers.get('sec-websocket-extensions')
			});
			const upgraded = server.upgrade(req, {
				data: {},
				headers: { 'Sec-WebSocket-Protocol': 'rivet' }
			});
			return upgraded ? undefined : new Response('upgrade failed', { status: 400 });
		}
		if (url.pathname === '/gateway/example/action/ping' && req.method === 'POST') {
			queryActions.push({
				path: url.pathname + url.search,
				connParams: req.headers.get('x-rivet-conn-params')
			});
			return Response.json({ output: 'pong' });
		}
		if (url.pathname === '/gateway/user/action/getProfile' && req.method === 'POST') {
			return (async () => {
				const body = await req.text();
				return Response.json({
					method: req.method,
					search: url.search,
					body,
					connParams: req.headers.get('x-rivet-conn-params'),
					authorization: req.headers.get('authorization'),
					cookie: req.headers.get('cookie'),
					xff: req.headers.get('x-forwarded-for'),
					forwarded: req.headers.get('forwarded'),
					xRealIp: req.headers.get('x-real-ip'),
					trueClientIp: req.headers.get('true-client-ip')
				});
			})();
		}
		return new Response('engine admin surface', { status: 401 });
	},
	websocket: {
		open(ws) {
			holdUpstreamClose = (code, reason) => ws.close(code, reason);
		},
		message(ws, msg) {
			ws.send(msg); // echo
		},
		close(_ws, code, reason) {
			upstreamClose = { code, reason };
		}
	}
});

const proxy = createRivetGatewayProxy({
	upstreamHttp: `http://127.0.0.1:${upstreamServer.port}`,
	allowedOrigins: ['http://localhost:4000'],
	log: quietLog
});
const proxyServer = Bun.serve({
	port: 0,
	hostname: '127.0.0.1',
	routes: {
		'/rivet/*': (req, server) => proxy.handleRequest(req, server) as Response | undefined
	},
	fetch: () => new Response('elysia would be here', { status: 404 }),
	websocket: proxy.websocket
});
const PROXY = `http://127.0.0.1:${proxyServer.port}`;
const PROXY_WS = `ws://127.0.0.1:${proxyServer.port}`;

afterAll(() => {
	proxyServer.stop(true);
	upstreamServer.stop(true);
});

function connectWs(
	url: string,
	opts: {
		protocols?: string[];
		origin?: string;
		headers?: Record<string, string>;
	} = {}
): Promise<WebSocket> {
	return new Promise((resolve, reject) => {
		const ws = new WebSocket(url, {
			protocols: opts.protocols ?? [],
			headers: {
				...(opts.origin ? { Origin: opts.origin } : {}),
				...(opts.headers ?? {})
			}
		} as never);
		const timer = setTimeout(() => reject(new Error('ws open timeout')), 5000);
		ws.addEventListener('open', () => {
			clearTimeout(timer);
			resolve(ws);
		});
		ws.addEventListener('error', () => {
			clearTimeout(timer);
			reject(new Error('ws connect failed'));
		});
	});
}

/** The browser leg upgrades first; the proxy's upstream dial lands a tick later. */
async function waitFor(cond: () => boolean, ms = 3000): Promise<void> {
	const deadline = Date.now() + ms;
	while (Date.now() < deadline) {
		if (cond()) return;
		await new Promise((r) => setTimeout(r, 15));
	}
	throw new Error('waitFor timed out');
}

describe('proxy integration', () => {
	test('browser client actions use the authenticated query gateway without /actors', async () => {
		queryActions.length = 0;
		const client = createClient<any>({
			endpoint: `${PROXY}/rivet`,
			disableMetadataLookup: true,
			encoding: 'json',
			gateway: { skipReadyWait: true }
		});
		try {
			const result = await client.example
				.getOrCreate(['example', 'key-1'], {
					params: { authToken: 'signed-actor-token' }
				})
				.ping();
			expect(result).toBe('pong');
			expect(queryActions).toHaveLength(1);
			expect(queryActions[0]!.path).toContain('/gateway/example/action/ping?');
			expect(queryActions[0]!.path).toContain('rvt-method=getOrCreate');
			expect(queryActions[0]!.connParams).toBe(JSON.stringify({ authToken: 'signed-actor-token' }));
		} finally {
			client.dispose();
		}
	});

	test('WS relay: echo round trip (text + binary), protocols + origin forwarded', async () => {
		upgrades.length = 0;
		upstreamClose = null;
		const ws = await connectWs(
			`${PROXY_WS}/rivet/gateway/user/connect?rvt-namespace=default&rvt-method=getOrCreate`,
			{
				protocols: ['rivet', 'rivet_encoding.bare', 'rivet_conn_params.%7B%7D'],
				origin: 'http://localhost:4000'
			}
		);

		// Upstream saw the identical subprotocol list, the browser's Origin, and
		// the full query string.
		await waitFor(() => upgrades.length === 1);
		expect(upgrades[0].protocols).toEqual([
			'rivet',
			'rivet_encoding.bare',
			'rivet_conn_params.%7B%7D'
		]);
		expect(upgrades[0].origin).toBe('http://localhost:4000');
		expect(upgrades[0].path).toBe(
			'/gateway/user/connect?rvt-namespace=default&rvt-method=getOrCreate'
		);
		expect(upgrades[0].extensions).toBeNull();

		// Text echo.
		const textEcho = new Promise<string>((resolve) => {
			ws.addEventListener('message', (e) => resolve(String(e.data)), {
				once: true
			});
		});
		ws.send('hello');
		expect(await textEcho).toBe('hello');

		// Binary echo.
		const binEcho = new Promise<Uint8Array>((resolve) => {
			ws.addEventListener('message', (e) => resolve(new Uint8Array(e.data as ArrayBuffer)), {
				once: true
			});
		});
		ws.send(new Uint8Array([1, 2, 3, 250]));
		expect(Array.from(await binEcho)).toEqual([1, 2, 3, 250]);

		// Client close propagates to the upstream with the same code.
		ws.close(1000, 'done');
		await new Promise((r) => setTimeout(r, 150));
		expect(upstreamClose?.code).toBe(1000);
	});

	test('WS relay: upstream-initiated close mirrors code to the browser', async () => {
		const before = upgrades.length;
		const ws = await connectWs(`${PROXY_WS}/rivet/gateway/user/connect`, {
			protocols: ['rivet'],
			origin: 'http://localhost:4000'
		});
		const closed = new Promise<{ code: number; reason: string }>((resolve) => {
			ws.addEventListener('close', (e) => resolve({ code: e.code, reason: e.reason }), {
				once: true
			});
		});
		// Wait for THIS connection's upstream leg before driving its close.
		await waitFor(() => upgrades.length > before);
		holdUpstreamClose?.(1011, 'internal error');
		// 1011 mirrors exactly.
		expect(await closed).toEqual({ code: 1011, reason: 'internal error' });
	});

	test('WS relay: upstream 1001 and its reason reach the browser intact', async () => {
		const before = upgrades.length;
		const ws = await connectWs(`${PROXY_WS}/rivet/gateway/user/connect`, {
			protocols: ['rivet'],
			origin: 'http://localhost:4000'
		});
		const closed = new Promise<{ code: number; reason: string }>((resolve) => {
			ws.addEventListener('close', (e) => resolve({ code: e.code, reason: e.reason }), {
				once: true
			});
		});
		await waitFor(() => upgrades.length > before);
		holdUpstreamClose?.(1001, 'engine redeploy');
		expect(await closed).toEqual({ code: 1001, reason: 'engine redeploy' });
	});

	test('WS relay: abrupt browser termination still closes the upstream leg', async () => {
		const before = upgrades.length;
		upstreamClose = null;
		const ws = await connectWs(`${PROXY_WS}/rivet/gateway/user/connect`, {
			protocols: ['rivet'],
			origin: 'http://localhost:4000'
		});
		await waitFor(() => upgrades.length > before);

		// The proxy receives this as abnormal closure (1006), which cannot be
		// emitted in a close frame. It must normalize before closing upstream.
		ws.terminate();
		await waitFor(() => upstreamClose !== null);
		expect(upstreamClose).toEqual({ code: 1001, reason: 'client closed' });
	});

	test('WS relay: one frame cannot overshoot the upstream buffer budget', async () => {
		const tightProxy = createRivetGatewayProxy({
			upstreamHttp: `http://127.0.0.1:${upstreamServer.port}`,
			allowedOrigins: ['http://localhost:4000'],
			maxBufferedBytes: 512,
			log: quietLog
		});
		const tightServer = Bun.serve({
			port: 0,
			hostname: '127.0.0.1',
			routes: {
				'/rivet/*': (req, server) => tightProxy.handleRequest(req, server) as Response | undefined
			},
			fetch: () => new Response('not found', { status: 404 }),
			websocket: tightProxy.websocket
		});

		try {
			const before = upgrades.length;
			const ws = await connectWs(`ws://127.0.0.1:${tightServer.port}/rivet/gateway/user/connect`, {
				protocols: ['rivet'],
				origin: 'http://localhost:4000'
			});
			await waitFor(() => upgrades.length > before);
			const closed = new Promise<{ code: number; reason: string }>((resolve) => {
				ws.addEventListener(
					'close',
					(event) => resolve({ code: event.code, reason: event.reason }),
					{ once: true }
				);
			});
			ws.send(new Uint8Array(513));
			expect(await closed).toEqual({
				code: 1013,
				reason: 'backpressure limit'
			});
		} finally {
			tightServer.stop(true);
		}
	});

	test('WS relay: bad Origin is rejected pre-dial', async () => {
		const before = upgrades.length;
		await expect(
			connectWs(`${PROXY_WS}/rivet/gateway/user/connect`, {
				protocols: ['rivet'],
				origin: 'https://evil.example.com'
			})
		).rejects.toThrow();
		// Rejected before dialing — no upgrade reached the upstream.
		expect(upgrades.length).toBe(before);
	});

	test('HTTP passthrough: method/query/body/headers forward, no credentials injected', async () => {
		const res = await fetch(`${PROXY}/rivet/gateway/user/action/getProfile?namespace=default`, {
			method: 'POST',
			headers: {
				'Content-Type': 'application/json',
				'x-rivet-conn-params': '{"internalSecret":"x"}',
				// Browsers auto-attach the api origin's session credentials to every
				// /rivet/* request — the proxy must strip, never forward them.
				Cookie: 'better-auth.session_token=supersecret',
				Authorization: 'Bearer supersecret'
			},
			body: JSON.stringify({ args: [] })
		});
		expect(res.status).toBe(200);
		const echoed = await res.json();
		expect(echoed.method).toBe('POST');
		expect(echoed.search).toBe('?namespace=default');
		expect(JSON.parse(echoed.body)).toEqual({ args: [] });
		expect(echoed.connParams).toBe('{"internalSecret":"x"}');
		expect(echoed.authorization).toBeNull(); // stripped, never forwarded
		expect(echoed.cookie).toBeNull(); // stripped, never forwarded
	});

	test('HTTP passthrough: X-Forwarded-For is OVERWRITTEN with the trusted derivation', async () => {
		// A client-supplied first entry must not reach the engine — the Guard
		// keys its per-IP rate-limit bucket on it. The LAST entry stands in for
		// what Railway's edge appends; here it wins.
		const res = await fetch(`${PROXY}/rivet/gateway/user/action/getProfile?namespace=default`, {
			method: 'POST',
			headers: { 'X-Forwarded-For': '6.6.6.6, 203.0.113.9' },
			body: JSON.stringify({ args: [] })
		});
		expect(res.status).toBe(200);
		expect((await res.json()).xff).toBe('203.0.113.9');

		// Garbage header → the direct socket IP (the loopback test client).
		const res2 = await fetch(`${PROXY}/rivet/gateway/user/action/getProfile?namespace=default`, {
			method: 'POST',
			headers: { 'X-Forwarded-For': 'not-an-ip' },
			body: JSON.stringify({ args: [] })
		});
		expect(res2.status).toBe(200);
		expect((await res2.json()).xff).toBe('127.0.0.1');
	});

	test('WS relay: upstream dial carries the trusted X-Forwarded-For', async () => {
		upgrades.length = 0;
		const ws = await connectWs(`${PROXY_WS}/rivet/gateway/user/connect?rvt-namespace=default`, {
			protocols: ['rivet'],
			origin: 'http://localhost:4000'
		});
		await waitFor(() => upgrades.length === 1);
		// No inbound XFF in the test → the browser leg's socket IP.
		expect(upgrades[0].xff).toBe('127.0.0.1');
		ws.close();
	});

	test('WS relay: inbound X-Forwarded-For derives last-entry, forged first entry never wins', async () => {
		upgrades.length = 0;
		const ws = await connectWs(`${PROXY_WS}/rivet/gateway/user/connect?rvt-namespace=default`, {
			protocols: ['rivet'],
			origin: 'http://localhost:4000',
			headers: { 'X-Forwarded-For': '6.6.6.6, 203.0.113.9' }
		});
		await waitFor(() => upgrades.length === 1);
		expect(upgrades[0].xff).toBe('203.0.113.9');
		ws.close();
	});

	test('HTTP passthrough: other client-identity headers are stripped, never forwarded', async () => {
		// The 2.3.10 Guard only reads X-Forwarded-For, but a future engine
		// honoring Forwarded / X-Real-IP would re-open client-chosen bucketing
		// if these passed through.
		const res = await fetch(`${PROXY}/rivet/gateway/user/action/getProfile?namespace=default`, {
			method: 'POST',
			headers: {
				Forwarded: 'for=6.6.6.6',
				'X-Real-IP': '6.6.6.6',
				'True-Client-IP': '6.6.6.6'
			},
			body: JSON.stringify({ args: [] })
		});
		expect(res.status).toBe(200);
		const echoed = await res.json();
		expect(echoed.forwarded).toBeNull();
		expect(echoed.xRealIp).toBeNull();
		expect(echoed.trueClientIp).toBeNull();
		// The trusted derivation still rides the one canonical header.
		expect(echoed.xff).toBe('127.0.0.1');
	});

	test('/metadata is a static stub, never proxied', async () => {
		const res = await fetch(`${PROXY}/rivet/metadata`);
		expect(res.status).toBe(200);
		expect(await res.json()).toEqual({});
	});

	test("admin surfaces are denied BY THE PROXY (404, not the engine's 401)", async () => {
		for (const path of [
			'/rivet/namespaces',
			'/rivet/runner-configs/default',
			'/rivet/actors',
			'/rivet/actors/actor-1',
			'/rivet/gateway/user/dynamic/reload',
			'/rivet/'
		]) {
			const res = await fetch(`${PROXY}${path}`);
			expect(res.status).toBe(404);
			expect((await res.json()).error).toBe('not_found');
		}
	});

	test('HTTP Origin gate applies to passthrough too', async () => {
		const res = await fetch(`${PROXY}/rivet/gateway/user/action/getProfile?namespace=default`, {
			method: 'POST',
			headers: { Origin: 'https://evil.example.com' },
			body: JSON.stringify({ args: [] })
		});
		expect(res.status).toBe(403);
		expect((await res.json()).error).toBe('origin_not_allowed');
	});
});
