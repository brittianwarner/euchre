/**
 * Local serverless development and the production Railway envoy share actors.
 * server/index.ts starts the production registry against a private engine.
 * Durable state belongs in c.state/c.kv/c.db, never c.vars or the app filesystem.
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
	shutdown: isLocalDev ? undefined : { disableSignalHandlers: true, gracePeriodMs: 90000 },
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

	// Only development configures a serverless callback. Production uses an envoy.
	startEngine: isLocalDev ? true : undefined,
	startServices: false,
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
