# 06 — Revised Architecture (Architecture Decision Record)

**Status:** binding, and superior in authority to `00-OVERVIEW.md` and `01-ARCHITECTURE.md` on every
point it names. **Date:** 2026-07-26. **Supersedes:** parts of `00` and `01`, itemised in §1.

`00` and `01` were written against a two-process, self-hosted, Postgres-backed deployment. That
deployment is not what we are building. This document corrects the *hosting, runtime mode, repository
layout, persistence, auth storage and client transport* decisions, and leaves everything else — the
euchre rules, the actor decomposition, the redaction boundary, the durability ordering, the AI
ladder, the frontend design — exactly as `00`–`05` specify.

Two claims made earlier in this project were factually wrong and shaped `01` §1 and §11. They are
retracted in §7, with the correct mechanism written down, so that nobody re-derives them.

**Reference implementation.** Everything asserted here about SvelteKit + RivetKit + Vercel is
demonstrated by a real, deployed application on disk at `/Users/brittianwarner/goods/rivet-game-svelte`.
Where this document and a documentation snapshot disagree, the working deployment wins.

---

## 1. Status ledger — what is superseded, what still stands

Read this section as the diff. Everything not listed as superseded is still binding exactly as
written in `00`/`01`.

### 1.1 `00-OVERVIEW.md` — superseded

| Section | The old decision (one line) | The new decision |
|---|---|---|
| §4 Dependencies, row `pnpm` | "Workspace manager. Default nested `node_modules` is required." | **No workspace.** One package, one `package.json`, `bun` 1.3.14 for every install. The nested-linker requirement evaporates with Postgres (§4.5). |
| §4, row `bun` | "`apps/game` runtime; `apps/web` stays on node 24.11.0 under `adapter-node`." | `bun` is the package manager and local dev runtime for **the one app**. Vercel builds and runs it on node 24. |
| §4, row `@sveltejs/adapter-node` | "Plain node server." | **`@sveltejs/adapter-vercel`** (already in devDependencies). |
| §4, rows `@better-auth/drizzle-adapter`, `postgres`, and `drizzle-orm`'s Postgres role | "Postgres (Better Auth) + actor-local SQLite." | Postgres is gone. `drizzle-orm` survives **only** as `rivetkit/db/drizzle` over actor SQLite. `postgres@3.4.9` is deleted. |
| §4 install commands | `pnpm --filter @euchre/game add …`, `pnpm --filter @euchre/web add …` | `bun add …` at the repo root. `@rivetkit/svelte` is `"file:./rivetkit-svelte"` — it is not published on npm. |
| §4 `.npmrc` note + the `pnpm why drizzle-orm` CI gate | "nested `node_modules` is load-bearing." | Deleted. There is one `drizzle-orm`, and the CI gate that asserted two is removed. |
| §5 Repository file tree | `packages/euchre-core` + `packages/protocol` + `apps/game` + `apps/web` | **One SvelteKit app.** The registry, the actors and the rules engine all live under `src/lib/`. Rule-engine purity is preserved by directory boundary and lint, not by package boundary (§2.4). |
| §5 spec note ("existing scaffold") | "M0 deletes it: remove `src/`, `static/`, `bun.lock`, `svelte.config.*`, `vite.config.ts`, `tsconfig.json`…" | **Retracted. The scaffold is kept and extended.** It already has `adapter-vercel`, `bun.lock` and a working SvelteKit build — those are now the right answers, not the wrong ones. |
| §6 Environment variables | `DATABASE_URL`, `PUBLIC_RIVET_ENDPOINT`, `APP_URL`, `RIVET_NAMESPACE`, `RIVET_TOKEN`, `RIVET_RUNNER_VERSION`, `RIVET_ENVOY_VERSION`, `RIVETKIT_STORAGE_PATH` | `RIVET_ENDPOINT` and `RIVET_PUBLIC_ENDPOINT` in URL-auth form, plus `BETTER_AUTH_SECRET`, `BETTER_AUTH_URL`, `RESEND_API_KEY`, `EMAIL_FROM`, `ANTHROPIC_API_KEY`, `ALLOWED_ORIGINS`. One process, one env file (§3.5). |
| §6 "two processes, two credential sets… `apps/game` never holds a database credential" | The blast-radius split | **Retired as stated.** There is one deployable, so there is one credential set. The security model is restated honestly in §5.6: it is now *token scope and actor-level authorization*, not process separation. |
| §7.2 "Postgres host for the Better Auth tables — Default: Neon over `postgres-js`" | Neon | **Retired outright.** See §5. |
| §7.4 "Deployment target — Default: long-lived Bun runner… Not serverless." | Runner mode | **Serverless mode on Vercel against Rivet Cloud.** The stated reason for runner mode ("keeps a live hand from being drained mid-trick") does not survive contact with how drains actually work — see §3.4. |
| §8 Running it locally | Two processes, `pnpm dev`, ports 5173 + 6420, `concurrently` | One process: `bun run dev`, port 5173. There is no gateway on 6420. |

### 1.2 `00-OVERVIEW.md` — still stands

- **§1 What we are building**, **§2 the design competition and why B won**, **§9 the glossary** — untouched.
- **§3 Binding invariants 1–23** all stand, with three amendments and no deletions:
  - **Invariant 16** (auth on every browser-reachable actor) stands in full — origin allowlist in
    `onBeforeConnect`, JWT in `createConnState`, per-user key assertion, auth via connection params
    and never a static `params`. Amended only in mechanism: the JWKS is resolved **in-process** via
    the `authStore` actor rather than over HTTP to a second deployment (§5.5), and on the client the
    params getter is `withActorParams()` from `@rivetkit/svelte` rather than a hand-written
    `getParams` (§6). The requirement is unchanged; the plumbing is.
  - **Invariant 20** ("bound everything") is *extended*: every `c.kv` key namespace must now carry a
    documented retention rule, for the same reason every `c.state` collection carries a cap. `c.kv`
    is unbounded by design, which is exactly how a cost line accumulates silently (§4.4).
  - **Invariant 22** ("no number is frozen before it is measured") now also governs
    `requestLifespan` and the migration-boundary latency (§3.3, §8).
- **§7.1** (Anthropic, two model tiers, `modelParams()`), **§7.3** (banter on by default), **§7.5**
  (`gameTo: 10`, `stickTheDealer: true`) — untouched.

### 1.3 `01-ARCHITECTURE.md` — superseded

| Section | The old decision (one line) | The new decision |
|---|---|---|
| §1 Topology + diagram | "Two long-lived server processes plus the browser… the browser's WebSocket goes to the Rivet gateway directly." | One deployable. The browser's WebSocket goes to **Rivet Cloud**, which calls **us** over HTTPS. Diagram in §2. The half of the old sentence that said *SvelteKit is never on the realtime path and never proxies an `Upgrade`* **is still true** — see §2.3. |
| §2 The registry | `apps/game/src/registry.ts`, `registry.start()`, `envoy: { version: BUILD_VERSION, poolName }`, "deliberately **not** mounted inside SvelteKit". | `src/lib/actors/registry.ts`, `setup({ use })` with no `envoy` block, mounted **inside** SvelteKit via `registry.handler()` in a catch-all route (§3.1). |
| §2 spec note on `envoy.version` / `RIVET_RUNNER_VERSION` / `RIVET_ENVOY_VERSION` and `build-version.ts` | "a monotonic build integer feeding `envoy.version`." | Dropped. `envoy`/runner-pool configuration is runner-mode machinery and does not apply to a serverless runner. Version boundaries come from Vercel's immutable per-deployment URLs and Rivet Cloud's runner config. **Open item 8.1** tracks confirming the roll semantics with Rivet Cloud. |
| §3.1 Storage policy, row `c.kv` | "**not used anywhere** — Deprecated in 2.3.9; the type definition says so verbatim. Any use is a review reject." | **Wrong, and retracted.** `c.kv` is a supported, documented low-level KV store and is now a first-class tier. Revised table in §4.2. |
| §4 Lifecycle, `CREATE` | `apps/web/src/routes/play/+page.server.ts` | `src/routes/(app)/play/+page.server.ts`. The rule that **the server creates the game and the browser never does** is unchanged. |
| §7 Connection authentication | `createClient(PUBLIC_RIVET_ENDPOINT)` + hand-written `getParams`; `createRemoteJWKSet` over HTTP to a second deployment. | `@rivetkit/svelte` shared client at `${origin}/api/rivet` + `withActorParams()`; JWKS fetched once per actor wake from the in-process `authStore` and verified with `createLocalJWKSet` (§5.5). |
| §11 Deployment (whole section) | Long-lived Bun runner + `adapter-node`, "deliberately **not** serverless", `drainOnVersionUpgrade`, `pnpm why drizzle-orm` gate. | Replaced by §3 and §8 of this document. |

### 1.4 `01-ARCHITECTURE.md` — still stands, and matters more than before

None of the following changes. Several become *load-bearing* under serverless, which is called out
where it applies.

- **§3.2 `euchreTable`** — the whole of it. Actions (all read-only except `submitMove`), the queue
  set with `turnId`/`seq`/`internalToken` on every message, `canPublish` is not the boundary and the
  constant-time `internalToken` compare is, the event set, and the schedule table.
- **§3.3 `aiSeat`** — one actor per opponent, `run` as a `workflow()`, the six protected step names,
  the workflow rule that actor data is reachable only inside `ctx.step(...)`. Workflow replay
  history is what makes a mid-decision migration free rather than re-billed — see §3.4.
- **§3.4 `playerProfile`** — key, tiny `c.state`, `c.db` for everything paginated, `getReplay`
  re-deriving frames server-side.
- **§5 the actor-to-actor call graph and the circular-inference caveat** — `import type` only for
  `registry` inside actor files, and **explicit return-type annotations on every cross-called
  action**. This gets *more* dangerous now, not less: the registry and the actors now share a module
  graph with SvelteKit's `$lib` resolution, so a value import of `registry` from an actor file can
  produce `undefined` at module-init inside a Vite dev transform as well as at runtime.
- **§6 the realtime protocol and redaction, in full.** `project()` is the only producer of
  `PublicGameView`; `projectSteps()` is the only gate on the animation channel; `pushSync` is the
  only sender of card identity; `no-broadcast-state` and the 10,000-game redaction fuzz stay
  required CI gates. **Nothing about redaction changes.** `conn.send` and `c.broadcast` behave
  identically in serverless mode — see §2.2.
- **§8 Durability, in full.** `saveState({ immediate: true })` between the mutation and *both* the
  fan-out and the ack, on every queue handler; `save-before-complete` stays a build gate. The crash
  table in §8 stands and now doubles as the **drain** table (§3.4).
- **§9 Failure and recovery, in full**, including §9.1's `onWake` reconciler — which stops being a
  rarely-exercised safety net and becomes a routine code path, because a serverless migration is a
  scheduled wake. `recovery.spec.ts` is therefore a higher-value test than it was.
- **§9.4 Server-held tempo is a security property** — unchanged, and it is also what absorbs
  migration latency (§3.4).
- **§10 Scaling and cost** — the per-actor limits, the ~700–840 calls per game ceiling, the
  `legal.length === 1` short-circuit, the 4096-token Haiku cache floor, and "no number is frozen
  before it is measured" all stand.

---

## 2. The corrected topology

### 2.1 The diagram

```
   ┌───────────────────────────────── BROWSER ──────────────────────────────────┐
   │  SvelteKit app served from Vercel, hydrated, Svelte 5 runes                │
   │                                                                            │
   │   Threlte / three.js scene ◄──── table.svelte.ts ────► @rivetkit/svelte    │
   │   HandA11y DOM fallback           ($state store)        useActor / handle  │
   └────────┬─────────────────────────────────────────────────────┬─────────────┘
            │                                                     │
            │ ①  HTTPS  page loads, form actions,                 │ ③  WSS
            │    POST /api/rivet-token,                           │    the ONE realtime
            │    GET  /api/rivet/metadata                         │    socket in the system
            ▼                                                     ▼
   ┌───────────────────────────────────┐              ┌──────────────────────────────┐
   │ VERCEL · @sveltejs/adapter-vercel │              │        RIVET CLOUD           │
   │ vercel.json {"framework":         │              │      api.rivet.dev           │
   │              "sveltekit"}         │              │                              │
   │                                   │              │  gateway  ·  scheduler       │
   │  /api/auth/*      → authStore     │              │  durable state · KV · SQLite │
   │  /api/rivet-token → 15-min JWT    │              │                              │
   │  /api/rivet/[...all]              │◄─────────────┤ ②  HTTPS, ordinary requests  │
   │        → registry.handler(request)│   GET /api/  │    Rivet Cloud calls US.     │
   │                                   │   rivet/start│    Long-lived request,        │
   │  ┌─────────────────────────────┐  │   (one per   │    one per running actor.     │
   │  │ registry  = setup({ use })  │  │    actor)    │    NO Upgrade header.         │
   │  │   euchreTable  ["table",gid]│  │              └──────────────┬───────────────┘
   │  │   aiSeat  ×3   [...,"seat"] │  │                             │
   │  │   playerProfile ["user",uid]│  │                             │ hibernating WS
   │  │   authStore     ["auth"]    │  │                             │ live-migrated
   │  └──────────────┬──────────────┘  │                             │ across ② requests
   └─────────────────┼─────────────────┘                             │
                     │ HTTPS · ai@7.0.37                             │
                     ▼                                     (client sees no interruption)
        ┌────────────────────────────┐
        │ Anthropic API              │
        │ claude-haiku-4-5  (plays)  │
        │ claude-opus-5     (bids)   │
        └────────────────────────────┘
```

### 2.2 Read the arrows

1. **① The browser talks to Vercel over ordinary HTTPS** for pages, form actions, the
   `POST /api/rivet-token` mint, and one `GET /api/rivet/metadata` at boot. `metadata` is plain
   JSON: `{ clientEndpoint, clientNamespace, clientToken }`, populated from `RIVET_PUBLIC_ENDPOINT`.
   That is the entire purpose of pointing the RivetKit client at `${origin}/api/rivet` — it is a
   *discovery* endpoint, not a proxy.

2. **③ The browser opens its WebSocket to Rivet Cloud**, at the `clientEndpoint` the metadata call
   returned, using the publishable (`pk_`) token. Rivet Cloud's gateway terminates that socket.

3. **② Rivet Cloud then calls us.** For each actor that needs to run, the engine opens a long-lived
   ordinary HTTPS request to `GET /api/rivet/start` on our Vercel deployment. Our catch-all route
   hands the standard `Request` to `registry.handler()` and returns a standard `Response`. The actor
   executes inside that function invocation. Actor output — `conn.send`, `c.broadcast` — travels
   back to Rivet Cloud over that request's response stream, and Rivet Cloud fans it out over the
   browser's socket. This is why §6 of `01` needs no change whatsoever: from inside the actor,
   `c.conns`, `conn.send('sync', …)` and `c.broadcast` are byte-identical to runner mode.

4. `RIVET_ENDPOINT` (the secret `sk_` URL) is what lets our deployment reach the engine and, more
   importantly, is what the engine uses to authenticate that a `/api/rivet/start` request came from
   *our* trusted Rivet endpoint and not from someone else's self-hosted engine. Without it, RivetKit
   silently falls back to a filesystem state driver and dies on Vercel's read-only filesystem with
   `ENOENT: no such file or directory, mkdir '.../state'`. §3.5 makes that a boot-time assertion
   rather than a production incident.

### 2.3 The two things that must be unmistakable

> **Our deployment never terminates a WebSocket.**
> There is no `Upgrade` handling anywhere in our code. There is no `ws`, no `@hono/node-ws`, no
> `onWebSocket` handler, no upgrade proxy. `adapter-vercel` is never asked to do anything but
> `Request → Response`. The single WebSocket in the system runs between the browser and Rivet
> Cloud, and neither end of it is ours.

> **The engine calls us; we do not call the engine to run actors.**
> Runner mode is a persistent outbound connection we hold open. Serverless mode is the inverse: the
> engine holds our URL (`serverless.url` in Rivet Cloud's runner config) and issues inbound HTTPS
> requests. Our deployment is a *handler*. It has no boot step, no daemon, no SIGTERM ownership, and
> nothing to keep alive between requests. `registry.start()` must not appear anywhere in this
> codebase.

### 2.4 What "one deployable" costs, honestly

`00` §3 invariant 1 says every euchre rule lives in one place and nothing else may write one. That
was enforced partly by `packages/euchre-core` being a separate package with **zero** runtime
dependencies — a boundary the module resolver enforced for free. Collapsing to one app gives that
up. The replacement is weaker and must therefore be explicit:

- `src/lib/euchre/**` may import from `src/lib/euchre/**` and nothing else. No `rivetkit`, no
  `three`, no `$app/*`, no `$env/*`, no `svelte`.
- An ESLint `no-restricted-imports` rule with a zone for that directory enforces it, and joins the
  four custom rules already in `00` §3.
- The rule-engine test suite continues to run with no network, no API key and no RivetKit import.
  If it ever needs one, the boundary has already been breached.

This is a real downgrade in enforcement strength and is recorded in `docs/LEDGER.md` as such. It
buys a single build, a single deploy, a single lockfile, and the removal of the entire pnpm
nested-linker apparatus that only existed to reconcile two `drizzle-orm` majors that no longer both
exist.

---

## 3. Runtime mode: serverless via `registry.handler()`

### 3.1 The mount point

```ts
// src/routes/api/rivet/[...all]/+server.ts
import { registry } from "$lib/actors/registry";
import type { RequestHandler } from "./$types";

const handle: RequestHandler = async ({ request }) => registry.handler(request);

export const GET = handle;
export const POST = handle;
export const PUT = handle;
export const DELETE = handle;
export const PATCH = handle;
export const OPTIONS = handle;
export const fallback = handle;
```

Exporting `fallback` as well as the six named verbs is not belt-and-braces; SvelteKit will 405 a
verb it has no export for, and the engine is entitled to use any of them. This is verbatim the
reference implementation's file.

```ts
// src/lib/actors/registry.ts
import { setup } from "rivetkit";
import { euchreTable } from "./euchre-table/index.js";
import { aiSeat } from "./ai-seat/index.js";
import { playerProfile } from "./player-profile/index.js";
import { authStore } from "./auth-store/index.js";

export const registry = setup({
  use: { euchreTable, aiSeat, playerProfile, authStore },
  maxIncomingMessageSize: 1_048_576,
});
```

No `endpoint`, `token` or `namespace` fields: `RIVET_ENDPOINT` and `RIVET_PUBLIC_ENDPOINT` carry all
three each, in `https://namespace:token@api.rivet.dev` URL-auth form, and RivetKit reads them from
the environment. No `envoy` block — that is runner-mode configuration.

```json
// vercel.json
{ "framework": "sveltekit" }
```

```js
// svelte.config.js
import adapter from "@sveltejs/adapter-vercel";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  preprocess: vitePreprocess(),          // plain — NEVER vitePreprocess({ script: true })
  kit: { adapter: adapter() },
};
```

The two endpoints RivetKit exposes under that catch-all are `GET /api/rivet/metadata` (validates
configuration, returns client connection info) and `GET /api/rivet/start` (runs an actor). Neither is
ever called by our own code. `GET /api/rivet/metadata` returning `200` with a populated
`clientEndpoint` is the pre-deploy smoke check.

### 3.2 `requestLifespan`

Each `/api/rivet/start` request has a configured lifespan — the engine's promise about how long it
will hold that request open before recycling it. It is set in **Rivet Cloud's runner config**, not in
our code:

```jsonc
// Rivet Cloud → runner config → datacenters.<dc>.serverless
{
  "url": "https://<deployment>/api/rivet",
  "requestLifespan": 3600,     // seconds; MUST NOT exceed the Vercel function max duration
  "minRunners": 0,
  "maxRunners": <n>,
  "slotsPerRunner": <n>
}
```

The binding rule: **`requestLifespan` must be less than or equal to the Vercel function's maximum
duration.** If it is longer, Vercel kills the invocation before Rivet's drain sequence begins, and
the migration becomes an unplanned crash instead of a planned one — still survivable (§3.4), but it
converts a graceful handoff into a redelivery. The exact value depends on the Vercel plan and is an
**open item** (§8.2); it is a number, and by `00` §3 invariant 22 no number is frozen before it is
measured.

### 3.3 The drain grace period

When a request nears its lifespan, the engine reserves a grace window at the end and begins stopping
actors inside it. With a 3600 s lifespan and the engine's 10 s default fallback, actors begin
stopping at 3590 s. The window is configurable as `drainGracePeriod` in the same runner config, up
to a 30-minute ceiling; the engine-level fallback (`pegboard.serverless_drain_grace_period`,
default 10 s) applies when no per-runner value is set.

During the grace window each actor runs its normal stop path: the run loop is signalled, in-flight
`c.saveState({ immediate: true })` completes, `onSleep` fires, hibernating WebSockets are handed to
the gateway to hold. After the full lifespan elapses the request is closed and anything still
running is rescheduled onto a fresh invocation.

**10 seconds is enough for a euchre table and is not a coincidence.** The longest thing our run loop
does is one reducer call and one immediate save — microseconds and one round trip. Nothing in the
loop awaits an LLM: `01` §5 forbids `wait: true` between actors outright, and the AI decision arrives
as its own inbound queue message. A drain therefore never interrupts a model call, because the table
was never blocked on one.

### 3.4 Migration across invocations, and why the "drained mid-trick" fear was misplaced

`00` §7.4 and `01` §11 both chose runner mode with one stated justification: *"Runner mode is what
keeps a live hand from being drained mid-trick on deploy."* That reasoning contains a hidden premise
— that being drained mid-trick **loses** something. It does not.

Rivet migrates actors between function invocations and preserves the actor across the move.
Everything the spec already relies on for crash recovery is exactly what carries a migration:

| Carried across a migration | Mechanism the spec already specifies |
|---|---|
| `c.state` — the whole `GameState` | `01` §3.1: persisted, loaded whole on every wake. Migration is a wake. |
| `c.db` and `c.kv` | Durable stores in Rivet, not on the function's filesystem. |
| Queue messages, acked or not | `00` §3 invariant 3: every mutation is a durable queue message. Unacked messages are redelivered. |
| Idempotency of a redelivered message | `00` §3 invariant 11 + `01` §8: `appliedSeq` lives in the *same* `c.state` as the mutation it guards, so reapplication is a no-op **by construction, not by luck**. |
| Armed schedules (`onTempoGate`, `onAiTimeout`, `releaseAiMove`, `onNudge`, `onAbandon`) | `01` §3.2: schedules persist across restart, upgrade and crash, and are re-armed on wake. |
| An in-flight AI decision | `01` §3.3: `aiSeat.run` is a `workflow()`. A step that already committed **replays from history and is not re-billed**; a step that had not committed re-runs. |
| The browser's connection | Hibernating WebSockets are held by the gateway and live-migrated. The client sees no interruption and does not reconnect. |
| The one genuinely lossy window (crash between `complete()` and arming the next turn) | `01` §9.1: the `onWake` reconciler, which already exists, is already individually tested, and now runs on every migration rather than only after a crash. |

A drain is a **planned** crash with a warning and a grace period. The spec's recovery table in `01`
§8 was written for **unplanned** crashes at arbitrary instruction boundaries. A planned drain is
strictly the easier case, and it is already covered row by row.

Two further facts make the concern smaller still. First, an actor sleeps after 30 s idle whenever its
run loop is blocked in `iter()` — a single-player euchre table waiting on a human is asleep long
before any lifespan boundary, so the common case has nothing to migrate. Second, the residual cost of
a migration that *does* land mid-hand is **latency**, not correctness: one wake, `onMigrate` →
`createVars` → `onWake`/reconcile, then the loop resumes. That latency lands inside phases the game
already holds open on purpose — the 700–1000 ms `trick_resolve` gate and the 900–1800 ms bid pacing
floor (`00` §3 invariants 15 and 4; `01` §9.4). The system was already designed to be slow at
exactly the moments a migration is likely to be visible.

**Therefore:** serverless mode costs us a p99 latency tail at migration boundaries, which is
measurable and bounded, and buys scale-to-zero, preview deployments, one deployable, and no process
to babysit. `drainOnVersionUpgrade` — a runner-mode flag — is not part of this architecture at all,
and `01` §11's warning that flipping it "would force the whole recovery table to be re-derived" is
withdrawn: the recovery table needs no changes.

### 3.5 Local development, and the one way to get this catastrophically wrong

`bun run dev` starts Vite on 5173 and that is the whole system. The catch-all route serves the
gateway locally; the client's `${origin}/api/rivet` endpoint resolves to it.

With `RIVET_ENDPOINT` unset, RivetKit falls back to its filesystem state driver. Locally that is
correct and convenient — actor state, SQLite and KV survive a dev-server restart under a gitignored
directory. **On Vercel it is fatal**, because the filesystem is read-only, and the failure surfaces
as a confusing `ENOENT … mkdir` rather than as "you forgot an environment variable".

Guard it. In `src/lib/server/env.ts`, parsed exactly once and nowhere else:

```ts
import { building, dev } from "$app/environment";
import { env } from "$env/dynamic/private";

if (!building && !dev && !env.RIVET_ENDPOINT) {
  throw new Error(
    "RIVET_ENDPOINT is unset in a non-dev build. RivetKit would fall back to the " +
    "filesystem driver, which cannot work on a read-only serverless filesystem.",
  );
}
```

If Vercel Deployment Protection is enabled on the project, Rivet Cloud's inbound `/api/rivet/start`
requests get a 401. Fix it in Rivet Cloud (Settings → Providers → Edit → Add Header) with
`x-vercel-protection-bypass` set to the project's bypass secret. This is a deployment-day trap that
costs an hour if it is not written down, so it is written down.

---

## 4. Persistence: `c.state`, `c.kv`, `c.db`. Nothing else.

### 4.1 Neon is retired

`00` §7.2 chose Neon Postgres for Better Auth's five tables plus `jwks`, and `00` §6 made
`DATABASE_URL` a required variable. **That decision is void.** Deleted along with it:

- `DATABASE_URL` as an environment variable;
- `postgres@3.4.9` (the postgres-js driver);
- `@better-auth/drizzle-adapter@1.6.25` in its Postgres role;
- the `.npmrc` nested-linker requirement and the `pnpm why drizzle-orm` CI assertion, both of which
  existed only to let `rivetkit`'s `drizzle-orm ^0.44.x` coexist with the Better Auth adapter's
  `0.45.2`;
- `apps/web/src/lib/server/db/**` as a Postgres schema location;
- the `drizzle-kit migrate` step against Postgres in `00` §8.

`drizzle-orm` and `drizzle-kit` **survive** — solely for actor-local SQLite via `rivetkit/db/drizzle`,
with migrations generated per actor folder and committed, exactly as `01` §11 already specifies. The
`better-sqlite3` dependency currently in the scaffold is deleted: nothing in this architecture opens
a SQLite file directly.

### 4.2 The three tiers

| Tier | Durable | How it loads | Shape | Correct use | Wrong use |
|---|---|---|---|---|---|
| `c.state` | yes — auto at action end, `stateSaveInterval`, or `saveState({ immediate: true })` | **whole, on every wake** | CBOR-serializable object | small, bounded, hot, read-in-full-every-time | anything that grows. Keep well under 128 KB. |
| `c.kv` | yes | per-key `get`/`put`, `list` by prefix | string or `Uint8Array` values under string or binary keys | variable-size or unbounded blobs addressed by a **known key**, written once and read rarely, where SQL buys nothing | as a substitute for `c.state` on hot fields (you pay a round trip per key), or without a retention rule |
| `c.db` | yes | queried, never loaded whole | SQLite + Drizzle, ≤10 GiB per actor | anything you will **filter, join, paginate, or aggregate** | tiny singleton rows that `c.state` holds for free |
| `c.vars` | **no** | rebuilt per wake in `createVars` | anything, including non-serializable | RNG closure rebuilt from `c.state.seed`, cached JWKS, `LanguageModel` instances | any datum that must survive a wake. Under serverless, `c.vars` dies on **every migration**, which is far more often than it died under runner mode. |

The `c.vars` row deserves emphasis. `01` §3.1 already said `c.vars` is rebuilt per wake, and `01`
§9.1 already explained that the seed lives in `c.state` while the RNG *closure* lives in `c.vars` —
which is precisely why a restart reproduces the same shuffle. Under serverless that discipline is
exercised constantly instead of occasionally. Anything that would be a latent bug in runner mode is
a routine bug here.

### 4.3 Revised storage policy, per actor

| Actor | `c.state` | `c.kv` | `c.db` (SQLite) |
|---|---|---|---|
| **`euchreTable`** `["table", gameId]` | `GameState` whole: `schema`, `v`, `internalToken`, `seed`, `turnId`, `turnDeadlineAt`, the three schedule ids, `pending`, `recentMoveIds` (≤32), `appliedSeq`, `status`, `cfg`, `score`, `seats`, and the live `HandState` (deck order, four hands, kitty, bids ≤8, current trick, `trickLog` ≤5). A few KB — the caps in `01` §3.2 are what keep it there. | `steps:{handNo}` — the completed `Step[]` choreography for one finished hand, as one write-once blob. Variable-size, read only by the replay viewer. `talk:{handNo}` — the full banter/rationale transcript for a hand, unbounded text that would bloat every journal row scan. **Retention:** both are deleted at self-reap; both are re-derivable from the journal, so loss is degradation, not corruption. | The append-only per-hand journal — `(handNo, seed, dealerSeat, deckOrder, moves[], trump, makerSeat, aloneSeat, tricksWon, result, delta)` — plus the one `match_meta` row written in `onCreate` holding the three snapshotted `PersonaConfig`s, which `dispatchAi` reads on the cold path to rebuild `createWithInput`. |
| **`aiSeat`** `["table", gameId, "seat", n]` | `{ gameId, seat, teamId, persona, memory, budget, meter, lastTurnId, episodeBuffer }` — every collection capped (`trumpSeen` ≤7, `leadHistory` ≤5, `bidHistory` ≤8, `humanRead` ≤8, `episodeBuffer` ≤12), so card counting survives a mid-hand migration. | `log:{handNo}` — the per-hand decision log (prompt hash, source, latency, token counts, rejected candidates) once the capped in-state ring rolls it off. Unbounded, append-once, read only by the M4 metering harness. **Retention:** dropped on `gameEnd` after the meter flush. | **none.** `01` §3.3's "there is no `c.db`" stands. `c.kv` is what removes the temptation to add one. |
| **`playerProfile`** `["user", userId]` | `{ schema: 1, activeGameId, dailyTokens, lifetime }` — deliberately tiny. | *Optional and default-off:* `replay:{matchId}` as a memoised, evictable cache of server-derived replay frames. It is an optimisation, it is derived data, and by invariant 22 it is not enabled before `getReplay` latency is measured. | `matches`, `hand_journal`, `personas`, `persona_versions`, `episodes`, `phrasebook`. Queried and paginated, never loaded whole. Unchanged from `01` §3.4. |
| **`authStore`** `["auth"]` *(new — §5)* | `{ schema: 1 }` and nothing else. | nothing. | Better Auth's five tables (`user`, `session`, `account`, `verification`, and the plugin tables) plus `jwks`. Migrations generated by `drizzle-kit` and committed. |

### 4.4 The new cap rule

`00` §3 invariant 20 requires an explicit cap on every map and array in `c.state`, because `c.state`
is loaded whole on every wake. `c.kv` has the opposite failure mode: it never makes you feel the
cost, so it grows forever. Extend the invariant:

> **Every `c.kv` key namespace declares, in the same file that writes it, a key format and a
> retention rule.** A namespace with no documented deletion trigger is a review reject, exactly as
> an uncapped `c.state` array is.

Every namespace in §4.3 satisfies this. `steps:` and `talk:` die at self-reap; `log:` dies at
`gameEnd`; `replay:` is evictable derived data and is off by default.

---

## 5. Auth: Better Auth stays; its storage moves into an actor

### 5.1 What stands

Better Auth 1.6.25 with the `magicLink` and `jwt` plugins; Resend 6.18.0 for delivery, branching on
`{ data, error }` because it never throws; the magic link logged to the console when
`RESEND_API_KEY` is absent so local sign-in never blocks; the `/auth/continue` interstitial that
survives SafeLinks/Proofpoint link-prefetching; short-lived JWTs with `audience: 'euchre-actors'`
minted at `POST /api/rivet-token`; `jose` verification inside every browser-reachable actor; the
origin allowlist in `onBeforeConnect`; the `c.key[1] === claims.userId` IDOR gate on `playerProfile`;
and every action re-checking `c.conn.state` rather than trusting the key it was reached through.

`00` §3 invariant 16 is unchanged in substance. Only the storage underneath, and the client-side
plumbing that delivers the token, change.

### 5.2 The three candidates, evaluated honestly

**(a) A Better Auth instance whose storage is a Rivet actor's `c.db` SQLite.**

Two sub-shapes, and the difference between them is the whole decision.

*(a1) A custom adapter in the SvelteKit process.* Write a `createAdapter()` implementation whose
`create`/`findOne`/`findMany`/`update`/`delete`/`count` call generic CRUD actions on an `authStore`
actor. Cost: you are reimplementing Better Auth's where-clause semantics — operators, `AND`/`OR`
nesting, sort, limit, offset, field-name transforms, `null` handling — against a store that will
happily return a plausible wrong answer. A subtly wrong `findOne` on the session table is a silent
authentication bug, which is the worst category of bug this project could ship. Reject.

*(a2) The Better Auth instance runs **inside** the actor.* `rivetkit/db/drizzle` gives a real Drizzle
instance bound to `c.db`, so the **stock** Better Auth Drizzle adapter applies with zero custom
adapter code. The actor's `onRequest` delegates to `auth.handler(request)`. Everything Better Auth
knows how to do, it does, against a real SQLite database, using code we did not write.

Shared cost of both: `authStore` is a **singleton** — the one global hot spot in a system whose
scaling story (`00` §10, `01` §10) was explicitly "there is no singleton; every key is sharded by
`gameId` or `userId`". Session reads on SSR become a cross-actor hop. This is a genuine regression
and is named as such.

**(b) libsql / Turso.** Hosted SQLite over HTTP. Works from a serverless function, is not Postgres,
has a usable free tier, and Better Auth supports it with zero custom code through the Drizzle adapter
over `drizzle-orm/libsql` or through Kysely's `LibsqlDialect`. No singleton, no cross-actor hop, and
the identical operational shape as (a2) from Better Auth's point of view. It fails on one thing only:
the directive says **no external database**, and Turso is one. It is otherwise the lowest-risk
option, which is why it is recorded here as the named escape hatch rather than dismissed.

**(c) Better Auth's memory or file SQLite adapter, with actor-backed persistence behind it.** This is
not a tradeoff; it is a correctness failure. `memoryAdapter` state lives in **one function
instance's** heap. Vercel runs many concurrent instances: a session created on instance A does not
exist on instance B, so a user signs in and is immediately signed out by the next request that lands
elsewhere. Adding write-through to an actor does not fix reads, and adding read-through makes it (a1)
with a stale cache and a write-back race in front of it. A `better-sqlite3` file adapter is worse
still — the filesystem is read-only. Reject outright.

### 5.3 The decision

> **Default: (a2).** Better Auth runs inside an `authStore` actor keyed `["auth"]`, over the stock
> Drizzle adapter bound to that actor's `c.db` SQLite. Zero custom adapter code. Turso (b) is the
> documented escape hatch if the singleton is ever measured to be a bottleneck; the migration is a
> `DATABASE_URL`-shaped change confined to one file, because the Drizzle schema is identical.

The singleton is tolerable for three specific reasons, and it is worth being precise about them
rather than waving:

1. **It is not on the realtime path.** `authStore` is touched on sign-in, magic-link verification,
   session read during SSR, and token mint. It is never touched during a trick, a bid, a deal, or an
   AI decision. The hot path — a browser connecting to `euchreTable` — verifies a JWT with local
   crypto and touches nothing (§5.5).
2. **This is a single-player game.** One human per table. Concurrent sign-ins are the only
   contention, and they are rare, bursty, and serialised through a store designed to serialise.
3. **The shard path exists if it is needed.** `userDirectory` keyed by an email-hash prefix plus a
   `session` actor keyed by a token prefix would remove the singleton at the cost of secondary-index
   coordination. It is recorded in `docs/LEDGER.md` as a v2 path with its price attached, not
   pretended away.

### 5.4 The shape

```
   browser ──POST /api/auth/sign-in/magic-link──► SvelteKit hooks.server.ts
                                                          │
                              forwards /api/auth/* verbatim│ .fetch(request)
                                                          ▼
                                            authStore ["auth"]  onRequest
                                              auth.handler(request)
                                              drizzle(c.db) ── SQLite
                                                          │
                                            Response (incl. Set-Cookie)
                                                          │
   browser ◄──────────────── returned unmodified ─────────┘
```

Three concrete consequences, each of which is easy to get wrong:

- **`svelteKitHandler` and `sveltekitCookies` are not used.** `00` §4 notes that `svelteKitHandler`
  intercepts `/api/auth/*` and that no `[...all]` route exists in Better Auth 1.6.x. Both facts stop
  mattering: Better Auth is not mounted in the SvelteKit process at all. `hooks.server.ts` forwards
  the request to the actor and returns the actor's `Response` **unmodified**, so `Set-Cookie` rides
  the response headers naturally. The `sveltekitCookies(getRequestEvent)` plugin exists to bridge
  cookies onto a SvelteKit request event that does not exist inside an actor; including it would
  throw. Drop it, and drop the "must be LAST in the plugin array" note with it.
- **Session read on SSR is one hop.** `hooks.server.ts` populates `locals.session` by calling the
  actor's `get-session` endpoint with the inbound `cookie` header, once per request, cached on
  `locals` for the duration. Do this only for routes under `(app)` and the auth routes — not for
  static assets, not for the login page.
- **`onBeforeConnect` on `authStore` rejects every browser connection.** No browser ever holds a
  WebSocket to `authStore`; it is reached only through `onRequest` from our own server code. It is
  still browser-*addressable* through the gateway, so per `00` §3 invariant 16 it declares the hooks
  and refuses.

### 5.5 How the session token reaches an actor connection

The hot path deliberately never touches `authStore`.

**Mint (once per connection lifetime, ~15 minutes).** `POST /api/rivet-token` is a SvelteKit route,
so it has the cookie. It asks `authStore` for a JWT via the `jwt` plugin and returns it.

```ts
// src/routes/api/rivet-token/+server.ts
import { error, json } from "@sveltejs/kit";
import { authFetch } from "$lib/server/auth-client";  // thin wrapper over authStore .fetch()
import type { RequestHandler } from "./$types";

export const POST: RequestHandler = async ({ request, locals }) => {
  if (!locals.user) error(401, "unauthenticated");
  const res = await authFetch("/api/auth/token", { headers: request.headers });
  if (!res.ok) error(401, "unauthenticated");
  const { token }: { token: string } = await res.json();
  return json({ token });
};
```

**Carry.** On the client, the token becomes a connection param via `withActorParams()` — the
adapter's replacement for a hand-written `getParams`:

```ts
// src/lib/game/use-table.svelte.ts
import { withActorParams } from "@rivetkit/svelte";
import type { registry } from "$lib/actors/registry";

const table = useActor(
  withActorParams<typeof registry, "euchreTable">(
    () => ({ name: "euchreTable" as const, key: ["table", gameId] }),
    () => ({ token: auth.rivetToken }),          // reactive: re-read on refresh
  ),
);
```

> **The gotcha that will cost a day if it is not written down.** `withActorParams` merges params into
> the actor options, and framework-base hashes those options to decide actor *identity*. A refreshed
> token therefore looks like a **different actor** and opens a **second WebSocket** to the same table.
> The adapter's own `AGENTS.md` documents the fix and the production app that uses it: supply an
> `actorIdentityHash()` that **excludes the volatile auth param** when constructing the shared client,
> so token refreshes do not fragment one actor into duplicate connections. Implement it in
> `src/lib/client/rivet.ts` at the point the shared client is created, not per-call-site.

**Verify.** Server-side, unchanged in substance from `01` §7 — origin allowlist, then JWT, then
authorization:

```ts
// src/lib/actors/euchre-table/index.ts
import { UserError } from "rivetkit";
import { HUMAN_SEAT } from "$lib/euchre";
import { CONFIG } from "$lib/server/env";
import { verifyPlayer } from "../auth/verify.js";
import type { TableConnState } from "./queues.js";

interface ConnParams { readonly token: string }

onBeforeConnect: (c, _params: ConnParams): void => {
  // Rivet has no WebSocket CORS, and the socket terminates at Rivet Cloud — Vercel's
  // own CORS configuration is not in the path at all. This allowlist is the only check.
  const origin = c.request?.headers.get("origin") ?? "";
  if (!CONFIG.allowedOrigins.includes(origin)) {
    throw new UserError("Origin not allowed", { code: "origin_not_allowed" });
  }
},

createConnState: async (c, params: ConnParams): Promise<TableConnState> => {
  let claims;
  try {
    claims = await verifyPlayer(c.vars.jwks, params.token, CONFIG.appUrl);
  } catch {
    throw new UserError("Invalid or expired token", { code: "invalid_token" });
  }
  if (claims.userId !== c.state.ownerUserId) {
    throw new UserError("Not your game", { code: "forbidden" });
  }
  return { userId: claims.userId, seat: HUMAN_SEAT, role: "player", since: Date.now() };
},
```

`c.vars.jwks` is built in `createVars`. Because the registry now lives in-process, the JWKS is
fetched from `authStore` directly and verified locally, rather than looping out over HTTP to a
second deployment:

```ts
// src/lib/actors/auth/verify.ts
import { createLocalJWKSet, jwtVerify, type JSONWebKeySet } from "jose";

export async function loadJwks(c: { client: <R>() => any }): Promise<JSONWebKeySet> {
  const res = await c.client<typeof registry>().authStore.getOrCreate(["auth"])
    .fetch("/api/auth/jwks");
  return (await res.json()) as JSONWebKeySet;
}

// createVars:  { jwks: createLocalJWKSet(await loadJwks(c)), … }
```

`createRemoteJWKSet(new URL('/api/auth/jwks', appUrl))` remains a correct fallback and is what `01`
§7 specified; it just costs a pointless round trip out through the CDN and back into the same
deployment. Either way the set is cached in `c.vars` and rebuilt per wake, so verification is one
in-memory JWK lookup per connect. **`aiSeat` still holds no credentials of any kind** — it is never
browser-reachable and has no connection hooks.

**Refresh.** A 15-minute JWT outlives most hands and dies during long ones. The client handles it
without a page reload, and — per the adapter's `AGENTS.md` — with `reconnect()`, **not**
`dispose()` + `mount()`:

```ts
table.onEvent("$error", (e) => {
  if (e.code === "invalid_token") void auth.refreshRivetToken().then(() => table.reconnect());
});
```

`reconnect()` drives framework-base's `enabled` toggle (disable → dispose → `idle` → re-enable →
create), re-runs the params getter for a fresh token, and rebinds `onEvent()` listeners. A plain
`dispose()` + `mount()` cannot replace a half-open "zombie" socket — one that a NAT or load balancer
has culled but that still reports `connected` — because framework-base only creates from `idle`.
That failure mode is invisible in development and common in production on mobile networks.

### 5.6 The security model, restated honestly

`00` §6 and `01` §1 claimed a real security property: two processes, two credential sets,
non-overlapping blast radii, and `apps/game` holding no database credential so that compromising the
actor process yielded no Postgres access. **One deployable means that property is gone**, and
pretending otherwise would be worse than losing it. What replaces it:

- The `ANTHROPIC_API_KEY` and `RIVET_ENDPOINT` (`sk_`) are server-only environment variables and
  are never referenced outside `src/lib/server/**` and actor files. `RIVET_PUBLIC_ENDPOINT` carries
  a publishable (`pk_`) token and is the only Rivet credential the browser ever sees.
- There is no Postgres credential to leak, because there is no Postgres.
- The boundaries that actually defend game integrity are unchanged and were never process
  boundaries: the `internalToken` constant-time compare on every actor→actor queue (`00` §3
  invariant 6), `project()` as the sole producer of client-visible state (invariant 7), the
  per-connection redaction in `pushSync`, and the `ownerUserId` check in `createConnState`.
- Recorded in `docs/LEDGER.md`: *bought one deployable; paid the two-process blast-radius split.*

---

## 6. The client layer: `@rivetkit/svelte` replaces the hand-rolled store

`00` §5 puts a hand-written `src/lib/client/table.svelte.ts` in the tree and `01` §7 sketches a
hand-written `joinTable()` with a bespoke `getParams`. Both are replaced. The adapter at
`/Users/brittianwarner/goods/euchre/rivetkit-svelte` owns transport; our code owns euchre.

### 6.1 One gotcha before the mapping

The reference implementation on disk calls `setupRivetKit()` / `getRivetContext()`. **Those are gone.**
It vendors an older copy. The local package's `AGENTS.md` and `README.md` are explicit: *"The package
no longer exports package-global default-context helpers."* Use `createRivetContext()` and an
app-local context instance. Copy the reference's *patterns*, not its imports.

Also required by the ground truth: `rivetkit-svelte/package.json` currently declares
`@rivetkit/framework-base@2.3.5` and peers `rivetkit@^2.3.5-layerr.0`. Both must be bumped to
`2.3.9` / `^2.3.9` before anything else in this section will typecheck against the pinned `rivetkit`.

### 6.2 Setup

```ts
// src/lib/client/rivet.ts
import { createClient, createRivetContext, createSharedRivetKit } from "@rivetkit/svelte";
import type { registry } from "$lib/actors/registry";     // TYPE-ONLY. Never a value import.

export const rivetContext = createRivetContext<typeof registry>("EuchreRivet");

const getClient = (() => {
  let client: ReturnType<typeof createClient<typeof registry>> | null = null;
  return () => (client ??= createClient<typeof registry>({
    endpoint: `${window.location.origin}/api/rivet`,   // discovery, not proxy — §2.2
    // actorIdentityHash: excludes the volatile `token` param — §5.5
  }));
})();

export const getRivet = createSharedRivetKit<typeof registry>(getClient);
```

```svelte
<!-- src/routes/+layout.svelte -->
<script lang="ts">
  import { rivetContext, getRivet } from '$lib/client/rivet';
  let { children } = $props();
  rivetContext.set(getRivet());
</script>

{@render children()}
```

One transport, endpoint-scoped, shared by page components and by the `table.svelte.ts` store class.

### 6.3 Mapping the spec's `TableStore` responsibilities

| Responsibility in `00`/`04` | Adapter primitive | Notes |
|---|---|---|
| Create the client; one transport for the app | `createClient` + `createSharedRivetKit(getClient)` | `useActor()` in components and the store class share one socket per actor. |
| Provide it to the tree | `createRivetContext<typeof registry>()` + `ctx.set(getRivet())` in `+layout.svelte` | App-local typed context. `setupRivetKit`/`getRivetContext` no longer exist. |
| Connection lifecycle in a page | `useActor(() => opts)` | Component-init only; `$effect` is the browser-only lifecycle boundary, so it is SSR-safe. |
| Connection lifecycle in a `.svelte.ts` store class | `createReactiveActor(opts)` + `mount()` / `dispose()` | Construction is side-effect-light and does not subscribe or connect until `mount()`, which is what makes it legal in a module. |
| Auth params + refresh | `withActorParams(base, () => ({ token }))` | Plus the identity-hash exclusion (§5.5). |
| Subscribe to `sync`/`thinking`/`chat`/`chatDelta`/`presence` | `handle.onEvent(name, fn)` | Rebinds automatically when the underlying connection changes — including across `reconnect()`. |
| Call `submitMove` / `snapshot` / `lastTrick` | Proxied actor methods on the handle | `await table.submitMove(moveId, clientMoveId)`. Methods are cached per connection instance. |
| Track in-flight submits, timeouts, errors | `actionDefaults: { timeout, throwOnError: false, guardConnection: true, onActionError }` → `isMutating`, `pendingActions`, `lastActionError`, `lastAction`, `resetActionState()` | **Gotcha:** with the default `throwOnError: false`, a rejected `submitMove` **resolves `undefined`** rather than throwing. The optimistic snap-back path in `04` must branch on `lastActionError` / a falsy result, **not** on a `catch` block. Getting this wrong means a rejected move silently sticks on screen. |
| Distinguish "not connected yet" from "connection lost" | Guard errors carry `code: "ACTOR_NOT_YET_CONNECTED"` vs `"ACTOR_DISCONNECTED"` | Retryable startup vs a real drop, without parsing message strings. |
| Connection status for the HUD | `connStatus`, `isConnected`, `hasEverConnected`, `lastError` | `hasEverConnected` is what distinguishes a first-load spinner from a reconnect banner. |
| Hard resync after any (re)connect | `$effect` on `isConnected` → `await table.snapshot()` | Mirrors the reference's `syncState()`, including its bounded exponential retry. `01` §6.1's rule stands: missed events are never replayed, so resync is `steps: []` — snap, do not animate. |
| Recover from an expired token or a zombie socket | `reconnect()` | Not `dispose()` + `mount()`. See §5.5. |
| Warm the actor on intent | `warmUp({ name: "euchreTable", key: ["table", gameId] })` | Single HTTP resolve, no WebSocket; deduplicated per RivetKit lifetime; no-ops during SSR. Correct on hover of "Resume game" in `/games`. Never call it "prefetch" — that word belongs to SvelteKit's `preloadData`. |
| Open the socket ahead of a committed navigation | `preConnect({ … })` → `{ dispose() }` | High-intent tier only. **The caller owns the lifecycle and must `dispose()` or the socket leaks.** Use on the `/play` transition, not on hover. |
| Aggregate health across `euchreTable` + `playerProfile` | `createConnectionHealth(getSources)` | Optional; powers a single connection chip in the HUD. |

### 6.4 What stays ours

Everything that knows what euchre is. `applyOptimistic(view, move)` and its snap-back; the
`director.svelte.ts` that consumes `sync{view, steps}` and reconciles; the `Step[]` animation queue
and `cardMotion.svelte.ts`; the `{brisk 0.7, table 1.0, slow 1.3}` tempo multiplier; `HandA11y`;
the whole Threlte layer. The store class in `src/lib/game/table.svelte.ts` becomes a `$state` class
holding `view`, `steps` and derived UI state — exactly the shape of the reference's
`GameStore` — with the adapter handle injected rather than a socket hand-rolled inside it. Per the
Svelte 5 rules this project is bound by: classes with `$state`, not stores; `$derived` over
`$effect`; keyed each blocks; snippets, not slots.

### 6.5 SSR

Do **not** set `export const ssr = false` globally. The reference implementation does, and it is
wrong for us: `04` makes an accessible server-rendered table the *first* UI, and the `(app)` route
group's redirect guard is a server load. Keep SSR on. It is safe because `useActor()` subscribes only
inside `$effect`, `createReactiveActor()` does not connect until `mount()`, `warmUp()`/`preConnect()`
no-op during SSR via `esm-env`'s `BROWSER`, and `@threlte/core`'s `<Canvas>` mounts its children
client-only so the 3D layer needs no SSR guards at all. This was left implicit by the spec; it is
decided here.

---

## 7. CORRECTIONS — two claims made earlier in this project were wrong

This section exists so that nobody re-derives these errors from the same sources.

### 7.1 "SvelteKit is unsupported on Vercel for RivetKit" — **WRONG**

**Where it came from.** A stale local documentation snapshot. `reference/connect/vercel.md` in the
on-disk RivetKit skill has sections for Next.js and Hono and then a section headed "Other" that says
*"Vercel currently supports Next.js and Hono frameworks for RivetKit deployments… For other
frameworks, consider deploying to Railway, Kubernetes, or another platform."* That sentence was read
as a platform capability statement. It is a statement about which worked examples that documentation
page shipped.

**The correct mechanism.** RivetKit's serverless integration is a plain WinterTC handler:
`registry.handler(request: Request) => Promise<Response>`. It has no framework dependency of any
kind. Any framework that can mount a catch-all route and hand it a standard `Request` can host it.
SvelteKit mounts it at `src/routes/api/rivet/[...all]/+server.ts` exporting every verb plus
`fallback` (§3.1). Separately and independently, `sveltekit` is a first-class Vercel framework
preset, which is all `vercel.json`'s `{"framework": "sveltekit"}` declares. The two facts have
nothing to do with each other, and neither one constrains the other.

**The proof.** `/Users/brittianwarner/goods/rivet-game-svelte` is a real, deployed SvelteKit +
RivetKit + Threlte application on Vercel using exactly this arrangement:
`src/routes/api/rivet/[...all]/+server.ts` → `registry.handler(request)`,
`svelte.config.js` → `adapter-vercel`, `vercel.json` → `{"framework": "sveltekit"}`.

### 7.2 "SvelteKit cannot handle the WebSocket upgrade" — **WRONG, and irrelevant**

**Where it came from.** Unfounded inference, built on a true premise. `01` §1 correctly observes that
`adapter-node` does not proxy a WebSocket `Upgrade` cleanly, and correctly used that to justify
keeping SvelteKit off the realtime path. The error was generalising that into *"therefore the
registry cannot live inside SvelteKit."* The conclusion does not follow, because hosting the registry
never requires terminating a WebSocket.

**The correct mechanism.** In serverless mode the browser's WebSocket terminates at **Rivet Cloud**.
Our deployment participates over ordinary HTTPS only, in two ways: the browser GETs
`/api/rivet/metadata` (plain JSON: `clientEndpoint`, `clientNamespace`, `clientToken`), and Rivet
Cloud opens a long-lived `GET /api/rivet/start` request per actor. Both are `Request → Response`.
Actor output flows back to Rivet Cloud on that request's response stream and Rivet Cloud fans it out
over the browser's socket.

There is no `Upgrade` header anywhere in our code. There is no `ws` dependency, no
`@hono/node-ws`, no `onWebSocket` handler, no upgrade proxy. `01` §1's actual requirement —
*"SvelteKit is never on the realtime path and therefore never has to proxy a WebSocket `Upgrade`"* —
**is satisfied exactly as written**, by a topology it did not anticipate. The premise was right and
survives; only the inference drawn from it was wrong.

### 7.3 "`c.kv` is deprecated in 2.3.9; any use is a review reject" — **WRONG**

`01` §3.1 asserts this "verbatim from the type definition". It is not so. `c.kv` is a documented,
supported low-level key-value store on every actor context, with `get`/`put`/`list`, string and
binary values, and prefix listing; it is what `c.state` is built on. Same failure mode as §7.1: a
confident claim sourced from a stale snapshot rather than from the installed package. `c.kv` is now
a first-class storage tier (§4).

### 7.4 The rule that prevents the next one

> Before writing "X is unsupported" or "X cannot do Y" into a binding document, check the working
> implementation on disk. A deployed application outranks a documentation snapshot, and a
> documentation snapshot outranks recollection. If no implementation is available, say "unverified"
> and open an item — do not promote an inference to a constraint. Every one of the three errors above
> was a doc snapshot or an inference promoted to a constraint, and each of them changed the
> architecture.

---

## 8. Open items

These are named rather than guessed, per `00` §3 invariant 22.

**8.1 Version and roll semantics under serverless.** `envoy.version` / `RIVET_RUNNER_VERSION` are
runner-mode machinery and are dropped (§1.3). What replaces them on Rivet Cloud + Vercel — how a new
Vercel deployment URL becomes the target of the runner config, and whether in-flight actors on the
previous deployment finish or migrate — must be confirmed against Rivet Cloud's runner-config
behaviour before M10's "deploy mid-hand does not disrupt play" exit criterion can be asserted. The
`rivet-dev/preview-namespace-action` GitHub Action is the intended mechanism for per-PR namespaces
and is the place to look first.

**8.2 `requestLifespan`.** Must be ≤ the Vercel function max duration for our plan. Set it once the
plan is known, and record the measured migration-boundary latency alongside the tempo floors in the
M4 metering pass.

**8.3 Does `c.kv` earn its place?** The `steps:`/`talk:`/`log:` namespaces in §4.3 are a design, not
a measurement. If the journal rows turn out small enough that SQLite handles them without bloating
scans, delete the namespaces and the tier goes unused for `euchreTable` and `aiSeat`. Decide after
M4, not before.

**8.4 `authStore` sharding.** Single actor in v1. Revisit only if sign-in latency is measured to be a
problem. The shard design and Turso escape hatch are both in `docs/LEDGER.md` with their prices.

**8.5 Where `05-IMPLEMENTATION-PLAN.md` needs re-cutting.** M0 (which currently deletes the scaffold)
and every milestone that assumes two processes, `pnpm --filter`, or a Postgres migration step must be
re-derived against this document. That is a separate document's job, not this one's.

---

## 9. One-paragraph summary for anyone who reads only this

One SvelteKit app deploys to Vercel with `adapter-vercel` and `vercel.json` `{"framework":
"sveltekit"}`. The RivetKit registry mounts inside it at `src/routes/api/rivet/[...all]/+server.ts`
via `registry.handler(request)`, in serverless mode against Rivet Cloud. The browser opens one
WebSocket **to Rivet Cloud**; Rivet Cloud calls **us** over ordinary HTTPS; we never terminate a
WebSocket and never proxy an `Upgrade`. Rivet migrates actors across function invocations while
preserving `c.state`, `c.kv`, `c.db`, durable queue messages, armed schedules and hibernating
sockets, so a drain mid-trick costs latency and not correctness — which is why runner mode is no
longer needed. Persistence is `c.state` / `c.kv` / `c.db` only; Neon and Postgres are retired. Better
Auth keeps magic link + Resend but runs **inside** an `authStore` actor over the stock Drizzle adapter
bound to that actor's SQLite, and hands the browser a 15-minute JWT that reaches actors through
`withActorParams()` and is verified locally in `createConnState`. The client uses `@rivetkit/svelte`
instead of a hand-rolled store. Everything about euchre — the reducer, the redaction boundary, the
durability ordering, the AI ladder, the three-actor decomposition, the accessible-first UI — is
unchanged.
