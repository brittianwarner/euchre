/**
 * Relay adapted from Layerr's private-engine gateway. The entry point adds a
 * signed-cookie gate and only permits existing table connections by actor ID.
 * Rivet 2.3.23 verifies short-lived actor-scoped gateway JWTs; the actor verifies
 * signed connection params and ownership. No admin credential is injected.
 * Cookies and Authorization are stripped upstream. Control-plane paths are denied.
 *
 * Relay mechanics:
 *   - Subprotocol list forwarded verbatim upstream; the static "rivet"
 *     matcher is echoed to the browser (the offer list is what the engine
 *     reads; the client does not validate the negotiated value).
 *   - Backpressure: both directions buffer into a bounded FIFO; a peer whose
 *     buffer exceeds the hard cap gets BOTH legs closed (1013) — a slow
 *     browser tab must never grow an unbounded buffer in api memory.
 *   - Close propagation: close code/reason mirrored both ways; upstream
 *     errors map to 1011 so the client's structured reconnect keeps working.
 *   - Upstream restarts: the browser leg closes with the upstream's code and
 *     the rivetkit client's existing retry-with-backoff does the reconnect.
 */
import type { Server, ServerWebSocket, WebSocketHandler } from 'bun';
import { isIP } from 'node:net';
const logger = console;

// ---- Tunables ----------------------------------------------------------------

/** Start queueing for a peer above this buffered amount (one max-size rivet message). */
const HIGH_WATER_BYTES = 4 * 1024 * 1024;
/** Resume flushing to a peer below this buffered amount. */
const LOW_WATER_BYTES = 1 * 1024 * 1024;
/** Hard per-connection buffer cap (both queues combined) — exceeding closes the socket. */
const DEFAULT_MAX_BUFFERED_BYTES = 16 * 1024 * 1024;
/** Rivet client frames share the same explicit ceiling as the relay buffer. */
const DEFAULT_MAX_PAYLOAD_BYTES = 16 * 1024 * 1024;
/** Upstream WS dial deadline. */
const DEFAULT_UPSTREAM_DIAL_TIMEOUT_MS = 10_000;
/** Upstream browser-style WebSocket has no drain event — poll cadence while flushing. */
const UPSTREAM_DRAIN_POLL_MS = 25;
/** HTTP passthrough ceiling; above the engine's own action/request timeouts. */
const HTTP_PROXY_TIMEOUT_MS = 120_000;

// ---- Pure route classification (the allowlist) --------------------------------

export type ProxyRouteKind = 'metadata' | 'http' | 'ws' | 'deny';

export interface ProxyRoute {
	kind: ProxyRouteKind;
	/** Path with the mount prefix stripped — what gets appended to the upstream base. */
	upstreamPath: string;
}

const GATEWAY_SEG = String.raw`[^/]+`; // actor name OR actorId[@token]
const WS_PATH_RE = new RegExp(`^/gateway/${GATEWAY_SEG}(?:/connect|/websocket(?:/.*)?)$`);
const HTTP_GATEWAY_RE = new RegExp(
	`^/gateway/${GATEWAY_SEG}(?:/action/[^/]+|/queue/[^/]+|/request(?:/.*)?)$`
);

/**
 * Classify an incoming path against the client wire-contract allowlist.
 * `pathname` includes the mount prefix (e.g. /rivet/gateway/user/connect).
 * Prefix stripping is strict — a path outside the mount is denied.
 *
 * Verified against the vendored rivetkit engine-client (2026-09-04): browser
 * connections and actions can target an actor query directly when the client
 * uses `gateway.skipReadyWait=true`. That keeps every data-plane request behind
 * the actor's onBeforeConnect/createConnState authorization and lets this edge
 * deny GET/PUT/POST/DELETE /actors in full. Dynamic reload is denied too: its
 * client call carries no connection params and therefore has no actor HMAC.
 * /metadata is NEVER proxied to the engine — a static stub is answered locally
 * so the engine cannot hand a browser a clientEndpoint/clientToken.
 */
export function classifyProxyPath(
	pathname: string,
	method: string,
	isUpgrade: boolean,
	prefix = '/rivet'
): ProxyRoute {
	if (!pathname.startsWith(prefix)) return { kind: 'deny', upstreamPath: '' };
	const path = pathname.slice(prefix.length) || '/';
	const m = method.toUpperCase();

	if (path === '/metadata' && m === 'GET') {
		return { kind: 'metadata', upstreamPath: path };
	}
	if (path === '/connect' || WS_PATH_RE.test(path)) {
		return isUpgrade ? { kind: 'ws', upstreamPath: path } : { kind: 'deny', upstreamPath: '' };
	}
	if (HTTP_GATEWAY_RE.test(path)) {
		return isUpgrade ? { kind: 'deny', upstreamPath: '' } : { kind: 'http', upstreamPath: path };
	}
	return { kind: 'deny', upstreamPath: '' };
}

/**
 * Origin gate. Browsers always send Origin on WS upgrades and CORS-flagged
 * fetches; those must match the allowlist. Origin-less requests (curl,
 * scripts, server-side probes) pass — actor HMAC verification is the
 * boundary for them, exactly as on Rivet Cloud.
 */
export function isOriginAllowed(origin: string | null, allowed: string[] | '*'): boolean {
	if (origin === null) return true;
	if (allowed === '*') return true;
	return allowed.includes(origin);
}

/** http(s) → ws(s) for the upstream dial. */
export function toWebSocketBase(httpBase: string): string {
	return httpBase.replace(/^http/i, 'ws');
}

/**
 * Trusted client IP for the engine's Guard rate-limit key.
 *
 * The Guard enforces a HARDCODED per-client-IP limit (10,000 req/60s + 2,000
 * in-flight — guard-core/src/request_context.rs; no config knob in 2.3.x) and
 * keys on the FIRST X-Forwarded-For entry when parseable, else the socket IP
 * (verified against the pinned 2.3.10 image, 2026-08-27). Before this
 * existed, every relayed browser connection reached the Guard from the api
 * service's single IP, so the entire platform shared ONE bucket and bulk
 * server-side traffic starved browser connects with `guard.rate_limit`.
 *
 * Stamping the real client IP gives every browser client its own bucket:
 * users can never rate-limit each other or be starved by server traffic,
 * while an actual abuser of the public /rivet surface still only exhausts
 * their own bucket.
 *
 * Trust order:
 *   1. LAST entry of the inbound X-Forwarded-For — appended by the nearest
 *      trusted hop (Railway's edge). Never the first entry: that one is
 *      client-forgeable, and forwarding it verbatim would let a client choose
 *      its own rate-limit bucket. Deliberately no backward walk on a
 *      non-parseable last entry — everything before it is client-supplied.
 *   2. The direct socket IP (local dev, private-network callers).
 *
 * Tolerates `ip:port` / `[v6]:port` entry shapes (RFC 7239-adjacent edge
 * formats). If the last entry is present but unparseable even after
 * port-stripping, a warn fires: that means an edge format change just
 * silently collapsed every browser into the shared socket-IP bucket — the
 * pre-incident topology — and the derivation needs updating.
 */
/**
 * Cap a WebSocket close reason at the protocol's 123 UTF-8 BYTE limit.
 * `.slice(0, 123)` counts UTF-16 code units — multibyte content can pass it
 * at up to 3× the byte limit, which byte-truncates mid-character on the Bun
 * server leg (client sees 1007 "invalid UTF8" with NO reason) and throws a
 * SyntaxError on the client-WebSocket upstream leg (verified Bun 1.4.0).
 * Encode → cap → decode, dropping any partial trailing sequence.
 */
export function capWsCloseReason(reason: string): string {
	const bytes = new TextEncoder().encode(reason);
	if (bytes.length <= 123) return reason;
	return new TextDecoder().decode(bytes.subarray(0, 123)).replace(/�+$/, '');
}

function stripXffEntryPort(entry: string): string {
	const bracketed = entry.match(/^\[([^\]]+)\](?::\d+)?$/);
	if (bracketed?.[1]) return bracketed[1];
	const v4WithPort = entry.match(/^(\d{1,3}(?:\.\d{1,3}){3}):\d+$/);
	if (v4WithPort?.[1]) return v4WithPort[1];
	return entry;
}

// The unparseable-entry warn is throttled: deriveTrustedClientIp is also on
// unauthenticated request paths (forms rate limiting), where the entry is
// attacker-supplied — unthrottled it is a free warn-log amplifier. One warn
// per window is all the signal "the edge format changed" needs.
const XFF_WARN_INTERVAL_MS = 60_000;
let lastXffWarnAt = 0;

export function deriveTrustedClientIp(
	forwardedFor: string | null,
	socketAddress: string | null
): string | null {
	if (forwardedFor) {
		const entries = forwardedFor
			.split(',')
			.map((entry) => entry.trim())
			.filter(Boolean);
		const last = entries[entries.length - 1];
		if (last) {
			const candidate = stripXffEntryPort(last);
			if (isIP(candidate) !== 0) return candidate;
			const now = Date.now();
			if (now - lastXffWarnAt >= XFF_WARN_INTERVAL_MS) {
				lastXffWarnAt = now;
				logger.warn(
					'rivet proxy: unparseable last X-Forwarded-For entry — falling back to socket IP (shared guard bucket)',
					// Client-controlled — keep printable ASCII only (no log injection).
					{ entry: last.replace(/[^\x20-\x7e]/g, '?').slice(0, 64) }
				);
			}
		}
	}
	if (socketAddress && isIP(socketAddress) !== 0) return socketAddress;
	return null;
}

// ---- Relay -------------------------------------------------------------------

/** Per-connection relay state (Bun.serve websocket `data`). */
export interface WsConnData {
	upstreamUrl: string;
	protocols: string[];
	origin: string | null;
	/** Trusted client IP stamped as X-Forwarded-For on the upstream dial. */
	clientIp: string | null;
	upstream: WebSocket | null;
	upstreamOpen: boolean;
	/** Browser → upstream frames held while the upstream dial is pending or backpressured. */
	downQueue: Array<string | Uint8Array<ArrayBuffer>>;
	downQueuedBytes: number;
	/** Upstream → browser frames held while the browser leg is backpressured. */
	upQueue: Array<string | Uint8Array>;
	upQueuedBytes: number;
	closed: boolean;
	dialTimer: ReturnType<typeof setTimeout> | null;
	drainTimer: ReturnType<typeof setInterval> | null;
}

function frameBytes(payload: string | Uint8Array): number {
	return typeof payload === 'string' ? Buffer.byteLength(payload) : payload.byteLength;
}

function totalQueuedBytes(d: WsConnData): number {
	return d.downQueuedBytes + d.upQueuedBytes;
}

function isSendableCloseCode(code: number): boolean {
	return (
		(code >= 1000 && code <= 1014 && code !== 1004 && code !== 1005 && code !== 1006) ||
		(code >= 3000 && code <= 4999)
	);
}

function normalizedCloseCode(code: number, fallback: 1001 | 1011): number {
	return isSendableCloseCode(code) ? code : fallback;
}

/**
 * Abort a Bun client WebSocket after graceful close rejects. TypeScript's DOM
 * `WebSocket` declaration wins for the global constructor in this package,
 * so describe Bun's runtime-only `terminate()` extension at this narrow seam.
 */
function terminateUpstream(socket: WebSocket | null): void {
	(
		socket as
			| (WebSocket & {
					terminate(): void;
			  })
			| null
	)?.terminate();
}

function canSendUpstreamFrame(
	upstream: WebSocket,
	payload: string | Uint8Array,
	maxBuffered: number
): boolean {
	const buffered = upstream.bufferedAmount;
	return buffered <= HIGH_WATER_BYTES && buffered + frameBytes(payload) <= maxBuffered;
}

function wouldExceedUpstreamBudget(
	d: WsConnData,
	payload: string | Uint8Array,
	maxBuffered: number
): boolean {
	return (
		(d.upstream?.bufferedAmount ?? 0) + totalQueuedBytes(d) + frameBytes(payload) > maxBuffered
	);
}

export interface RivetGatewayProxyOptions {
	/** Bare engine base URL, e.g. http://rivet-engine.railway.internal:6420 (no userinfo, no path). */
	upstreamHttp: string;
	allowedOrigins: string[] | '*';
	/** Mount prefix the route is registered under. */
	prefix?: string;
	maxBufferedBytes?: number;
	upstreamDialTimeoutMs?: number;
	log?: Pick<typeof logger, 'info' | 'warn' | 'error'>;
}

export interface RivetGatewayProxy {
	/** Bun routes-map handler: `(req, server)`. Returns undefined after a successful upgrade. */
	handleRequest: (
		req: Request,
		server: Server<WsConnData>
	) => Response | undefined | Promise<Response | undefined>;
	/** Bun.serve `websocket` handler block — active only for proxy-upgraded connections. */
	websocket: WebSocketHandler<WsConnData>;
}

const HOP_BY_HOP_REQ = new Set([
	'connection',
	'keep-alive',
	'transfer-encoding',
	'upgrade',
	'host',
	'content-length',
	// Credential hygiene, not hop-by-hop: browsers auto-attach the api
	// origin's Better Auth session (Cookie, and any ambient Authorization)
	// to every /rivet/* request. The rivetkit client authenticates with the
	// Layerr HMAC actor token via `x-rivet-conn-params` (verified
	// actor-router-consts.ts) and NEVER these headers, so stripping loses
	// nothing and keeps Layerr session credentials off the engine.
	'cookie',
	'authorization',
	// Client-identity headers: the proxy derives the ONE trusted identity
	// itself (deriveTrustedClientIp → x-forwarded-for). The 2.3.10 Guard
	// keys only on X-Forwarded-For, but a future engine honoring the
	// standardized Forwarded header (or X-Real-IP et al.) would re-open
	// client-chosen rate-limit bucketing if these passed through.
	'forwarded',
	'x-real-ip',
	'x-client-ip',
	'true-client-ip',
	'cf-connecting-ip',
	'x-cluster-client-ip'
]);
const HOP_BY_HOP_RES = new Set(['connection', 'keep-alive', 'transfer-encoding']);

function deny(status: 403 | 404, reason: string): Response {
	return new Response(JSON.stringify({ error: reason }), {
		status,
		headers: { 'Content-Type': 'application/json' }
	});
}

export function createRivetGatewayProxy(opts: RivetGatewayProxyOptions): RivetGatewayProxy {
	const prefix = opts.prefix ?? '/rivet';
	const maxBuffered = opts.maxBufferedBytes ?? DEFAULT_MAX_BUFFERED_BYTES;
	const dialTimeoutMs = opts.upstreamDialTimeoutMs ?? DEFAULT_UPSTREAM_DIAL_TIMEOUT_MS;
	const log = opts.log ?? logger;
	const upstreamHttp = opts.upstreamHttp.replace(/\/+$/, '');
	const upstreamWs = toWebSocketBase(upstreamHttp);

	const metadataStub = () =>
		// Deliberately empty: the rivetkit client only acts on camelCase
		// clientEndpoint/clientNamespace/clientToken overrides; absent keys mean
		// "keep the configured endpoint". A successful-but-empty body also keeps
		// any client WITHOUT disableMetadataLookup out of the forever-retry loop.
		new Response('{}', {
			status: 200,
			headers: {
				'Content-Type': 'application/json',
				'Cache-Control': 'no-store'
			}
		});

	const passthroughHttp = async (
		req: Request,
		upstreamPath: string,
		clientIp: string | null
	): Promise<Response> => {
		const url = new URL(req.url);
		const upstreamUrl = `${upstreamHttp}${upstreamPath}${url.search}`;
		const headers = new Headers();
		req.headers.forEach((value, key) => {
			if (!HOP_BY_HOP_REQ.has(key.toLowerCase())) headers.set(key, value);
		});
		// Guard rate-limit key: OVERWRITE, never pass through — a forwarded
		// client-supplied X-Forwarded-For would let the client pick its own
		// bucket (the Guard trusts the first entry). See deriveTrustedClientIp.
		if (clientIp) {
			headers.set('x-forwarded-for', clientIp);
		} else {
			headers.delete('x-forwarded-for');
		}
		const hasBody = req.method !== 'GET' && req.method !== 'HEAD';
		try {
			const upstream = await fetch(upstreamUrl, {
				method: req.method,
				headers,
				body: hasBody ? req.body : undefined,
				redirect: 'manual',
				signal: AbortSignal.timeout(HTTP_PROXY_TIMEOUT_MS)
			});
			if (upstream.status === 429) {
				// Guard rate limit crossing the relay — the one place the browser
				// lane's saturation is visible server-side. Stable grep tag shared
				// with the elysia 503 mapping.
				log.warn('[guard-503] rivet proxy: upstream 429 (guard rate limit)', {
					path: upstreamPath,
					clientIp
				});
			}
			const resHeaders = new Headers();
			upstream.headers.forEach((value, key) => {
				if (!HOP_BY_HOP_RES.has(key.toLowerCase())) resHeaders.set(key, value);
			});
			return new Response(upstream.body, {
				status: upstream.status,
				statusText: upstream.statusText,
				headers: resHeaders
			});
		} catch (err) {
			log.warn('rivet proxy: upstream HTTP error', {
				path: upstreamPath,
				error: err instanceof Error ? err.message : String(err)
			});
			return deny(404, 'upstream_unreachable');
		}
	};

	const closeBoth = (ws: ServerWebSocket<WsConnData>, code: number, reason: string): void => {
		const d = ws.data;
		if (d.closed) return;
		d.closed = true;
		if (d.dialTimer) clearTimeout(d.dialTimer);
		if (d.drainTimer) clearInterval(d.drainTimer);
		const capped = capWsCloseReason(reason);
		try {
			ws.close(code, capped);
		} catch {
			/* already closed */
		}
		try {
			d.upstream?.close(code, capped);
		} catch {
			// `WebSocket.close` rejects reserved/invalid close codes. Internal
			// callers currently use valid codes, but terminate guarantees the
			// engine leg cannot leak if that invariant ever regresses.
			terminateUpstream(d.upstream);
		}
	};

	const websocket: WebSocketHandler<WsConnData> = {
		// Long-lived actor sockets: ride above Bun's 120s default. Rivet traffic
		// (presence heartbeats ~30s, engine pings) keeps real connections warm;
		// this is the ceiling for genuinely idle tabs, not the mechanism.
		idleTimeout: 960,
		// Keep Bun's native socket buffer and the relay's userland queues under
		// the same hard ceiling. `close` mirrors the failure to the upstream leg.
		maxPayloadLength: DEFAULT_MAX_PAYLOAD_BYTES,
		backpressureLimit: maxBuffered,
		closeOnBackpressureLimit: true,
		sendPings: true,
		// Activity sockets carry tiny revision doorbells. Compression adds CPU
		// and per-connection state without reducing the authoritative HTTP read.
		perMessageDeflate: false,

		open(ws) {
			const d = ws.data;
			d.dialTimer = setTimeout(() => {
				if (!d.upstreamOpen) {
					log.warn('rivet proxy: upstream dial timeout', {
						url: d.upstreamUrl
					});
					closeBoth(ws, 1011, 'upstream dial timeout');
				}
			}, dialTimeoutMs);

			let upstream: WebSocket;
			try {
				// Bun's client WebSocket supports custom headers (Origin forwarding
				// so the actor-level CORS guard sees the browser's real origin) via
				// a nonstandard options bag the DOM lib types don't model.
				const WsClient = WebSocket as unknown as new (
					url: string,
					options: {
						protocols: string[];
						headers: Record<string, string>;
						perMessageDeflate: boolean;
					}
				) => WebSocket;
				const dialHeaders: Record<string, string> = {};
				if (d.origin) dialHeaders.Origin = d.origin;
				// Per-client Guard rate-limit bucket — the bare dial previously made
				// every relayed browser socket count against the api host's single
				// IP. See deriveTrustedClientIp.
				if (d.clientIp) dialHeaders['X-Forwarded-For'] = d.clientIp;
				upstream = new WsClient(d.upstreamUrl, {
					protocols: d.protocols,
					headers: dialHeaders,
					// Rivet control/event frames are small; do not negotiate compression
					// on the private engine leg either.
					perMessageDeflate: false
				});
			} catch (err) {
				log.error('rivet proxy: upstream dial threw', {
					error: err instanceof Error ? err.message : String(err)
				});
				closeBoth(ws, 1011, 'upstream dial failed');
				return;
			}
			d.upstream = upstream;
			upstream.binaryType = 'arraybuffer';

			upstream.addEventListener('open', () => {
				d.upstreamOpen = true;
				if (d.dialTimer) clearTimeout(d.dialTimer);
				d.dialTimer = null;
				// Flush anything the browser sent while the dial was in flight.
				while (d.downQueue.length > 0 && !d.closed) {
					const payload = d.downQueue[0]!;
					if (!canSendUpstreamFrame(upstream, payload, maxBuffered)) break;
					d.downQueue.shift();
					d.downQueuedBytes -= frameBytes(payload);
					upstream.send(payload);
				}
				if (d.downQueue.length === 0) d.downQueuedBytes = 0;
			});

			upstream.addEventListener('message', (event) => {
				if (d.closed) return;
				const data = event.data as string | ArrayBuffer;
				const payload = typeof data === 'string' ? data : new Uint8Array(data);
				// Once a frame is queued, later frames must queue behind it even if
				// the native buffer becomes writable before Bun invokes `drain`.
				// Rivet's wire protocol is ordered; bypassing upQueue corrupts it.
				if (d.upQueue.length > 0 || ws.getBufferedAmount() > HIGH_WATER_BYTES) {
					const size = frameBytes(payload);
					if (totalQueuedBytes(d) + size > maxBuffered) {
						log.warn('rivet proxy: browser-leg buffer cap exceeded, closing', {
							queuedBytes: totalQueuedBytes(d)
						});
						closeBoth(ws, 1013, 'backpressure limit');
						return;
					}
					d.upQueue.push(payload);
					d.upQueuedBytes += size;
					return;
				}
				const sent = ws.send(payload);
				if (sent === 0) {
					closeBoth(ws, 1011, 'browser send failed');
				}
			});

			upstream.addEventListener('close', (event) => {
				if (d.closed) return;
				d.closed = true;
				if (d.dialTimer) clearTimeout(d.dialTimer);
				if (d.drainTimer) clearInterval(d.drainTimer);
				try {
					// Mirror the engine's close so the client's structured reconnect
					// logic sees the real reason (engine redeploy, etc.). Bun 1.4 keeps
					// standard close codes and the reason intact across the client leg.
					ws.close(normalizedCloseCode(event.code, 1011), event.reason || 'upstream closed');
				} catch {
					/* already closed */
				}
			});

			upstream.addEventListener('error', (event) => {
				// A guard-rejected upgrade (HTTP 429 before any WS close frame)
				// lands here — carry whatever detail Bun surfaces so a rate-limit
				// rejection is distinguishable from an engine outage in both the
				// log and the browser-visible close reason. (An upgrade the engine
				// ACCEPTS then closes with "guard.rate_limit#…" takes the close
				// handler above instead, which already mirrors the reason.)
				const detail =
					(event as { message?: unknown }).message ??
					(event as { error?: { message?: unknown } }).error?.message;
				const message = typeof detail === 'string' ? detail : '';
				log.warn('rivet proxy: upstream WS error', {
					url: d.upstreamUrl,
					error: message || undefined
				});
				// closeBoth byte-caps the reason to the WS 123-byte limit.
				closeBoth(ws, 1011, message ? `upstream error: ${message}` : 'upstream error');
			});
		},

		message(ws, message) {
			const d = ws.data;
			if (d.closed) return;
			const payload = message;
			const upstream = d.upstream;
			if (
				d.upstreamOpen &&
				upstream &&
				d.downQueue.length === 0 &&
				canSendUpstreamFrame(upstream, payload, maxBuffered)
			) {
				upstream.send(payload);
				return;
			}
			const size = frameBytes(payload);
			if (wouldExceedUpstreamBudget(d, payload, maxBuffered)) {
				log.warn('rivet proxy: upstream-leg buffer cap exceeded, closing', {
					bufferedBytes: (d.upstream?.bufferedAmount ?? 0) + totalQueuedBytes(d)
				});
				closeBoth(ws, 1013, 'backpressure limit');
				return;
			}
			d.downQueue.push(payload);
			d.downQueuedBytes += size;
			// Browser-style WebSocket has no drain event — poll bufferedAmount and
			// flush when the upstream catches up.
			if (!d.drainTimer) {
				d.drainTimer = setInterval(() => {
					if (d.closed || !d.upstreamOpen || !d.upstream) return;
					if (d.upstream.bufferedAmount > LOW_WATER_BYTES) return;
					while (d.downQueue.length > 0) {
						const queued = d.downQueue[0]!;
						if (!canSendUpstreamFrame(d.upstream, queued, maxBuffered)) break;
						d.downQueue.shift();
						d.downQueuedBytes -= frameBytes(queued);
						d.upstream.send(queued);
					}
					if (d.downQueue.length === 0) {
						d.downQueuedBytes = 0;
						if (d.drainTimer) clearInterval(d.drainTimer);
						d.drainTimer = null;
					}
				}, UPSTREAM_DRAIN_POLL_MS);
			}
		},

		close(ws, code, reason) {
			const d = ws.data;
			if (d.closed) return;
			d.closed = true;
			if (d.dialTimer) clearTimeout(d.dialTimer);
			if (d.drainTimer) clearInterval(d.drainTimer);
			try {
				// 1005/1006/1015 are receive-only status codes. An abrupt browser
				// disappearance must still tear down the engine leg, so normalize it
				// to Going Away rather than passing an invalid code to Bun's client.
				d.upstream?.close(
					normalizedCloseCode(code, 1001),
					capWsCloseReason(reason || 'client closed')
				);
			} catch {
				terminateUpstream(d.upstream);
			}
		},

		// Bun server leg drained below its watermark → flush held upstream frames.
		drain(ws) {
			const d = ws.data;
			if (d.closed) return;
			// Cork batches a burst of queued frames into one native write cycle.
			ws.cork(() => {
				while (d.upQueue.length > 0 && ws.getBufferedAmount() <= LOW_WATER_BYTES) {
					const payload = d.upQueue.shift() as string | Uint8Array;
					d.upQueuedBytes -= frameBytes(payload);
					const sent = ws.send(payload);
					if (sent === 0) {
						closeBoth(ws, 1011, 'browser send failed');
						return;
					}
					if (sent === -1) break;
				}
			});
			if (d.upQueue.length === 0) d.upQueuedBytes = 0;
		}
	};

	const handleRequest = (
		req: Request,
		server: Server<WsConnData>
	): Response | undefined | Promise<Response | undefined> => {
		const url = new URL(req.url);
		const isUpgrade = req.headers.get('upgrade')?.toLowerCase() === 'websocket';
		const route = classifyProxyPath(url.pathname, req.method, isUpgrade, prefix);

		if (route.kind === 'deny') {
			// Proxy-level denial (never forwarded): admin surfaces share the Guard
			// port — /namespaces, /runner-configs, /datacenters, the dashboard.
			return deny(404, 'not_found');
		}
		if (route.kind === 'metadata') return metadataStub();

		const origin = req.headers.get('origin');
		if (!isOriginAllowed(origin, opts.allowedOrigins)) {
			log.warn('rivet proxy: origin rejected', { origin, path: url.pathname });
			return deny(403, 'origin_not_allowed');
		}

		const clientIp = deriveTrustedClientIp(
			req.headers.get('x-forwarded-for'),
			server.requestIP(req)?.address ?? null
		);

		if (route.kind === 'http') {
			return passthroughHttp(req, route.upstreamPath, clientIp);
		}

		// WS relay. Offered subprotocols are the rivet wire handshake (encoding,
		// conn params carrying the Layerr HMAC actor token) — forward verbatim;
		// they are opaque bytes to us.
		const offered = (req.headers.get('sec-websocket-protocol') ?? '')
			.split(',')
			.map((p) => p.trim())
			.filter(Boolean);
		const upgradeHeaders: Record<string, string> = {};
		if (offered.includes('rivet')) {
			upgradeHeaders['Sec-WebSocket-Protocol'] = 'rivet';
		}
		const upgraded = server.upgrade(req, {
			data: {
				upstreamUrl: `${upstreamWs}${route.upstreamPath}${url.search}`,
				protocols: offered,
				origin,
				clientIp,
				upstream: null,
				upstreamOpen: false,
				downQueue: [],
				downQueuedBytes: 0,
				upQueue: [],
				upQueuedBytes: 0,
				closed: false,
				dialTimer: null,
				drainTimer: null
			},
			headers: upgradeHeaders
		});
		if (!upgraded) {
			return deny(404, 'upgrade_failed');
		}
		return undefined;
	};

	return { handleRequest, websocket };
}
