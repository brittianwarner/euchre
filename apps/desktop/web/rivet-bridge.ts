/**
 * The Rune web build's one door to Rivet (web plan S4).
 *
 * It wraps the official rivetkit 3.1.0 browser client, configured exactly as
 * `apps/web/src/lib/client/rivet.ts` configures it: JSON encoding, `<origin>/rivet`, and
 * `getToken` + `getParams` from `POST /api/table-session/{gameId}` with the browser's own
 * cookies. Rust (`src/net/web/table.rs`) sees five calls and three callbacks, all strings.
 *
 * `bun run bridge` bundles this file to `js/rivet-bridge.js`. wasm-bindgen reads that bundle
 * at compile time, so it must exist before `cargo build --target wasm32-unknown-unknown`.
 */
import { createClient, type ActorConnRaw } from 'rivetkit/client';

/** The table actor's events (`euchre_protocol::TABLE_EVENT_NAMES`). */
const EVENTS = ['sync', 'thinking', 'chat', 'chatDelta', 'presence'] as const;

/** `POST /api/table-session/{gameId}` (`euchre_protocol::TableSessionResponse`). */
interface TableSession {
	actorId: string;
	token: string;
	gatewayToken?: string | null;
	expiresAt: number;
}

/** What Rust reads from every failure (`rivet_error::BridgeError`). */
interface ErrorShape {
	group: string | null;
	code: string | null;
	message: string;
	metadata: unknown;
	status: number | null;
}

/** rivetkit's `ActorConnStatus`. */
type Status = 'idle' | 'connecting' | 'connected' | 'disconnected';

/** The handle `openTable` returns. */
export interface TableBridge {
	/** Resolves to the actor id once the session is minted; rejects with error JSON. */
	ready(): Promise<string>;
	/** Resolves to the action's result as JSON text; rejects with error JSON. */
	action(name: string, argsJson: string): Promise<string>;
	/** rivetkit's connection status now. */
	status(): Status;
	/** Closes the socket and the client. Safe to call twice. */
	dispose(): Promise<void>;
}

/** A failed table-session mint. */
class SessionError extends Error {
	readonly group = 'session';
	readonly code: string;
	readonly status: number;

	constructor(status: number) {
		super(`table-session ${status}`);
		this.code = `http_${status}`;
		this.status = status;
	}
}

function errorShape(err: unknown): ErrorShape {
	const e = (err ?? {}) as Partial<Record<keyof ErrorShape, unknown>>;
	return {
		group: typeof e.group === 'string' ? e.group : null,
		code: typeof e.code === 'string' ? e.code : null,
		message: String(e.message ?? err),
		metadata: e.metadata ?? null,
		status: typeof e.status === 'number' ? e.status : null
	};
}

function errorJson(err: unknown): string {
	return JSON.stringify(errorShape(err));
}

/** A refused token: the cached session must be minted again before the next dial. */
function refusesToken(err: unknown): boolean {
	const code = errorShape(err).code;
	return code === 'forbidden' || code === 'invalid_token';
}

/**
 * Opens one table. `onEvent(name, json)` for each table event, `onStatus(status)` on every
 * rivetkit status change (and once at the start), `onError(json)` for connection errors and
 * for a refused re-mint during a reconnect (`{group: "session", code: "http_<status>"}`).
 */
export function openTable(
	origin: string,
	gameId: string,
	onEvent: (name: string, json: string) => void,
	onStatus: (status: string) => void,
	onError: (json: string) => void
): TableBridge {
	let session: TableSession | undefined;
	let pending: Promise<TableSession> | undefined;
	let disposed = false;
	// Cached until 10 s before expiry, single-flight, refreshed on `forceRefresh` (rivet.ts).
	const credentials = (forceRefresh = false): Promise<TableSession> => {
		if (!forceRefresh && session && Date.now() < session.expiresAt - 10_000) {
			return Promise.resolve(session);
		}
		return (pending ??= fetch(`${origin}/api/table-session/${encodeURIComponent(gameId)}`, {
			method: 'POST',
			credentials: 'same-origin',
			headers: { 'Content-Type': 'application/json' }
		})
			.then(async (response) => {
				if (!response.ok) {
					const err = new SessionError(response.status);
					// A refused re-mint is final for this table (403/404/400/401); Rust ends
					// the connection with the matching error.
					onError(errorJson(err));
					throw err;
				}
				session = (await response.json()) as TableSession;
				return session;
			})
			.finally(() => {
				pending = undefined;
			}));
	};
	const client = createClient({
		endpoint: `${origin}/rivet`,
		encoding: 'json',
		devtools: false,
		gateway: { skipReadyWait: true },
		getToken: async ({ forceRefresh }: { forceRefresh: boolean }) => {
			const value = await credentials(forceRefresh);
			if (!value.gatewayToken) throw new Error('Missing table gateway token');
			return value.gatewayToken;
		}
	});
	let conn: ActorConnRaw | undefined;
	const ready = credentials().then((s) => {
		if (disposed) throw { group: 'client', code: 'closed', message: 'disposed while opening' };
		// The client is untyped (no registry here); the raw connection has every call we use.
		const c = client
			.getForId('euchreTable', s.actorId, {
				getParams: async () => ({ token: (await credentials()).token })
			})
			.connect() as unknown as ActorConnRaw;
		conn = c;
		for (const name of EVENTS) {
			c.on(name, (payload: unknown) => onEvent(name, JSON.stringify(payload ?? null)));
		}
		c.onStatusChange((status: string) => onStatus(String(status)));
		c.onError((err: unknown) => {
			if (refusesToken(err)) session = undefined;
			onError(errorJson(err));
		});
		onStatus(String(c.connStatus));
		return s.actorId;
	});
	// `ready()` may never be awaited (a dispose during the mint): keep the rejection handled.
	ready.catch(() => {});
	return {
		ready: () => ready.catch((err: unknown) => Promise.reject(errorJson(err))),
		action: async (name, argsJson) => {
			await ready.catch(() => {});
			// rivetkit would queue a call made while the socket is down and send it late.
			if (disposed || !conn || conn.connStatus !== 'connected') {
				throw errorJson({ group: 'client', code: 'closed', message: 'not connected' });
			}
			try {
				const out = await conn.action({ name, args: JSON.parse(argsJson) as unknown[] });
				return JSON.stringify(out ?? null);
			} catch (err) {
				if (refusesToken(err)) session = undefined;
				throw errorJson(err);
			}
		},
		status: () => (conn ? (String(conn.connStatus) as Status) : 'idle'),
		dispose: async () => {
			if (disposed) return;
			disposed = true;
			await ready.catch(() => {});
			try {
				await conn?.dispose();
			} catch {
				/* already closed */
			}
			try {
				await client.dispose();
			} catch {
				/* already closed */
			}
		}
	};
}
