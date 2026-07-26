> **⚠ PARTIALLY SUPERSEDED — read [`06-REVISED-ARCHITECTURE.md`](./06-REVISED-ARCHITECTURE.md) first.**
> That ADR outranks this document on every point it names, and §1 of it is the itemised diff.
> Superseded here: **§4** (pnpm workspace, `adapter-node`, Postgres/Neon packages, `.npmrc` nested
> linker, install commands); **§5** (the `apps/*` + `packages/*` file tree, and the note that M0
> deletes the existing scaffold — it is kept); **§6** (`DATABASE_URL`, `PUBLIC_RIVET_ENDPOINT`, the
> two-process env split); **§7.2** (Neon — retired outright); **§7.4** (long-lived Bun runner — it is
> serverless on Vercel + Rivet Cloud); **§8** (two processes on 5173 + 6420 — there is one).
> Still binding: §1, §2, §3 invariants 1–23 (16, 20 and 22 amended in mechanism only), §7.1, §7.3,
> §7.5, §9.

# 00 — Overview & Decision Record

**Status:** binding. **Date:** 2026-07-26. **Audience:** an expert engineer who has never played euchre and has never used RivetKit or Threlte.

Everything in this document is a decision, not a proposal. Where a downstream document disagrees with this one, this one wins.

| Doc | Owns |
|---|---|
| `00-OVERVIEW.md` | This file: product, decision record, invariants, dependencies, file tree, env, local run, glossary. |
| `01-ARCHITECTURE.md` | Actor topology, queue/event/schedule wiring, auth boundaries, crash-recovery reconciler, deployment. |
| `02-GAME-RULES-ENGINE.md` | `packages/euchre-core`: the reducer, legality, scoring table, projection/redaction, invariants V1–V12. |
| `03-AI-AGENTS.md` | `aiSeat`: the decision ladder, prompt layering, personas, banter, token budget, metering. |
| `04-FRONTEND-UX.md` | SvelteKit + Threlte: scene graph, atlas, picking, layout proof, tempo, accessibility, onboarding. |
| `05-IMPLEMENTATION-PLAN.md` | Milestones M0–M10, exit criteria, CI gates, the ledger of deliberate deletions. |

---

## 1. What we are building

A single-player euchre table. One human sits South; three AI opponents sit West, North (partner) and East. The human signs in with an emailed magic link, plays a match to 10 points against LLM-driven opponents that make their own decisions under the real rules, can review every past match as a replayable hand-by-hand log, and can rewrite the prompts that define each opponent's personality and the table's overall dynamics.

**Euchre in five sentences, for the reader who has never played it.** Twenty-four cards (9, 10, J, Q, K, A in each of four suits) are dealt five each to four players in two fixed partnerships sitting opposite each other; four cards are left over, and the top one is turned face up. In two rounds of bidding the players decide which suit is *trump* — the suit that beats all others for this hand — and whichever player fixes trump is the *maker*, whose side has undertaken to win at least three of the five *tricks*. A trick is one card from each player, going clockwise; you must play a card of the suit that was led if you hold one, and the highest trump wins, or if nobody trumped, the highest card of the led suit. The one rule that surprises everyone: the jack of the trump suit (*right bower*) is the highest card, and the jack of the *other suit of the same colour* (*left bower*) is the second-highest and **counts as a trump card, not as a member of its printed suit** — with hearts trump, the jack of diamonds *is a heart*, and a player holding only that diamond is void in diamonds. Makers who take 3 or 4 tricks score 1, all 5 score 2, and makers who fail score nothing while the *defenders* score 2 — that failure is a *euchre*, and it is the whole point of the game's name.

**What makes this build unusual, and what each choice buys.**

*The rules live in one zero-dependency package, and nothing else is allowed to know them.* `packages/euchre-core` is a pure reducer, `apply(state, seat, move) → { state, steps }`. The server imports it to decide truth; the AI to know which moves exist; the browser to grey out illegal cards and play optimistically before the server answers; the replay viewer to re-derive frames from a seed; the test suite to play ten thousand games headlessly. It is the only arrangement in which legal-move highlighting, optimistic play, server validation, AI candidate filtering, coach hints and replay share one implementation — and therefore the only one in which they cannot disagree.

*Each AI opponent is its own durable actor, not a function call.* `aiSeat` is a RivetKit actor keyed `["table", gameId, "seat", "1"|"2"|"3"]`, owning one persona, its own card-counting memory, its own model handles, its own token budget and circuit breaker, and its own failure domain. A wedged LLM at seat 3 cannot stall seats 1 and 2 or the table. It is a *workflow*, so a crash after the model call commits replays the recorded result instead of paying for it twice.

*Hidden information is enforced by the type system.* One function, `project(state, seat)`, produces the wire view, and that type has no field capable of holding another player's hand, a buried kitty card, or the dealer's discard — leaking is a compile error, not a review miss. The animation channel gets the same treatment: `Step[]` is projected too, and the `dealerDiscarded` step has no `card` field at all.

*The pauses that carry information are server phases, not client sleeps.* The 700–1000 ms spent reading a completed trick is real server-held state; the next card has not been computed. A patched client that skips every local animation still cannot see it early. Presentation timing — flight arcs, staggers, celebration — is entirely client-side and user-adjustable.

*The accessible UI is the first UI.* The visually-hidden focusable button list that screen-reader support requires anyway ships **un-hidden** at M2 as a complete, playable table: bidding, dealer discard, loners, scoring to 10, three AI opponents, auth. The 3D layer arrives at M6 on top of a game that already works — which is what makes Threlte genuinely severable under schedule pressure.

---

## 2. The design competition

Four independent designs were produced against the same brief and scored against a weighted rubric by design review. The winner was then corrected and merged with material from the other three; the merged result is what this spec set describes.

### 2.1 The entrants

| ID | Name | One-line thesis |
|---|---|---|
| **A** | Table Monolith | One `euchreTable` actor; AI is an in-process module called from the run loop. |
| **B** | Actor Per Seat | `euchreTable` + one `aiSeat` actor per opponent + one `playerProfile` per user. |
| **C** | Workflow Table | The table itself is a `workflow()`; every hand is a replayable step history. |
| **D** | Thin Server | The browser holds the hand and drives animation; the server validates after the fact. |

### 2.2 Scoreboard

Scores are 0–5 design-review judgments against the rubric, not measurements. Weighted total is out of 100.

| Criterion | Weight | A | B | C | D |
|---|---:|---:|---:|---:|---:|
| Brief fidelity — each AI agent is its own Rivet actor | 15 | 0 | **5** | 4 | 2 |
| Hidden-information safety (no hand can reach a client) | 20 | 4 | **5** | 4 | 0 |
| Rules correctness & headless testability | 15 | 4 | **5** | 4 | 3 |
| Crash / restart / deploy durability | 15 | 2 | **4** | 5 | 1 |
| AI latency & cost bounded per decision | 15 | 3 | **4** | 2 | 3 |
| UI severability (playable before 3D exists) | 10 | 3 | **5** | 3 | 0 |
| Implementation risk for one engineer | 10 | 4 | **4** | 2 | 4 |
| **Weighted total** | **100** | **58** | **89** | **69** | **31** |

**B wins** on three decisive margins: it is the only entry that satisfies the brief's "each AI agent is its own Rivet actor" without contortion, the only one in which a leak is a type error, and the only one that produces a fully playable game before any 3D code exists.

**Why the others lost.** **A** puts LLM calls inside the table's mutation loop, so one slow model blocks all four seats and a per-seat token budget has nowhere to live; it also violates the brief outright. **C** is the most durable design on paper, but making the *table* a workflow turns step names into a public interface for the entire rulebook — every rule change risks replay divergence — and workflow bodies may only touch `c.state`/`c.db`/`client()` inside `ctx.step(...)`, which makes a simple run loop into ceremony. **D** dies on one line: if the browser holds the hand, the hand is on the client, and no amount of validation puts it back.

### 2.3 Corrections applied to the winner during synthesis

Four defects in the winning entry, each now a binding rule:

1. **Stacked retry budgets.** B set `maxRetries` on `generateObject`, *and* on the enclosing workflow step, *and* looped in the decision ladder. `abortSignal` is a wall-clock budget across the whole call including SDK retries, so the inner retries are inert and the outer ones multiply the tail. Corrected to one budget owned by the ladder: `maxRetries: 0` everywhere, one escalation attempt on the bid path only, a 4000 ms hard cap from dispatch.
2. **A 3× inflated cost model.** B claimed ~1500 LLM calls per 10-point game by multiplying an already-all-seats figure by three again. The real ceiling is ~8 decisions per seat per hand (2 bids + 1 discard + 5 plays) = 24 per hand ≈ **700–840 per game**, before the forced-move short-circuit removes single-legal-move decisions.
3. **A human turn timer.** B gave the human 30 seconds per turn. There is one human and nobody is waiting on them; a turn timer means the game plays itself while the user reads the rules panel. Deleted, and written down here so it is not re-added. The replacement is a 90 s partner nudge and a 240 s auto-play so an abandoned game still terminates.
4. **A server-authored beat script.** Presentation belongs to the client (with a `{brisk 0.7, table 1.0, slow 1.3}` user multiplier); only the *information-bearing* pauses — `trick_resolve`, `hand_score`, the cut window — are server-held phases.

### 2.4 Imported from the losing entries

- **From C:** `aiSeat` is a workflow (the table is not), which is exactly where replay-safe step history pays for itself. C's protected-step-name discipline became the `protected-workflow-step-names` lint rule.
- **From A:** the single serialized run loop with a total error branch on every message, and the schedule → action → queue bridge as the only path from a timer into a mutation.
- **From D:** optimistic client play via `applyOptimistic(view, move)` with server snap-back — D's one good idea, made safe by keeping the authoritative hand on the server.

---

## 3. Binding invariants

These are enforced by lint rules, type shapes, or required CI gates. None is a style preference.

1. **One rulebook, one writer.** Every euchre rule lives in `packages/euchre-core`. No euchre rule may be written in any other file. *Enforced by:* code review plus the fact that no other package may import three.js-free rule helpers it would have to duplicate.
2. **Effective suit, always.** No code outside `packages/euchre-core/src/cards.ts` may read `suitOf()` for a suit comparison. Voidness, follow-suit legality, led-suit determination and the trick comparator all go through `effectiveSuit(card, trump)`. *Enforced by:* the `no-suit-compare` ESLint rule.
3. **Every mutation is a durable queue message processed by one serialized run loop.** Actions are read-only with exactly one exception: `submitMove`, which mutates nothing itself and only calls `c.queue.enqueueAndWait`. Schedules fire actions that do nothing but `c.queue.send(...)`.
4. **Persist before fan-out, persist before ack.** `await c.saveState({ immediate: true })` sits between the mutation and *both* the broadcast and `message.complete()`, on every queue handler, with no exceptions. *Enforced by:* the `save-before-complete` ESLint rule.
5. **The acting seat is never taken from the payload.** Human messages use `c.conn.state.seat`; internal messages use `c.state.hand.turnSeat`. Any `seat` field in an argument or queue body is hostile input, used only to detect a mismatch — a disagreement is dropped and logged as an attack, not as a bug.
6. **`canPublish` cannot distinguish internal from external publishes.** A browser using the stateless HTTP handle has no `c.conn`, so `c.conn === undefined` is *true* for a forged publish and an internal-only guard fails open. Player queues therefore gate on `c.conn?.state.role === 'player' && c.conn.state.seat === 0`; internal queues use `c.conn === undefined` as defence in depth only; the **actual** boundary on every actor→actor queue is an unguessable `internalToken` minted in `createState` and constant-time-compared in the run loop before any state read. Every `canPublish` handler returns an explicit boolean and ends with `return false`.
7. **Redaction is structural.** `project(state, seat)` is the only producer of `PublicGameView`; `pushSync` is the only sender of card identity; `c.broadcast` may never be called with anything derived from `hands`, `kitty` or `dealerDiscard`. *Enforced by:* the `no-broadcast-state` ESLint rule and a 10,000-game redaction fuzz gate in CI.
8. **The animation channel is a redaction surface.** `Step[]` passes through `projectSteps(steps, seat)` before reaching any client. The deal choreography is the most natural place in the whole system to leak four hands.
9. **V12 holds in replay.** The three buried kitty cards and the dealer's discard are never revealed to any client, ever, including post-game. The teaching replay shows all four *hands* — that is how anyone learns euchre — but masks the buried three and the discard in every frame; the raw seed never reaches a client, because the server re-derives the frames.
10. **Every queue handler has a total error branch.** A `RuleError` completes the message with a typed rejection carrying `metadata.legal`; it is never left unacked. Only a non-`UserError` may throw and leave the message for redelivery.
11. **Idempotency on every message type.** Every queue message carries `turnId` and `seq`; human submissions add a client `moveId`. `appliedSeq` lives in the same `c.state` as the mutation. `turnId` (server nonce) and `recentMoveIds` (32-entry ring buffer, double-click dedupe) solve two different problems and are never conflated.
12. **No cross-actor call may block the mutation loop.** `wait: true` between actors is forbidden outright; every cross-actor send from the run loop carries an `AbortSignal`.
13. **The engine guarantees competence; the LLM supplies character.** The AI's output schema is `z.enum(legalMoveIds)` rebuilt per decision, *and* the table independently re-derives `legalMoves()` and re-validates before applying. `difficulty` narrows the candidate set (expert = full legal set, casual = engine top-3, rookie = noise-injected ranking) and never touches legality, latency or token count.
14. **One retry budget, owned by the decision ladder.** `generateObject` runs with `maxRetries: 0` and `abortSignal: AbortSignal.timeout(2200)` inside `ctx.step({ timeout: 2600, maxRetries: 0 })`. Ladder: forced (no API call) → constrained LLM (2200 ms) → `thinking{extended:true}` at +400 ms → deterministic heuristic at a 4000 ms hard cap from dispatch. A rejected AI move falls back to heuristic top-1 and is **never** retried as the same message.
15. **The pacing floor is persisted, not a `setTimeout`.** An early decision parks in `c.state.pending` with a `revealAt`, released by `c.schedule.after(…, 'releaseAiMove', turnId)`. Floors: one legal move 250–450 ms; a genuine choice 900–1400 ms; a bid 900–1800 ms.
16. **Every browser-reachable actor enforces auth.** All three declare `onBeforeConnect` (origin allowlist — Rivet has no WebSocket CORS) and `createConnState` (`jose` + `createRemoteJWKSet` against `/api/auth/jwks`). Per-user actors assert `c.key[1] === claims.userId` in `createConnState`, and every action re-checks `c.conn.state` rather than trusting the key. Auth arrives via `getParams`, never a static `params` — WebSockets carry no custom headers.
17. **Personas are snapshotted at game creation and never re-read.** The AI actors hold zero database credentials; a finished game replays with the prompt that produced it; a Settings edit cannot mutate an opponent between trick 3 and trick 4. The browser never creates a game — `/play/+page.server.ts` does.
18. **Banter is a separate generation on public state only.** A distinct `streamText` seeded with `publicStateOnly(view)`, fired concurrently with the decision, salience-gated, table-arbitrated, filtered post-generation, and replaced by a phrasebook line on rejection. Folding the quip into the decision call is forbidden — that prompt contains the AI's hand.
19. **The model is injectable.** `LanguageModel` instances arrive via registry config → actor create input, so `MockLanguageModelV4` swaps in without touching an env var. Constructing a provider from `process.env` inside `createVars` is a defect.
20. **Consumer-or-delete, and bound everything.** Every declared queue has a consumer in the same actor's loop or workflow, or it is deleted — an unconsumed queue accumulates against the 1000-message cap for the life of the match. Every map and array in `c.state` has an explicit cap; `c.state` is loaded whole on every wake.
21. **Workflow step names are a public interface.** `aiSeat`'s step names (`seen`, `rank`, `llm`, `validate`, `reply`, `meter`) never change; renames go through `ctx.removed(name, originalType)`. *Enforced by:* the `protected-workflow-step-names` ESLint rule.
22. **No number is frozen before it is measured.** Tempo floors, the 4000 ms watchdog and the per-game token budget are frozen only after the M4 metering harness reports p50/p95 and cache-read/cache-write token counts over ≥200 real decisions.
23. **The lift is the undo.** Two-stage tap, with drag-to-play an equal citizen at a 25%-of-card-height snap threshold. There is no post-commit undo window — it would desynchronise the AI actors and teach hesitancy. Rejections spring back over 180 ms with a shake and an error tone.

---

## 4. Dependencies

All versions below were verified against the npm registry on 2026-07-26. Where a version is *not* the latest, the reason is stated — those are deliberate pins, not staleness.

| Package | Version | Where | Why this exact version |
|---|---|---|---|
| `pnpm` | 10.19.0 | toolchain | Workspace manager. **Default nested `node_modules` is required**: `rivetkit@2.3.9` depends on `drizzle-orm@^0.44.2` while `apps/web` needs `0.45.2` (the Better Auth adapter's peer). A flat/hoisted linker cannot satisfy both. |
| `bun` | 1.3.14 | `apps/game` runtime | `bun --watch src/index.ts` → `registry.start()`. `apps/web` stays on node 24.11.0 under `adapter-node`. |
| `typescript` | 5.9.3 | root | **Not 7.0.2** (latest): outside `@sveltejs/kit@2.70.1`'s peer range (`^5.3.3 \|\| ^6`), and svelte2tsx/svelte-check compatibility is unproven. |
| `rivetkit` | 2.3.9 | `apps/game`, `apps/web` client | Verified latest. The brief's `2.3.0-rc.5` is stale. Provides `actor()`, `queue()`, `event()`, `workflow()`, `c.schedule`, `c.cron`, `c.db`. |
| `svelte` | 5.56.8 | `apps/web` | Runes only. `@threlte/core@8` is Svelte-5-only (`$effect.pre` internals). |
| `@sveltejs/kit` | 2.70.1 | `apps/web` | `getRequestEvent` (required by `sveltekitCookies`) needs ≥2.20. |
| `@sveltejs/vite-plugin-svelte` | **6.2.4** | `apps/web` | See the spec note below — 7.2.0 is incompatible with the pinned vite. Plain `vitePreprocess()` only. |
| `@sveltejs/adapter-node` | 5.5.7 | `apps/web` | Plain node server. It is never on the realtime path, so it does not need to proxy WebSocket `Upgrade`. |
| `vite` | 7.3.6 | `apps/web` | **Pinned below latest (8.1.5) on purpose**: vite 8 + `@threlte/extras@9.21.0` is the known-hazardous pair (vite-plugin-svelte#1313 — TS-enum transpile leaves `THREE.Box3` undefined at runtime). Revisit only with a Threlte smoke test in CI. |
| `three` | 0.185.1 | `apps/web` | `NeutralToneMapping` (r162+) is required; the `AgXToneMapping` default desaturates felt green and card reds. |
| `@types/three` | 0.185.1 | `apps/web` (dev) | Version-locked to `three`. |
| `@threlte/core` | 8.5.16 | `apps/web` | `<Canvas>` mounts children client-only, so no SSR guards are needed anywhere in the 3D layer. |
| `@threlte/extras` | 9.21.0 | `apps/web` | `interactivity()`, `meshBounds`, `MeshDiscardMaterial`, `ContactShadows`, `transitions()`, `PerfMonitor`. **`useSpring` does not exist in v9** — use `svelte/motion`. |
| `ai` | 7.0.37 | `apps/game` | `generateObject`/`streamText`. v7 names: `maxOutputTokens`, `instructions`, `inputSchema`, `onEnd`/`onStepEnd`, `MockLanguageModelV4`. |
| `@ai-sdk/anthropic` | 4.0.21 | `apps/game` | `claude-haiku-4-5` for plays, `claude-opus-5` for bids. The provider does **not** strip `temperature`/`top_p`/`top_k` on the Opus-5 family (HTTP 400) — hence `modelParams()`. |
| `zod` | 4.4.3 | `apps/game` | `z.enum(legalMoveIds)` rebuilt per decision — the mechanism that makes an illegal AI move structurally impossible. Satisfies `ai@7`'s peer `^3.25.76 \|\| ^4.1.8`. |
| `better-auth` | 1.6.25 | `apps/web` | `magicLink` + `jwt` plugins. `svelteKitHandler` intercepts `/api/auth/*` — **no `[...all]` catch-all route exists in 1.6.x**. |
| `@better-auth/drizzle-adapter` | 1.6.25 | `apps/web` | Separate package now; `better-auth/adapters/drizzle` no longer exists. Peers `drizzle-orm@^0.45.2`. |
| `resend` | 6.18.0 | `apps/web` | Magic-link delivery. Returns `{ data, error }` and **never throws** — always branch on `error`. |
| `jose` | 6.2.4 | `apps/game` | `createRemoteJWKSet` against `/api/auth/jwks`, so `apps/game` holds no DB credentials and no shared secret. |
| `drizzle-orm` | 0.45.2 | `apps/web` | Postgres (Better Auth) + actor-local SQLite via `rivetkit/db/drizzle`. `better-auth@1.6.25` peers this exact minor. |
| `drizzle-kit` | 0.31.10 | root (dev) | Migration generation for Postgres and for each actor's SQLite folder. Migrations are committed. |
| `postgres` | 3.4.9 | `apps/web` | postgres-js driver for Neon. Postgres holds Better Auth's five tables + `jwks` **only**. |
| `vitest` | 4.1.10 | root (dev) | Engine unit/property tests, `rivetkit/test` `setupTest` with fake timers, the redaction fuzz gate. |
| `@vitest/coverage-v8` | 4.1.10 | root (dev) | Coverage gate: 100% of `reduce.ts`/`legal.ts`/`score.ts` branches. |
| `fast-check` | 4.9.0 | root (dev) | Property tests for `effectiveSuit`, follow-suit legality, the trick comparator, and the 10,000-game redaction fuzz. |
| `sharp` | 0.35.3 | `apps/web` (dev) | Build-time card-atlas packing (PNG + `atlas.json`). Never a runtime dependency. |
| `@resvg/resvg-js` | 2.6.2 | `apps/web` (dev) | SVG→raster for card art at build time; 52 runtime `CanvasTexture` uploads are banned. |
| `eslint` | 10.8.0 | root (dev) | Hosts the four custom rules that make the binding invariants mechanical rather than review items. |
| `svelte-check` | 4.7.3 | `apps/web` (dev) | CI typecheck of `.svelte` files; catches the pierced-prop and snippet mistakes `tsc` misses. |

> **Spec note (dependency conflict resolved).** The binding decision list pinned both `vite@7.3.6` and `@sveltejs/vite-plugin-svelte@7.2.0`. These are mutually exclusive: `@sveltejs/vite-plugin-svelte@7.2.0` declares `peerDependencies: { vite: "^8.0.0-beta.7 || ^8.0.0" }`. The vite pin carries an explicit, product-critical hazard rationale (vite 8 + `@threlte/extras@9.21.0` breaks the 3D layer at runtime), so the vite pin is kept and the plugin is dropped to **`@sveltejs/vite-plugin-svelte@6.2.4`**, the newest release whose peer range is `vite: "^6.3.0 || ^7.0.0"`. It sits inside `@sveltejs/kit@2.70.1`'s declared peer range for the plugin (`^3.0.0 || ^4.0.0-next.1 || ^5.0.0 || ^6.0.0-next.0 || ^7.0.0`) and accepts `svelte@5.56.8` (`^5.0.0`). The `vitePreprocess({ script: true })` prohibition is unchanged and independent of this: use plain `vitePreprocess()`. When Threlte publishes a vite-8-verified release, both pins move together in one commit gated on a Threlte smoke test in CI.

> **Spec note (build-time version env var).** The binding decisions name `RIVET_RUNNER_VERSION` as the build-time constant feeding `envoy.version`; RivetKit's own documented env var for that value is `RIVET_ENVOY_VERSION`, and `envoy.version` is a **number**. Resolution: the build script emits `apps/game/src/build-version.ts` exporting a numeric `BUILD_VERSION`, sourced from `RIVET_RUNNER_VERSION`, and the deploy sets `RIVET_ENVOY_VERSION` to the same integer so the programmatic and environment paths cannot disagree. Never `Date.now()` at runtime — every restart would drain and reschedule every actor.

### Install commands

```bash
# toolchain (once, globally)
corepack enable && corepack prepare pnpm@10.19.0 --activate
curl -fsSL https://bun.sh/install | bash        # bun 1.3.14

# workspace root — dev tooling only
pnpm add -w -D typescript@5.9.3 eslint@10.8.0 vitest@4.1.10 \
  @vitest/coverage-v8@4.1.10 fast-check@4.9.0 drizzle-kit@0.31.10 concurrently@9

# packages/euchre-core — ZERO runtime dependencies, by rule
pnpm --filter @euchre/core add -D vitest@4.1.10 fast-check@4.9.0

# packages/protocol
pnpm --filter @euchre/protocol add @euchre/core@workspace:*

# apps/game — the RivetKit registry (bun runtime)
pnpm --filter @euchre/game add rivetkit@2.3.9 ai@7.0.37 \
  @ai-sdk/anthropic@4.0.21 zod@4.4.3 jose@6.2.4 \
  @euchre/core@workspace:* @euchre/protocol@workspace:*
pnpm --filter @euchre/game add -D drizzle-kit@0.31.10   # optional peer of rivetkit, needed for db:generate

# apps/web — SvelteKit + Threlte + Better Auth
pnpm --filter @euchre/web add svelte@5.56.8 @sveltejs/kit@2.70.1 \
  three@0.185.1 @threlte/core@8.5.16 @threlte/extras@9.21.0 \
  better-auth@1.6.25 @better-auth/drizzle-adapter@1.6.25 \
  resend@6.18.0 drizzle-orm@0.45.2 postgres@3.4.9 rivetkit@2.3.9 \
  @euchre/core@workspace:* @euchre/protocol@workspace:*
pnpm --filter @euchre/web add -D @sveltejs/adapter-node@5.5.7 \
  @sveltejs/vite-plugin-svelte@6.2.4 vite@7.3.6 @types/three@0.185.1 \
  svelte-check@4.7.3 sharp@0.35.3 @resvg/resvg-js@2.6.2 drizzle-kit@0.31.10
```

`pnpm@10` auto-installs peer dependencies, so `@better-auth/core` and `@better-auth/utils` (non-optional peers of the Drizzle adapter) resolve without an explicit line. `ws`, `eventsource` and `drizzle-kit` are *optional* peers of `rivetkit`; only `drizzle-kit` is needed, and only for migration generation.

`.npmrc` at the repo root must **not** set a hoisted linker:

```ini
# .npmrc — nested node_modules is load-bearing.
# rivetkit@2.3.9 depends on drizzle-orm ^0.44.2; apps/web needs 0.45.2 for the
# Better Auth adapter. Both must coexist. Do not add node-linker=hoisted.
engine-strict=true
auto-install-peers=true
```

CI asserts this holds: `pnpm why drizzle-orm` must show `^0.44.x` nested under `rivetkit` **and** `0.45.2` at `apps/web`.

---

## 5. Repository file tree

```
euchre/
├── package.json                              # workspace root; scripts: dev, test, lint, db:generate
├── pnpm-workspace.yaml
├── pnpm-lock.yaml                            # committed; CI runs `pnpm why drizzle-orm`
├── .npmrc                                    # default nested linker (rivetkit pins drizzle-orm ^0.44)
├── tsconfig.base.json
├── eslint.config.js                          # hosts the four custom rules (§3)
├── vitest.workspace.ts
├── .env.example
├── .github/workflows/ci.yml                  # typecheck · lint · unit · redaction-fuzz · adversarial · svelte-check
├── docs/
│   ├── 00-OVERVIEW.md   01-ARCHITECTURE.md   02-GAME-RULES-ENGINE.md
│   ├── 03-AI-AGENTS.md  04-FRONTEND-UX.md    05-IMPLEMENTATION-PLAN.md
│   ├── RULES.md                              # the A1–A10 rules the engine implements, with the A8 table
│   └── LEDGER.md                             # "What I bought, and what it costs" — every deletion, its price, its v2 path
│
├── packages/
│   ├── euchre-core/                          # ZERO runtime dependencies. The entire rulebook.
│   │   ├── package.json  tsconfig.json
│   │   └── src/
│   │       ├── index.ts
│   │       ├── types.ts                      # the shared type block, verbatim
│   │       ├── cards.ts                      # CARD_IDS, rankOf/suitOf, sameColor, effectiveSuit, trumpRank, plainRank
│   │       ├── rng.ts                        # mulberry32, hashSeed(seed, handNo), shuffle
│   │       ├── deck.ts                       # newDeck, dealPackets (3-2 / 2-3 alternation)
│   │       ├── seats.ts                      # nextSeat, partnerOf, teamOf, seatRole, firstActiveFrom, nextActiveSeat
│   │       ├── legal.ts                      # legalMoves(state, seat): LegalMove[] + whyIllegal copy
│   │       ├── trick.ts                      # cardValue, trickWinner
│   │       ├── score.ts                      # scoreHand — the exact A8 table, table-driven
│   │       ├── reduce.ts                     # apply(state, seat, move) -> { state, steps }   ← the whole game
│   │       ├── project.ts                    # project(state, seat) + projectSteps(steps, seat)
│   │       ├── optimistic.ts                 # applyOptimistic(view, move) for the browser
│   │       ├── strength.ts                   # handStrength, loner shortlist, per-seat thresholds
│   │       ├── heuristic.ts                  # rankMoves — fallback + coach + AI prior, one implementation
│   │       ├── notation.ts                   # encodeForLlm(view, legal, ranking) — dense line notation
│   │       ├── guard.ts                      # screenBanter(text, view) -> { ok } | { reject, reason }
│   │       ├── replay.ts                     # replay({seed, moves}) -> ReplayFrame[]  (kitty[1..3] + discard masked)
│   │       ├── script.ts                     # the fixed first-run tutorial deal (left bower T1 + a march)
│   │       ├── invariants.ts                 # V1..V12 assertions, dev + CI only
│   │       └── __tests__/
│   │           ├── effective-suit.spec.ts    legal.spec.ts        trick-winner.spec.ts
│   │           ├── scoring.spec.ts           # all 7 A8 rows + the clamp at 10
│   │           ├── loner.spec.ts             # trick-1 lead for each of the 4 loner seats; 15-card count
│   │           ├── stick-the-dealer.spec.ts  # dealer_must_call ≠ bidding_closed
│   │           ├── misdeal.spec.ts           # all-pass-twice throw-in; 3-streak moves the deal left
│   │           ├── full-game.spec.ts         # seeded game to 10, headless, < 50 ms
│   │           ├── optimistic-agreement.spec.ts   # applyOptimistic ≡ apply on the human's own legal set
│   │           └── redaction.fuzz.spec.ts    # 10,000 games — REQUIRED CI GATE
│   │
│   └── protocol/                             # wire contracts only; reviewed separately from the actors
│       ├── package.json  tsconfig.json
│       └── src/{index.ts,events.ts,queues.ts,views.ts,errors.ts}
│
├── apps/game/                                # RivetKit registry — its own Bun process, gateway :6420
│   ├── package.json                          # bun --watch src/index.ts
│   ├── tsconfig.json
│   └── src/
│       ├── index.ts                          # registry.start()
│       ├── registry.ts                       # setup({ use, envoy: { version: BUILD_VERSION } })
│       ├── build-version.ts                  # generated at build time; numeric BUILD_VERSION
│       ├── config.ts                         # env + the MODEL INJECTION SEAM (registry config -> create input)
│       ├── auth/verify.ts                    # jose + createRemoteJWKSet, cached in c.vars
│       ├── auth/origin.ts                    # allowlist for onBeforeConnect (Rivet has no WS CORS)
│       ├── actors/
│       │   ├── euchre-table/
│       │   │   ├── index.ts                  # the actor: run loop, advanceTurn, lifecycle
│       │   │   ├── queues.ts                 # canPublish guards + message shapes (turnId + seq on ALL)
│       │   │   ├── events.ts                 # canSubscribe guards
│       │   │   ├── fanout.ts                 # pushSync — the ONLY sender of card data
│       │   │   ├── dispatch-ai.ts            # cross-actor send, AbortSignal, never wait:true
│       │   │   ├── reconcile.ts              # the onWake reconciler (crash-between-ack-and-arm)
│       │   │   ├── tempo.ts                  # server-held gates: trick_resolve, hand_score, cut window
│       │   │   ├── talk-arbiter.ts           # ≤1 line/seat/trick, ≤2/trick, 2.5 s cooldown, dramatic override
│       │   │   ├── schema.ts                 # c.db hand journal
│       │   │   ├── drizzle.config.ts
│       │   │   └── drizzle/                  # committed migrations
│       │   ├── ai-seat/
│       │   │   ├── index.ts                  # workflow(); ctx.loop('turns') over decide|resetHand|handEnd|gameEnd
│       │   │   ├── model.ts                  # modelParams(modelId) allowlist + injected factory
│       │   │   ├── prompt.ts                 # 3-layer prompt + the cacheControl policy
│       │   │   ├── decide.ts                 # forced -> constrained LLM -> heuristic ladder
│       │   │   ├── banter.ts                 # streamText on public state, salience gate, leak filter
│       │   │   ├── phrasebook.ts             # canned per-persona lines: the leak/offline fallback
│       │   │   ├── memory.ts                 # per-hand voids / trumpSeen / humanRead, all capped
│       │   │   └── budget.ts                 # per-game token circuit breaker
│       │   └── player-profile/
│       │       ├── index.ts
│       │       ├── schema.ts                 # matches, hand_journal, personas, persona_versions, episodes, phrasebook
│       │       ├── drizzle.config.ts
│       │       └── drizzle/
│       └── __tests__/
│           ├── table-run-loop.spec.ts        # setupTest + fake timers, whole scripted hands
│           ├── auth.spec.ts                  # forged internal publish, IDOR on playerProfile, origin reject
│           ├── ai-ladder.spec.ts             # MockLanguageModelV4 array: illegal id, malformed JSON, timeout
│           ├── adversarial-persona.spec.ts   # injection corpus: 0 illegal moves, 0 card refs, 0 schema deviation
│           ├── cache-prefix.spec.ts          # asserts the Opus cached prefix exceeds 4096 tokens
│           ├── recovery.spec.ts              # every crash window incl. ack-then-arm; reconciler
│           ├── queue-consumers.spec.ts       # every declared queue has a consumer (consumer-or-delete)
│           └── metering.harness.ts           # p50/p95 + cacheRead/cacheWrite; run before freezing tempo/budget
│
├── apps/web/                                 # SvelteKit — seven server files, zero game logic
│   ├── package.json  svelte.config.js  vite.config.ts  drizzle.config.ts  tsconfig.json
│   ├── scripts/
│   │   ├── build-atlas.ts                    # resvg + sharp: 25 SVGs x 3 variants -> atlas PNG + atlas.json
│   │   └── card-art/                         # SVG sources: four-colour, two-colour, large-index
│   ├── static/cards/                         # build output, gitignored
│   └── src/
│       ├── app.d.ts  app.html                # viewport-fit=cover
│       ├── hooks.server.ts                   # getSession + svelteKitHandler({event,resolve,auth,building})
│       ├── lib/
│       │   ├── server/{auth.ts,db/index.ts,db/schema.ts,email.ts,rivet.ts}
│       │   ├── auth-client.ts
│       │   ├── client/{rivet.ts,table.svelte.ts,settings.svelte.ts}
│       │   ├── three/
│       │   │   ├── Stage.svelte              # <Canvas> + HUD siblings; canvas role="img" aria-label
│       │   │   ├── Rig.svelte  Lighting.svelte  Felt.svelte  Kitty.svelte
│       │   │   ├── Card.svelte  HitProxy.svelte  PlayerHand.svelte  OpponentHand.svelte
│       │   │   ├── TrickZone.svelte  TrickPile.svelte  DealerButton.svelte
│       │   │   ├── cardMotion.svelte.ts      # the timeline class; zero $state per frame
│       │   │   ├── director.svelte.ts        # applies sync{view,steps}; reconciles; snapshot-only mode
│       │   │   ├── layout.ts                 # hand arc; PROVES the 48x64 CSS px exposure floor
│       │   │   ├── atlas.ts                  # baked-UV geometry cache + deck-variant swap path
│       │   │   └── tempo.ts                  # the client tempo table x {brisk 0.7, table 1.0, slow 1.3}
│       │   ├── ui/
│       │   │   ├── Hud.svelte  ScoreBoard.svelte  TrumpBadge.svelte  TurnIndicator.svelte
│       │   │   ├── BidPanel.svelte           # LegalMove[]-driven: Pass / Order it up / I assist / I take it
│       │   │   ├── SuitPicker.svelte         # round 2, excludes turnedDownSuit; alone toggle rides the call
│       │   │   ├── DiscardPanel.svelte       # the 6-card fan; discard is just a card tap
│       │   │   ├── TalkLog.svelte  LastTrick.svelte  Celebration.svelte
│       │   │   ├── HandA11y.svelte           # the visually-hidden button list — playable on its own (M2)
│       │   │   ├── LiveRegions.svelte        # TWO regions: game-state (polite) + banter (default OFF)
│       │   │   ├── Coach.svelte  RulesPanel.svelte  IllegalWhy.svelte
│       │   │   └── Tutorial.svelte           # first-run scripted hand
│       │   └── a11y/{sound.ts,haptics.ts,announce.ts,motion.ts}
│       └── routes/
│           ├── +layout.svelte  +layout.server.ts
│           ├── login/+page.svelte
│           ├── login/check-email/+page.svelte
│           ├── auth/continue/+page.svelte    # SafeLinks/Proofpoint interstitial — requires a real click
│           ├── api/rivet-token/+server.ts    # 15-min JWT, audience 'euchre-actors'
│           └── (app)/
│               ├── +layout.server.ts         # redirect guard
│               ├── play/+page.server.ts      # profile.activeGame() -> resume or create (SERVER creates)
│               ├── play/[gameId]/{+page.server.ts,+page.svelte}
│               ├── games/{+page.server.ts,+page.svelte}
│               ├── games/[matchId]/+page.svelte          # replay through the SAME director
│               └── settings/{+page.server.ts,+page.svelte}
└── (no other top-level directories)
```

> **Spec note (existing scaffold).** The repository root currently holds a single-app SvelteKit starter (`adapter-vercel`, Tailwind, `better-sqlite3`, a root `src/`, `bun.lock`). It predates this spec and matches none of the binding decisions. M0 deletes it: remove `src/`, `static/`, `bun.lock`, `drizzle.config.ts`, `svelte.config.*`, `vite.config.ts`, `tsconfig.json` and the root `package.json`'s dependency block, then scaffold the tree above. `CLAUDE.md`, `.prettierrc`, `.vscode/` and `.mcp.json` may stay.

---

## 6. Environment variables

Two processes, two credential sets, and the split is deliberate: **`apps/game` never holds a database credential.** It verifies users by fetching the JWKS from `apps/web` over HTTP, so a compromise of the actor process yields no Postgres access and no session-signing secret.

| Variable | Process | Required | Meaning |
|---|---|---|---|
| `DATABASE_URL` | web | yes | Neon Postgres connection string. Holds Better Auth's five tables + `jwks` only. |
| `BETTER_AUTH_SECRET` | web | yes | ≥32 chars (`openssl rand -base64 32`). Signs sessions. |
| `BETTER_AUTH_URL` | web | yes | Public origin of `apps/web`, e.g. `http://localhost:5173`. Also the JWT issuer. |
| `RESEND_API_KEY` | web | no | Magic-link delivery. **Absent ⇒ the link is logged to the console**, so local dev never blocks. |
| `EMAIL_FROM` | web | no | Defaults to `Euchre <onboarding@resend.dev>`, which only delivers to the Resend account owner. |
| `PUBLIC_RIVET_ENDPOINT` | web (client) | yes | Gateway URL the **browser** connects to directly. `http://localhost:6420` in dev. |
| `APP_URL` | game | yes | Origin of `apps/web`; used to build the JWKS URL and as the expected JWT `issuer`. |
| `ALLOWED_ORIGINS` | game | yes | Comma-separated `onBeforeConnect` allowlist. Rivet has no WebSocket CORS. |
| `ANTHROPIC_API_KEY` | game | yes | The only LLM credential. Never reaches `apps/web` or a browser. |
| `AI_PLAY_MODEL` | game | no | Defaults `claude-haiku-4-5`. Card plays. |
| `AI_BID_MODEL` | game | no | Defaults `claude-opus-5`. Bids, dealer discard, go-alone. |
| `RIVET_ENDPOINT` | game | no | Engine endpoint. Unset ⇒ `registry.start()` boots a local engine on 6420. |
| `RIVET_NAMESPACE` | game | no | Defaults `default`; `production` in prod. |
| `RIVET_TOKEN` | game | prod only | Engine auth when connecting to a hosted engine. |
| `RIVET_RUNNER_VERSION` | game (build) | prod only | Monotonic build integer → `envoy.version`. Set `RIVET_ENVOY_VERSION` to the same value. |
| `RIVETKIT_STORAGE_PATH` | game | no | Local actor-state/SQLite path in dev. |
| `RIVET_EXPOSE_ERRORS` | game | no | `1` in dev only — returns full error detail to clients instead of `internal_error`. |

`apps/game` parses these in exactly one place, and nowhere else reads `process.env`:

```ts
// apps/game/src/config.ts — the ONLY module in apps/game that reads process.env.
import { z } from "zod";

const Env = z.object({
  APP_URL: z.url(),
  ALLOWED_ORIGINS: z.string().min(1),
  ANTHROPIC_API_KEY: z.string().min(1),
  AI_PLAY_MODEL: z.string().min(1).default("claude-haiku-4-5"),
  AI_BID_MODEL: z.string().min(1).default("claude-opus-5"),
  RIVET_NAMESPACE: z.string().min(1).default("default"),
});

export interface GameConfig {
  readonly appUrl: string;
  readonly jwksUrl: URL;
  readonly allowedOrigins: readonly string[];
  readonly anthropicApiKey: string;
  readonly playModelId: string;
  readonly bidModelId: string;
  readonly namespace: string;
}

const parsed = Env.parse(process.env);

export const config: GameConfig = {
  appUrl: parsed.APP_URL,
  jwksUrl: new URL("/api/auth/jwks", parsed.APP_URL),
  allowedOrigins: parsed.ALLOWED_ORIGINS.split(",").map((o) => o.trim()).filter(Boolean),
  anthropicApiKey: parsed.ANTHROPIC_API_KEY,
  playModelId: parsed.AI_PLAY_MODEL,
  bidModelId: parsed.AI_BID_MODEL,
  namespace: parsed.RIVET_NAMESPACE,
};
```

The model **factory** is built from `config` here and injected through registry config into each `aiSeat`'s create input (invariant 19), which is why `MockLanguageModelV4` can replace it in tests without any env var being set.

### `.env.example`

```dotenv
# ─────────────── apps/web (node 24, port 5173 dev / 3000 prod) ───────────────
DATABASE_URL=postgres://user:password@ep-xxx.us-east-2.aws.neon.tech/euchre?sslmode=require
BETTER_AUTH_SECRET=replace-me-openssl-rand-base64-32
BETTER_AUTH_URL=http://localhost:5173

# Optional. With no key the magic link is printed to the server console.
RESEND_API_KEY=
EMAIL_FROM=Euchre <onboarding@resend.dev>

# The browser connects to the Rivet gateway DIRECTLY. Must be PUBLIC_-prefixed.
PUBLIC_RIVET_ENDPOINT=http://localhost:6420

# ─────────────── apps/game (bun 1.3.14, gateway port 6420) ───────────────
APP_URL=http://localhost:5173
ALLOWED_ORIGINS=http://localhost:5173
ANTHROPIC_API_KEY=sk-ant-replace-me
AI_PLAY_MODEL=claude-haiku-4-5
AI_BID_MODEL=claude-opus-5

RIVET_NAMESPACE=default
RIVETKIT_STORAGE_PATH=./.rivet
RIVET_EXPOSE_ERRORS=1
# RIVET_ENDPOINT=            # unset in dev: registry.start() runs a local engine
# RIVET_TOKEN=               # production only
# RIVET_RUNNER_VERSION=1     # production only; build-time integer
# RIVET_ENVOY_VERSION=1      # production only; must equal RIVET_RUNNER_VERSION
```

---

## 7. Open questions, chosen defaults, and how to change them

**7.1 LLM provider and the two model tiers.** *Default:* Anthropic via `@ai-sdk/anthropic@4.0.21` — `claude-haiku-4-5` for card plays (persona temperature 0.2–0.8), `claude-opus-5` for bids/discard/go-alone with **no sampling parameters at all** and `thinking: { type: 'disabled' }`. Two provider-specific facts are baked in: the Opus-5 family returns HTTP 400 on `temperature`/`top_p`/`top_k` and the SDK does not strip them (hence `modelParams()`), and Haiku 4.5's 4096-token prompt-cache floor decides whether a `cacheControl` breakpoint is worth the 1.25× write premium. *To change:* the model is injected at registry level, so a provider swap touches two files — `apps/game/src/config.ts` and `apps/game/src/actors/ai-seat/model.ts`. It invalidates `cache-prefix.spec.ts` and the parameter allowlist, nothing else.

**7.2 Postgres host for the Better Auth tables.** *Default:* Neon over `postgres-js`. Postgres holds Better Auth's five tables plus `jwks` — no game data, no history, no personas; all of that lives in actor-local SQLite. It is the one credential `apps/web` needs and `apps/game` deliberately does not. *To change:* any wire-protocol Postgres is a `DATABASE_URL` change. Moving to SQLite would break the two-writer assumption *only* if game data moved there, which the storage split forbids.

**7.3 Is AI banter on by default?** *Default:* **on**, with the leak filter, salience gate and phrasebook fallback, a per-user off switch in Settings, and a *separate, default-off* `aria-live` region for screen readers. Banter is simultaneously the only genuine hidden-information leak path and the largest "feels like a real table" differentiator. The regex screen catches explicit rank+suit references but not paraphrase ("I've got the big one"), so it is further bounded by public-state-only prompting, a 90-character cap, an instruction-level ban on claims about one's own holdings, and a logged rejection trail. *To change:* if a pre-launch review of logged utterances finds a soft-leak rate above ~1%, flip the default off. The game is complete without it.

**7.4 Deployment target.** *Default:* `apps/game` as a long-lived Bun runner process (`registry.start()`, runner mode, `drainOnVersionUpgrade` false); `apps/web` on `adapter-node` behind the same domain; the browser connects to the gateway on 6420 **directly**. Not serverless. Runner mode is what keeps a live hand from being drained mid-trick on deploy; the direct connection is what keeps SvelteKit off the realtime hot path. *To change:* a serverless target flips `drainOnVersionUpgrade` to true and requires re-deriving the deploy-mid-match row of the recovery table in `01-ARCHITECTURE.md`.

**7.5 Game length and stick-the-dealer.** *Default:* `gameTo: 10`, `stickTheDealer: true` — both `cfg` flags the reducer branches on. `5 | 7 | 11` and `stickTheDealer: false` are implemented and tested (the `false` path is what exercises the throw-in and the deal-moves-left misdeal) but neither is exposed in the UI in v1. Ten is the North American and tournament norm and is what the scoring-clamp test asserts; stick-the-dealer is now dominant, eliminates dead hands, and is what makes `dealer_must_call` a distinct machine code from `bidding_closed`. *To change:* exposing either is a Settings row, not a code change.

---

## 8. Running it locally

**Prerequisites:** node 24.11.0, pnpm 10.19.0 (`corepack enable`), bun 1.3.14, a Neon (or any) Postgres URL. An `ANTHROPIC_API_KEY` is needed only from M4 onward; before that the AI seats are heuristic and the game is fully playable offline.

```bash
git clone <repo> euchre && cd euchre
cp .env.example .env                 # fill DATABASE_URL + BETTER_AUTH_SECRET at minimum
pnpm install                         # nested linker; verify with `pnpm why drizzle-orm`

pnpm --filter @euchre/web exec npx auth@latest generate   # writes the 5 Better Auth tables into schema.ts
pnpm db:generate                     # drizzle-kit generate for Postgres AND each actor SQLite folder
pnpm --filter @euchre/web exec drizzle-kit migrate         # apply Postgres migrations
pnpm --filter @euchre/web build:atlas                      # resvg + sharp -> static/cards/atlas.png + atlas.json

pnpm dev                             # runs both processes concurrently
```

| Process | Command | Port | Serves |
|---|---|---|---|
| `apps/game` | `bun --watch src/index.ts` | **6420** | Rivet engine + gateway. Browsers connect here over WebSocket. Inspector at `/inspector/state`, `/inspector/queue`, `/inspector/summary`. |
| `apps/web` | `vite dev` | **5173** | SvelteKit: auth, `/play`, `/games`, `/settings`, `/api/rivet-token`. |

`pnpm dev` at the root is `concurrently` over `pnpm --filter @euchre/game dev` and `pnpm --filter @euchre/web dev`. Do not merge the two processes: SvelteKit's HMR re-evaluates modules, which would repeatedly re-instantiate the registry and thrash actor versions, and `registry.start()` owns SIGTERM handling you do not want entangled with Vite.

**First run.** Open `http://localhost:5173`, enter any email, and — with no `RESEND_API_KEY` — copy the magic link printed in the `apps/web` console. `/play` calls `playerProfile.activeGame()`; with no active game the **server** creates a `euchreTable`, snapshots three personas into it, and redirects to `/play/[gameId]`.

**Tests.**

```bash
pnpm -r typecheck                     # tsc --noEmit across every workspace
pnpm -r lint                          # eslint incl. the four custom rules
pnpm --filter @euchre/web check       # svelte-check 4.7.3
pnpm test                             # vitest --run, whole workspace
pnpm --filter @euchre/core test -- --coverage    # 100% branch gate on reduce/legal/score
pnpm --filter @euchre/core test redaction.fuzz   # the 10,000-game gate
pnpm --filter @euchre/game test                  # setupTest + fake timers; zero network
```

Every test in `@euchre/core` and `apps/game` runs with no network and no API key: the engine is pure, and the model is injected (`MockLanguageModelV4`). Only `metering.harness.ts` makes real calls, and it is not part of `pnpm test`.

---

## 9. Glossary

Terms used without further explanation in `01`–`05`.

**Bower** — a jack that is trump. The **right bower** is the jack of the trump suit and is the highest card in the deck; the **left bower** is the jack of the *other suit of the same colour* (♠↔♣, ♥↔♦), is the second-highest card, and **belongs to the trump suit for every purpose** — following suit, voidness, leading, and the trick comparator. This is `effectiveSuit()`, and it is the single most important function in the codebase.

**Trump** — the suit that outranks all others for one hand. Ranked high to low: right bower, left bower, A, K, Q, 10, 9 — **seven** cards. The suit that lost its jack to the left bower has only five; the two off-colour suits have six each. Plain suits rank A K Q J 10 9.

**Trick** — one card contributed by each active player, played clockwise. Won by the highest trump, or if no trump was played, the highest card of the suit that was led. Five tricks per hand.

**Follow suit** — you must play a card of the led suit (by *effective* suit) if you hold one. If you are void you may play anything; there is no obligation to trump. Failing to follow when able is a **renege** (or revoke) and normally ends the hand with 2 points to the other side (4 against a loner). Structurally impossible here: illegal cards are absent from the legal set server-side, unclickable client-side, and never offered to an AI.

**Kitty** — the four undealt cards. `kitty[0]` is turned face up as the **up-card**; `kitty[1..3]` are buried and never seen by anyone, ever, including in replay.

**Up-card** — the face-up kitty card that round-1 bidding is about. If everyone passes, the dealer **turns it down** and it stays *publicly identified* on the table — load-bearing, because every player uses it to count the seven trump.

**Bidding** — two rounds. Round 1: each seat in turn either **passes** or accepts the up-card's suit as trump. Round 2 (only if all four passed): each seat either passes or **calls** any suit except the turned-down one.

**Order it up / I assist / I take it** — the three spoken forms of the same round-1 acceptance, distinguished by who says it. An opponent of the dealer *orders it up*; the dealer's partner *assists*; the dealer *takes it*. The engine emits the right phrase from `seatRole`.

**Next / Crossing the creek** — round-2 calls. **Next** is the same colour as the turned-down suit (strong from first seat: four passes is evidence the turned suit is dead and the next-colour bower is live). **Crossing the creek** is the opposite colour (better from second and fourth seat).

**Maker** — the player who fixed trump. Their side has undertaken to win at least three tricks.

**Dealer discard** — if the up-card suit becomes trump in round 1, the dealer *must* take the up-card into hand and discard one of their six cards face down. The discard is never revealed to any client, ever. Legal to discard the up-card itself.

**Stick the dealer** — with the default `stickTheDealer: true`, the dealer may not pass in round 2; if the other three pass, the dealer must name a suit. Rejected passes here return the distinct code `dealer_must_call`, never `bidding_closed`, and the UI removes the Pass button rather than disabling it.

**Going alone / loner** — the maker may declare "alone" *simultaneously with* the winning call. Their partner sits out with five dead cards; every trick that hand has three cards, not four. A lone maker taking all five scores **4**.

**March** — the makers take all five tricks: 2 points (4 if alone).

**Euchre / set** — the makers take fewer than three tricks: **2 points to the defenders**, alone or not.

**Throw-in / misdeal** — with `stickTheDealer: false`, all four passing twice throws the hand in: the deal moves left and a new hand is dealt. Three consecutive misdeals by the same dealer also move the deal left.

**Eldest hand** — the seat to the dealer's left. Bids first and leads the first trick — except under a loner, where the lead falls to the first *active* seat clockwise from eldest.

**Cut / bump** — before the deal, the player to the dealer's right may cut the deck or decline ("bump", knocking the deck). Here it is a real, seeded, deterministic cut of an already-persisted deck order, so a crash cannot reshuffle.

**In the barn** — at 9 points, one from winning. **Whitewashed / skunked** — losing 10–0. **Dutchman's Point** — holding both bowers and the ace of trump.

**Donation** — deliberately calling a hand you expect to lose when the opponents are at 9, conceding 2 rather than risking a 4-point loner.

**Throwing off / laying off** — playing your worst card when your partner has already won the trick. Never trump your partner's winner.

**Second hand low, third hand high** — playing second to a trick, play cheap (two players still act after you). Playing third, if your partner is not already winning, play your highest legal card.

**Table talk** — giving your partner information by any means other than the cards you play. Illegal at a real table, and the highest-risk LLM behaviour in this product. AI banter is generated from public state only, capped at 90 characters, screened for card references, and replaced by a canned phrasebook line on rejection.
