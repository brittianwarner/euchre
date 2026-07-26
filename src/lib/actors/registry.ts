/**
 * Rivet actor registry.
 *
 * This registry is mounted INSIDE the SvelteKit app at `/api/rivet/*`
 * (see `src/routes/api/rivet/[...all]/+server.ts`). There is no separate
 * backend process — one deployable, running in Rivet's **serverless** runtime
 * mode against Rivet Cloud.
 *
 * Topology reminder: the browser opens its WebSocket directly to Rivet Cloud.
 * Rivet Cloud then issues ordinary HTTPS requests back to `/api/rivet/*` on
 * this deployment (`GET /api/rivet/metadata` to validate config, and
 * `GET /api/rivet/start` to run an actor). Nothing here terminates a socket.
 *
 * Endpoints are configured purely by environment variable and read by RivetKit
 * at runtime — see `.env.example`:
 *   - `RIVET_ENDPOINT`        (secret, `sk_` token) — where this backend finds the engine
 *   - `RIVET_PUBLIC_ENDPOINT` (publishable, `pk_` token) — what `/api/rivet/metadata`
 *     hands back to browsers so they know where to open their WebSocket
 *
 * Without `RIVET_ENDPOINT`, RivetKit falls back to its filesystem driver, which
 * fails on Vercel's read-only filesystem.
 *
 * Persistence rule for every actor added here: durable data lives in
 * `c.state` (small, bounded, CBOR-serializable), `c.kv` (unbounded key/value),
 * or `c.db` (actor-local SQLite). Never `c.vars` — that is wiped on sleep,
 * restart, crash, and every serverless function migration. There is no external
 * database in this project.
 *
 * @see https://rivet.dev/docs/general/runtime-modes
 * @see https://rivet.dev/docs/general/endpoints
 */

import { actor, setup } from 'rivetkit';
import { aiSeat } from './ai-seat';
import { euchreTable } from './euchre-table';
import { playerProfile } from './player-profile';

/**
 * Placeholder actor that exists so the Rivet wiring is testable before any
 * real euchre actors land. Real game actors (table, hand, agents) replace or
 * join this in later milestones.
 *
 * The `pings` counter is not decorative: if it survives across calls, the
 * whole path (browser → Rivet Cloud → `/api/rivet/start` → actor state
 * persistence) is proven working end to end.
 */
export const health = actor({
	options: {
		name: 'Health',
		icon: 'heart-pulse'
	},
	state: { pings: 0 },
	actions: {
		/** Liveness probe. Returns `{ ok: true, ts }`. */
		ping: (c): { ok: true; ts: number } => {
			c.state.pings += 1;
			return { ok: true, ts: Date.now() };
		}
	}
});

/**
 * Local-dev only: where the bundled engine should call back to reach us.
 *
 * In serverless mode the engine drives everything — it calls
 * `GET <url>/start` to run an actor. Rivet Cloud learns that URL from the
 * provider configured in the dashboard (or by the preview-namespace GitHub
 * action). The engine RivetKit spawns on :6420 during `bun run dev` has no
 * such configuration and no way to guess the Vite port, so without this every
 * actor call fails with `actor_ready_timeout` from the guard.
 *
 * Override with `RIVET_DEV_SERVERLESS_URL` if you run Vite on another port.
 * (That is our env var, not a RivetKit builtin.)
 */
// Prefer 127.0.0.1 over localhost — Vite is often bound to IPv4 only, and
// `localhost` resolving to ::1 makes every actor wake time out.
const DEV_SERVERLESS_URL = 'http://127.0.0.1:5173/api/rivet';

/** True when no Rivet Cloud endpoint is configured, i.e. `bun run dev`. */
const isLocalDev = !process.env.RIVET_ENDPOINT;

export const registry = setup({
	use: {
		health,
		// M4: the authoritative table, one actor per AI opponent (`aiSeat`, key
		// ["table", gameId, "seat", "1"|"2"|"3"]), and the durable per-user profile
		// (`playerProfile`, key ["user", userId]). The table dispatches a decision
		// request to the seat that owns the acting chair and receives its reply
		// over its own `aiDecision` queue; it never trusts the reply's `seat` or
		// `turnId` without re-deriving both.
		euchreTable,
		aiSeat,
		playerProfile
	},

	// Both of the following are local-dev only. In production `RIVET_ENDPOINT`
	// points at Rivet Cloud, whose provider URL is the deployed Vercel origin —
	// configured in the Rivet dashboard, never hardcoded here.
	//
	// `configurePool` is rejected by RivetKit unless paired with `startEngine`
	// or an explicit endpoint ("configurePool requires either endpoint or
	// startEngine"), hence both flip together.
	startEngine: isLocalDev ? true : undefined,
	configurePool: isLocalDev
		? { url: process.env.RIVET_DEV_SERVERLESS_URL ?? DEV_SERVERLESS_URL }
		: undefined
});

/**
 * Registry type used to type the client on both sides:
 * `createClient<Registry>(...)` on the server, and the `@rivetkit/svelte`
 * context in the browser.
 */
export type Registry = typeof registry;
