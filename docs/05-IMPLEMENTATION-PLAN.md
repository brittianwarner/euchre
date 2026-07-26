# 05 — Implementation Plan

Build order, exit criteria, test strategy, CI, risk, and scope boundary for the euchre app specified in
`00-OVERVIEW.md` (product), `01-ARCHITECTURE.md` (actors, wire protocol, file tree),
`02-GAME-RULES-ENGINE.md` (`@euchre/core`), `03-AI-AGENTS.md` (`aiSeat`, prompts, ladder) and
`04-FRONTEND-UX.md` (Threlte, interaction, accessibility).

Every version number below is pinned exactly as in `01-ARCHITECTURE.md`. Estimates are engineer-days for one
senior full-stack engineer who has read all five documents; the critical path is called out in §3.

---

## 1. Sequencing principle

The order is chosen so that **a complete, winnable game of euchre exists at M2** — before any three.js, any LLM,
and any auth — and every later milestone is an addition to a working product rather than a prerequisite for one.
Three properties make that possible:

1. **The rulebook is a pure package** (`packages/euchre-core`, zero runtime dependencies). It is fully testable
   with no server, no browser and no network, so it can be finished and frozen first.
2. **The AI seat is a swappable decision function.** `rankMoves(state, seat)[0]` is a complete, competent euchre
   player. M2 ships it as the opponent; M4 replaces the *source* of the decision without touching the table.
3. **The accessible DOM hand is milestone zero of the UI.** `HandA11y.svelte` is a focusable button list that the
   accessibility layer requires anyway; shipped un-hidden it is a playable card table. The 3D layer at M6 is
   therefore additive and genuinely severable under schedule pressure.

Consequences worth stating plainly: auth (M3) lands *after* the playable artifact because a local dev game with a
stubbed connection state is faster to iterate on than one behind a magic link; and the LLM (M4) lands after auth
because `aiSeat` is a per-user-budgeted actor whose token ceiling is meaningless without an identity.

> **Spec note:** the repository currently contains a single-app `sv create` scaffold at its root
> (`svelte.config.js`, `src/routes`, `bun.lock`, `@sveltejs/adapter-vercel`, Tailwind). It predates this spec and
> conflicts with the pinned stack (adapter-node, no Tailwind, pnpm lockfile). M0 deletes it and rebuilds the tree
> from `01-ARCHITECTURE.md`; nothing in it is carried forward.

---

## 2. Milestone index

| # | Title | Depends on | Est. (d) | Ships what |
|---|---|---|---|---|
| M0 | Monorepo, toolchain, four lint rules | — | 2 | Green CI on an empty tree |
| M1 | `@euchre/core` — the entire rulebook | M0 | 6 | Headless euchre |
| M2 | **Playable artifact**: table actor + heuristic AI + DOM client | M1 | 6 | A winnable game in a browser |
| M3 | Auth end to end, every authorization boundary | M2 | 3 | Magic link, JWT, IDOR-proof actors |
| M4 | `aiSeat` — the decision ladder, deterministically testable | M3 | 5 | Real LLM opponents |
| M5 | Personas, three-layer prompt, Settings | M4 | 4 | Character, and a Settings page |
| M6 | The Threlte table | M2, M5 | 8 | The 3D card table |
| M7 | Tempo, sound, haptics, banter | M6 | 4 | The "real table" feel |
| M8 | `playerProfile` — history, replay, memory | M5 | 4 | `/games`, replay, episodes |
| M9 | Onboarding, accessibility, mobile | M7 | 5 | Teachability and reach |
| M10 | Hardening, soak, deploy | M8, M9 | 4 | Production |

**Total 51 engineer-days.** M8 is off the critical path (it depends only on M5) and can run in parallel with
M6/M7, which shortens a two-engineer schedule to roughly 47 days of critical path: M0→M1→M2→M3→M4→M5→M6→M7→M9→M10.

---

## 3. Milestones

### M0 — Monorepo, toolchain, and the four lint rules · 2 d

**Deliverable.** A pnpm workspace containing four empty-but-typechecking packages, a lockfile, the ESLint flat
config that hosts the four binding-invariant rules, and CI that runs typecheck + lint + the rules' own tests.

**Files created.** `package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml`, `.npmrc`, `tsconfig.base.json`,
`eslint.config.js`, `eslint.config.spec.ts`, `vitest.config.ts`, `.env.example`, `.github/workflows/ci.yml`,
`docs/RULES.md`, `docs/LEDGER.md`, and the four package manifests: `packages/euchre-core/{package.json,tsconfig.json}`,
`packages/protocol/{package.json,tsconfig.json}`, `apps/game/{package.json,tsconfig.json}`,
`apps/web/{package.json,tsconfig.json,svelte.config.js,vite.config.ts}`.
**Files deleted.** The entire pre-existing root scaffold listed in the spec note above.

> **Spec note:** `01-ARCHITECTURE.md` lists `vitest.workspace.ts`. Vitest 4 removed `defineWorkspace` and the
> workspace file in favour of `test.projects` inside the root config (verified against the installed
> `vitest@4.1.10` type declarations — `defineWorkspace` is absent, `projects?: TestProjectConfiguration[]` is
> present). The file is therefore named `vitest.config.ts`; nothing else about the layout changes.
>
> **Spec note:** ESLint 10 loads a TypeScript flat config only with `jiti` present. The config is authored as
> ESM JavaScript with JSDoc types (`eslint.config.js`, exactly as in the file tree) so no extra dependency and no
> loader are needed; its rules are `export`ed so `eslint.config.spec.ts` can drive them through `RuleTester`.

```yaml
# pnpm-workspace.yaml
packages:
  - "packages/*"
  - "apps/*"
```

```
# .npmrc — the nested (isolated) linker is REQUIRED: rivetkit@2.3.9 pins drizzle-orm ^0.44.x
# while apps/web needs 0.45.2 for @better-auth/drizzle-adapter. Hoisting collapses them and breaks one of the two.
node-linker=isolated
engine-strict=true
strict-peer-dependencies=false
```

```ts
// vitest.config.ts — Vitest 4 `projects`, not the removed workspace file.
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    projects: [
      {
        test: {
          name: "core",
          root: "./packages/euchre-core",
          environment: "node",
          coverage: {
            provider: "v8",
            include: ["src/reduce.ts", "src/legal.ts", "src/score.ts"],
            thresholds: { branches: 100, functions: 100, lines: 100, statements: 100 },
          },
        },
      },
      { test: { name: "game", root: "./apps/game", environment: "node" } },
      { test: { name: "web", root: "./apps/web", environment: "node" } },
      { test: { name: "lint-rules", include: ["eslint.config.spec.ts"], environment: "node" } },
    ],
  },
});
```

One rule in full; the other three are the same shape with the predicates given after it.

```js
// eslint.config.js (excerpt)
/** @type {import("eslint").Rule.RuleModule} */
const noSuitCompare = {
  meta: {
    type: "problem",
    schema: [],
    messages: {
      banned:
        "suitOf() may not be used in a suit comparison outside cards.ts. Use effectiveSuit(card, trump): " +
        "with hearts trump, the jack of diamonds IS a heart, and suitOf() will silently disagree.",
    },
  },
  create(context) {
    if (context.filename.endsWith("packages/euchre-core/src/cards.ts")) return {};
    const isSuitOf = (n) =>
      n?.type === "CallExpression" && n.callee.type === "Identifier" && n.callee.name === "suitOf";
    return {
      BinaryExpression(node) {
        if (!["===", "!==", "==", "!="].includes(node.operator)) return;
        if (isSuitOf(node.left) || isSuitOf(node.right)) context.report({ node, messageId: "banned" });
      },
      SwitchStatement(node) {
        if (isSuitOf(node.discriminant)) context.report({ node, messageId: "banned" });
      },
      MemberExpression(node) {
        // `voids[suitOf(c)]`, `set.has(suitOf(c))` — indexing by raw suit is the same defect.
        if (node.computed && isSuitOf(node.property)) context.report({ node, messageId: "banned" });
      },
    };
  },
};
export const rules = { "no-suit-compare": noSuitCompare /* + the three below */ };
```

| Rule | AST predicate | Fires when |
|---|---|---|
| `no-broadcast-state` | `CallExpression` on `c.broadcast` / `<x>.broadcast` whose arguments contain a `MemberExpression` reaching `.hands`, `.kitty`, `.dealerDiscard`, or an identifier named `state`/`gameState` | a broadcast could carry hidden cards |
| `save-before-complete` | in any function containing an assignment whose object chain starts `c.state`, the first `<m>.complete(` call is not preceded in source order by `await c.saveState({ immediate: true })` | ack before durability |
| `protected-workflow-step-names` | string literal argument to `ctx.step(` / `ctx.loop(` / `ctx.sleep(` outside the frozen set `{seen, rank, llm, validate, reply, meter, turns}` and not wrapped in `ctx.removed(` | a rename that would diverge in-flight workflow replay |

**Tasks.**
- [ ] Delete the legacy root scaffold; `git init`; first commit is the empty workspace.
- [ ] Write `pnpm-workspace.yaml`, `.npmrc`, `tsconfig.base.json` (`strict`, `target: es2022`, `moduleResolution: bundler`, `verbatimModuleSyntax`, `noUncheckedIndexedAccess`).
- [ ] Install the exact pinned versions; commit `pnpm-lock.yaml`.
- [ ] Root scripts: `dev` (`pnpm -r --parallel dev`), `test`, `test:ci`, `lint`, `typecheck`, `check` (svelte-check), `db:generate` (`find apps/game/src/actors -name drizzle.config.ts -exec drizzle-kit generate --config {} \;`).
- [ ] Author the four rules plus, for each, one passing and one deliberately-failing `RuleTester` fixture.
- [ ] `apps/game/src/index.ts` = `registry.start()` over an empty registry; `apps/web` = a bare SvelteKit app on adapter-node.
- [ ] CI workflow (§6) running typecheck, lint, unit, and the dependency assertion.

**Exit criteria.**
1. `pnpm -r typecheck` and `pnpm -r lint` exit 0 on a clean checkout.
2. `pnpm why drizzle-orm` prints a `^0.44.x` resolution nested under `rivetkit` *and* `0.45.2` at `apps/web`; CI asserts both strings are present and fails if either disappears.
3. All four rules have a green fixture and a red fixture; `vitest --project lint-rules` proves each rule reports exactly once on its red fixture.
4. `pnpm dev` brings up `apps/game` on `:6420` (Bun, `bun --watch src/index.ts`) and `apps/web` on `:5173` (node 24.11.0) concurrently, and both survive a file save.

---

### M1 — `@euchre/core`: the entire rulebook, headless · 6 d

**Deliverable.** `packages/euchre-core` complete and frozen: the `types.ts` block from `01-ARCHITECTURE.md`
verbatim, plus `cards, rng, deck, seats, legal, trick, score, reduce, project, optimistic, strength, heuristic,
notation, guard, replay, script, invariants` and the full test suite. Zero runtime dependencies.

**Files.** Every file under `packages/euchre-core/src/` and `packages/euchre-core/src/__tests__/` in the tree;
`packages/protocol/src/{index,events,queues,views,errors}.ts` (wire contracts re-exported from core types).

> **Spec note:** the reducer contract in `01-ARCHITECTURE.md` declares `apply`, but a `GameState` has to come from
> somewhere and the engine-only phases (`deal`, `trick_resolve`, `hand_score`) have no acting seat. Two further
> exports close this: `createGame(init): GameState` — the sole constructor, called by the table's `createState`
> and by every test — and `advance(state): ApplyResult` — the total transition for engine-only phases, a no-op in
> any phase with a `turnSeat`. The table calls `advance` from its `onTempoGate` handler; the fuzz harness calls it
> in the same loop. No euchre rule moves outside this package.

**Tasks.**
- [ ] `types.ts` copied verbatim; `tsc --strict` clean.
- [ ] `cards.ts`: `CARD_IDS` (24), `effectiveSuit`, `trumpRank`, `plainRank`, `sameColor` — the only file allowed to compare `suitOf()`.
- [ ] `rng.ts`: `mulberry32`, `hashSeed(seed, handNo)`, `shuffle`, tested against a checked-in golden sequence.
- [ ] `deck.ts`: `dealPackets` in the 3-2 / 2-3 alternation, emitting `Step.dealt`; asserts 24 cards accounted for.
- [ ] `legal.ts`: all six acting phases, including stick-the-dealer (`pass` removed, `dealer_must_call` on violation), the dealer's six-card discard set, `[]` for `sittingSeat`, and `whyIllegal` copy for the `Shift+arrow` learner path.
- [ ] `trick.ts` / `score.ts`: comparator and the A8 table, both table-driven.
- [ ] `reduce.ts`: `createGame`, `apply`, `advance` — both bidding rounds, going alone, misdeal/throw-in with `misdealLimit`, scoring, clamp at `cfg.gameTo`.
- [ ] `project.ts`: `project` + `projectSteps` (blanks the card on any `dealt` packet not addressed to this seat).
- [ ] `optimistic.ts`, `strength.ts`, `heuristic.ts`, `notation.ts`, `guard.ts`, `replay.ts`, `script.ts`, `invariants.ts` (V1–V12), and the ten spec files in the tree.

**Exit criteria.**
1. A seeded full game to 10 plays headless in Vitest in **< 50 ms** with no network and no actor.
2. All seven A8 rows plus the clamp (a march at 9 ends 10–x, never 11) pass as a table-driven test.
3. Trick-1 lead under a loner is correct for each of the four possible loner seats, including the case where the sitting partner is eldest and the lead falls to the dealer's partner.
4. V10 asserts at hand end: 20 cards played normally, 15 under one loner.
5. `fast-check` property test over 200 000 random `(card, trump)` pairs: `effectiveSuit` agrees with the bower rule, and follow-suit legality never admits a renege.
6. `applyOptimistic` and `apply` agree on the human's own legal set across 10 000 random states.
7. The 10 000-game redaction fuzz passes (§4.5), replay frames included.
8. Branch coverage on `reduce.ts`, `legal.ts`, `score.ts` is 100 % (enforced by the thresholds in `vitest.config.ts`).

---

### M2 — Playable artifact: `euchreTable` + heuristic AI + DOM-only client · 6 d

**Deliverable.** The authoritative table actor with its single serialized run loop, all five queues and their
`canPublish` guards, the five events, the schedules, the server-held tempo gates, and the `onWake` reconciler.
AI seats are stubbed *inside the table* with `rankMoves()[0]` behind the persisted pacing floor — no `aiSeat`
actor, no LLM, no API key. The client is `HandA11y.svelte` **un-hidden**, plus `BidPanel`, `SuitPicker`,
`DiscardPanel`, `ScoreBoard`, `TalkLog`. No three.js exists yet.

**Files.** `apps/game/src/{index,registry,config}.ts`; `apps/game/src/actors/euchre-table/{index,queues,events,fanout,dispatch-ai,reconcile,tempo,talk-arbiter,schema,drizzle.config}.ts` + `drizzle/`;
`apps/game/src/__tests__/{table-run-loop,recovery,queue-consumers}.spec.ts`;
`apps/web/src/lib/client/{rivet,table.svelte.ts}`; `apps/web/src/lib/ui/{HandA11y,BidPanel,SuitPicker,DiscardPanel,ScoreBoard,TalkLog,LastTrick,IllegalWhy}.svelte`;
`apps/web/src/routes/(app)/play/{+page.server.ts}` and `play/[gameId]/{+page.server.ts,+page.svelte}`.

**Tasks.**
- [ ] Registry with `envoy: { version: BUILD_VERSION }` from a build-time `RIVET_RUNNER_VERSION` — never `Date.now()`.
- [ ] `submitMove` as the only connection-reachable mutation: stamp the seat from `c.conn.state`, then `c.queue.enqueueAndWait("move", …, { timeout: 8000 })`.
- [ ] The run loop over `c.queue.iter({ completable: true })`: constant-time `internalToken` check on internal messages → dedupe on `appliedSeq`/`recentMoveIds` → `apply` → `await c.saveState({ immediate: true })` → `pushSync` → `m.complete(...)` → arm the next turn. `RuleError` completes with a typed rejection carrying `metadata.legal`, never left unacked.
- [ ] `fanout.ts`: `pushSync` is the only sender of card data, per connection.
- [ ] `tempo.ts`: `trick_resolve` (700–1000 ms; 1100 ms sealing a euchre), `hand_score` (1400–2400 ms) and the cut window (2200 ms) as server-held phases gated by `c.schedule.after(gate, "onTempoGate", turnId)`.
- [ ] Schedules `onTurnOpen`, `onNudge` (90 s), `onAbandon` (240 s), `onAiTimeout`, `releaseAiMove`, `onTempoGate`, `flushProfile` — each action does nothing but `c.queue.send(...)`.
- [ ] `reconcile.ts`: on wake, re-derive the expected action from `(phase, turnSeat, turnId, pending)` and re-arm / re-dispatch / release a parked decision past its `revealAt`.
- [ ] `c.db` hand journal (Drizzle, committed migrations); client store `table.svelte.ts` as a `$state` class applying `sync` and snapping back on `UserError`.

**Exit criteria.**
1. A human plays a complete game to 10 in a browser against three heuristic opponents — both bidding rounds, dealer discard, stick-the-dealer, going alone, scoring — using only DOM buttons.
2. Illegal cards are absent from the server legal set and `disabled` client-side; a forged `submitMove` for an illegal move returns `UserError` with `metadata.legal`.
3. `setupTest` + fake timers drives a scripted 10-point game end to end with zero network.
4. Kill the game process mid-trick and restart: the unacked message is redelivered, `appliedSeq` makes reapplication a no-op, play resumes at the same turn.
5. Kill the process in the ack-then-arm window: the reconciler re-arms and the game advances. This is an explicit test, not an assertion.
6. `queue-consumers.spec.ts` proves every declared queue has a consumer, and fails the build otherwise.
7. With the browser closed, `onNudge` fires at 90 s and `onAbandon` at 240 s; the game terminates rather than hanging.

---

### M3 — Auth end to end, and every authorization boundary · 3 d

**Deliverable.** Better Auth 1.6.25 with `magicLink` + `jwt` + `sveltekitCookies(getRequestEvent)` **last**;
Resend delivery with a console fallback; the `/auth/continue` interstitial; Neon Postgres holding only the five
auth tables plus `jwks`; `/api/rivet-token`; and `onBeforeConnect` + `createConnState` on **all three** actors.

**Files.** `apps/web/src/lib/server/{auth,db/index,db/schema,email,rivet}.ts`, `apps/web/src/lib/auth-client.ts`,
`apps/web/src/hooks.server.ts`, `apps/web/src/app.d.ts`, `apps/web/drizzle.config.ts`,
`apps/web/src/routes/login/{+page.svelte,check-email/+page.svelte}`, `apps/web/src/routes/auth/continue/+page.svelte`,
`apps/web/src/routes/api/rivet-token/+server.ts`, `apps/web/src/routes/(app)/+layout.server.ts`,
`apps/game/src/auth/{verify,origin}.ts`, `apps/game/src/__tests__/auth.spec.ts`.

**Tasks.**
- [ ] `auth.ts` on `process.env` only (the `auth` CLI cannot resolve `$env/static/private`), with `magicLink({ expiresIn: 900, storeToken: "hashed" })`, `jwt({ audience: "euchre-actors", expirationTime: "15m" })` and `sveltekitCookies(getRequestEvent)` **last**.
- [ ] `hooks.server.ts` returns `svelteKitHandler({ event, resolve, auth, building })`, not `resolve(event)`. No `[...all]` route exists in 1.6.x.
- [ ] `npx auth@latest generate` → `drizzle-kit generate` → `drizzle-kit migrate`; migrations committed.
- [ ] `email.ts` branches on Resend's `{ data, error }` (it never throws) and logs the URL when `RESEND_API_KEY` is unset.
- [ ] `/auth/continue` requires a real click before `/magic-link/verify`, defeating SafeLinks/Proofpoint prefetch.
- [ ] Client `getParams` (never static `params`) re-mints the JWT on every connect and reconnect.
- [ ] `verify.ts`: `createRemoteJWKSet` against `${APP_URL}/api/auth/jwks`, cached in `c.vars`; `origin.ts` allowlist in `onBeforeConnect`.
- [ ] `createConnState` on `playerProfile` asserts `c.key[1] === claims.userId`; every action re-checks `c.conn.state`.

**Exit criteria.**
1. Magic-link sign-in completes against a real Resend send; with no key the link is logged and dev never blocks.
2. Prefetching the email link does not burn the token; the interstitial requires a click.
3. An expired JWT is rejected at connect, and `getParams` re-mints so the connection self-heals without a reload.
4. IDOR: user B connecting to `playerProfile` keyed `["user", A]` is rejected inside `createConnState`.
5. Forged publish: a stateless handle publishing `aiDecision` with a guessed `turnId` is dropped and logged as an attack. **This test must fail when the `internalToken` check is deleted** — that is what proves `canPublish` is not the boundary.
6. An unlisted `Origin` is rejected.
7. CI asserts `sveltekitCookies` is last in `plugins` and that `handle` returns `svelteKitHandler`.

---

### M4 — `aiSeat`: the decision ladder, deterministically testable · 5 d

**Deliverable.** `aiSeat` as a `workflow()` with `ctx.loop("turns")` over `decide | resetHand | handEnd | gameEnd`;
the `modelParams()` allowlist; the registry-level model-factory injection seam; the
forced → constrained-LLM → heuristic ladder; the persisted reveal floor; the 4000 ms watchdog; the per-game token
circuit breaker; and the metering harness that **freezes the numbers**.

**Files.** `apps/game/src/actors/ai-seat/{index,model,prompt,decide,banter,phrasebook,memory,budget}.ts`;
`apps/game/src/config.ts` (model injection); `apps/game/src/actors/euchre-table/dispatch-ai.ts` (real dispatch);
`apps/game/src/__tests__/{ai-ladder,cache-prefix,metering.harness}.ts`.

**Tasks.**
- [ ] Workflow step names frozen at `seen, rank, llm, validate, reply, meter` (renames only via `ctx.removed`).
- [ ] `generateObject` with `maxRetries: 0` and `abortSignal: AbortSignal.timeout(2200)` inside `ctx.step({ name: "llm", timeout: 2600, maxRetries: 0 })`; exactly one escalation attempt, bid path only, `claude-opus-5`, 1400 ms.
- [ ] Per-decision schema `z.object({ moveId: z.enum(ids), rationale: z.string().max(160), confidence: z.number().min(0).max(1) })`, `ids` from the table-supplied legal set.
- [ ] `modelParams(modelId)` allowlist: `claude-haiku-4-5` gets `temperature`; the Opus-5 family gets `{}` plus `thinking: { type: "disabled" }` — sending `temperature`/`top_p`/`top_k` there is an HTTP 400 the SDK does not strip.
- [ ] `budget.ts`: on exceed, `degraded = true` and heuristic-only for the rest of the match.
- [ ] Table side: `dispatch-ai.ts` never uses `wait: true` and always carries an `AbortSignal`; `onAiTimeout` at 4000 ms auto-plays `rankMoves()[0]`; between 2200 and 4000 ms the table broadcasts `thinking { extended: true }`.
- [ ] `metering.harness.ts` over ≥ 200 real decisions on `claude-haiku-4-5`.

**Exit criteria.**
1. A `MockLanguageModelV4` array drives a full deterministic hand including an illegal id, malformed JSON and a timeout, each exercising the correct rung — with no env var touched (§4.3).
2. The table independently re-derives `legalMoves()` and re-validates before applying; a deliberately version-skewed rules package causes rejection and a logged fallback to heuristic top-1.
3. `modelParams("claude-opus-5")` returns `{}`; a unit test asserts no sampling parameter is sent for every id in the no-sampling family.
4. `legal.length === 1` short-circuits with zero API calls and still honours the 250–450 ms floor.
5. Killing the process after the `llm` step commits replays the recorded result: exactly one `doGenerate` call, not two.
6. Measured p50/p95 latency and `usage.inputTokenDetails.cacheReadTokens`/`cacheWriteTokens` are recorded in `docs/LEDGER.md`. **The tempo floors, the 4000 ms cap and the token budget are frozen only after this number exists.**
7. Measured cost per 10-point game recorded against the ceiling of ~8 decisions/seat/hand = 24/hand ≈ 700–840 calls/game before the forced short-circuit.

---

### M5 — Personas, the three-layer prompt, and Settings · 4 d

**Deliverable.** `prompt.ts` with Layer 0 (immutable rules + output contract), Layer 1 (delimited untrusted persona,
2000-char cap, plus the 1000-char house prompt), Layer 2 (dense line notation with the engine's ranked top-3);
`difficulty` as a candidate-set narrower; persona snapshotting into game-create input; the Settings page writing a
`persona_versions` row on every save.

**Files.** `apps/game/src/actors/ai-seat/{prompt,decide}.ts`; `apps/game/src/actors/player-profile/{index,schema,drizzle.config}.ts` + `drizzle/` (personas + versions only at this milestone);
`apps/web/src/routes/(app)/settings/{+page.server.ts,+page.svelte}`; `apps/web/src/lib/client/settings.svelte.ts`;
`apps/web/src/routes/(app)/play/+page.server.ts` (server creates the game, snapshotting personas);
`apps/game/src/__tests__/adversarial-persona.spec.ts`.

**Tasks.**
- [ ] Three-layer prompt with one `cacheControl` breakpoint at the end of Layer 0+1 on the Opus path only.
- [ ] `difficulty`: expert = full legal set, casual = engine top-3, rookie = noise-injected ranking. Never touches legality, latency or token count.
- [ ] `/play` server action creates the game via `euchreTable.create(...)` with the persona snapshot; the browser never creates a game.
- [ ] Settings form action scoped by `locals.user.id`, capped and validated, writing `persona_versions`.
- [ ] Adversarial corpus: injection strings, rule-override attempts, "reveal your hand", "output raw text".

**Exit criteria.**
1. Editing a persona demonstrably changes play across two games with the same seed, recorded as a diffed decision log.
2. A Settings edit mid-game does not change the running opponents.
3. Difficulty behaves as specified at all three settings; illegality remains impossible at every setting.
4. The adversarial corpus produces zero illegal moves, zero schema deviations and zero card references — a required CI gate.
5. `cache-prefix.spec.ts` asserts the Opus cached prefix exceeds 4096 tokens even with an empty user persona, and that no `cacheControl` breakpoint is emitted on the Haiku path.

---

### M6 — The Threlte table: scene, atlas, picking, hand geometry · 8 d

**Deliverable.** The complete 3D layer and the build-time card atlas. The DOM game from M2 stays and remains fully
playable throughout.

**Files.** `apps/web/scripts/build-atlas.ts` + `apps/web/scripts/card-art/**`;
`apps/web/src/lib/three/{Stage,Rig,Lighting,Felt,Kitty,Card,HitProxy,PlayerHand,OpponentHand,TrickZone,TrickPile,DealerButton}.svelte`;
`apps/web/src/lib/three/{cardMotion.svelte.ts,director.svelte.ts,layout.ts,atlas.ts}`;
`apps/web/src/lib/ui/{Hud,TrumpBadge,TurnIndicator}.svelte`; `apps/web/src/routes/(app)/play/[gameId]/+page.svelte`.

**Tasks.**
- [ ] `build-atlas.ts`: `@resvg/resvg-js` → raster, `sharp` → one padded atlas PNG + `atlas.json`, for three art variants (four-colour, two-colour, large-index). Runtime `CanvasTexture` uploads are banned.
- [ ] Per-card four-vertex geometries with atlas UVs baked in, sharing one `MeshStandardMaterial` and one texture.
- [ ] `layout.ts` as a pure function, unit-tested independently of the renderer.
- [ ] Interaction: hit proxies with `MeshDiscardMaterial` + `meshBounds`, `interactivity({ filter: (h) => h.slice(0, 1) })`, two-stage tap plus drag with a 25 %-of-card-height snap.
- [ ] `cardMotion.svelte.ts`: a timeline class writing straight into `Object3D`s; zero `$state` writes per frame; exactly one `useTask`, with `autoInvalidate: false`.
- [ ] `director.svelte.ts` applies `sync { view, steps }`, reconciles on empty `steps`, supports snapshot-only mode.
- [ ] `NeutralToneMapping`, no shadow maps, `ContactShadows frames={0}` refreshed only when a card lands, `dpr` clamped to `[1, 1.5]` when `navigator.hardwareConcurrency <= 4`.

**Exit criteria.**
1. Faces use baked-UV geometries sharing one material and one texture; the ban list above holds.
2. `layout.ts` **proves** the affordance floor: a unit test over viewport widths 320–430 CSS px asserts every hand card's unoccluded strip is ≥ 48 px wide and ≥ 64 px tall at hand sizes 1–6.
3. A test asserts `context.interactiveObjects.length === handSize` — the raycast set is never the scene graph.
4. Peak draw calls at trick 5 ≤ 60, measured and recorded; resident GPU texture memory ≤ 13 MB, measured and recorded.
5. Sustained 60 fps on a Pixel 6a-class device through a full hand, with measured **zero** idle frame cost.
6. Deck-variant swap disposes the old atlas and loads the new one in < 300 ms, and is blocked mid-hand. Suit is never colour alone.
7. The bidding UI is `LegalMove[]`-driven end to end (`BidPanel` resolves "Order it up" / "I assist" / "I take it" by seat role; `SuitPicker` excludes `turnedDownSuit`; `alone` rides the call; stick-the-dealer removes Pass entirely).
8. The six-card discard fan renders and picks correctly; `discard:9C` is a plain card tap.

---

### M7 — Tempo, sound, haptics, and banter · 4 d

**Deliverable.** The client tempo table with the `{ brisk 0.7, table 1.0, slow 1.3 }` multiplier; `sound.ts` and
`haptics.ts`; the full banter path — `streamText` on public state, salience gate, table-side talk arbiter, leak
filter, phrasebook fallback; physical trick piles.

**Files.** `apps/web/src/lib/three/tempo.ts`, `apps/web/src/lib/a11y/{sound,haptics,motion}.ts`,
`apps/web/src/lib/ui/{TalkLog,Celebration}.svelte`; `apps/game/src/actors/ai-seat/{banter,phrasebook}.ts`;
`apps/game/src/actors/euchre-table/talk-arbiter.ts`; `packages/euchre-core/src/guard.ts` (finalised).

**Exit criteria.**
1. The client tempo table matches the phase table in `04-FRONTEND-UX.md` exactly, including the 1100 ms pause when a trick seals a euchre.
2. Snapshot-only rendering is a CI-exercised mode: drop every animation, apply only the sync view, and the table is still exactly right — just instant.
3. A client that skips every local delay still cannot observe the next card before `onTempoGate` fires.
4. Talk arbiter enforced and failing **closed**: ≤ 1 line/seat/trick, ≤ 2/trick table-wide, 2.5 s cooldown, forced on at loner/euchre/march/trump-called/9–9, plus a 400–900 ms partner reaction to *your* bid that bypasses the trick cap.
5. `chatDelta` batching at ~80 ms / ~40 chars keeps a three-seat storm ≥ 10× under the 1000-message queue cap.
6. Every rejected banter line emits a phrasebook line — never silence, never a regeneration — logged with the persona id.
7. Sound cues ≤ 400 ms for deal, card-place (pitch-varied), trick-take, trump-called, euchre (descending), march (ascending), game-won; haptics light/medium/success/error.
8. `prefers-reduced-motion` and the in-app override collapse flights to 0.001 s but **preserve** the 700–1000 ms trick read pause.

---

### M8 — `playerProfile`: history, replay, and episodic memory · 4 d *(parallel branch)*

**Deliverable.** The full `playerProfile` schema and committed migrations; `recordHand`/`recordMatch`; the
`{seed, moves[]}` replay re-derived **server-side**; `/games` and `/games/[matchId]` rendered through the same
director as live play; cross-game episodes with the "what they remember about you" panel and per-episode forget.

**Files.** `apps/game/src/actors/player-profile/{index,schema}.ts` + `drizzle/`;
`apps/game/src/actors/ai-seat/memory.ts` (episode emission); `apps/web/src/routes/(app)/games/{+page.server.ts,+page.svelte,[matchId]/+page.svelte}`;
`packages/euchre-core/src/replay.ts` (finalised).

**Exit criteria.**
1. A finished match appears in `/games` with the correct score and hand count; an **abandoned** match also appears, because hands are journalled at every hand boundary.
2. `/games/[matchId]` replays through the same director. All four hands are shown (the teaching feature); the three buried kitty cards and the dealer's discard are masked in every frame; the raw seed never reaches a client.
3. The redaction fuzz covers the replay path explicitly and fails if a kitty or discard card appears.
4. Replay storage ≤ 3 KB per game; an old game renders with any later rendering improvements automatically.
5. Episodes rank by `salience × exp(−age / 14 d)`, top-8, ≤ 400 tokens in the dossier; a grudge resurfaces as a needle line when the human declares alone again.
6. Forget removes the episode and the change is visible in the next game's dossier.
7. The nightly `rollup` cron recomputes lifetime stats and expires stale episodes.

---

### M9 — Onboarding, accessibility, and mobile · 5 d

**Deliverable.** The three onboarding modes; two separated `aria-live` regions; the labelled canvas; the full
keyboard model; the large-index art variant; safe-area and portrait HUD reflow.

**Files.** `apps/web/src/lib/ui/{Coach,RulesPanel,IllegalWhy,Tutorial,LiveRegions,HandA11y}.svelte`;
`apps/web/src/lib/a11y/announce.ts`; `apps/web/src/lib/three/{Stage.svelte,layout.ts}` (portrait branch);
`apps/web/src/app.html` (`viewport-fit=cover`); `apps/web/scripts/build-atlas.ts` (large-index variant).

**Exit criteria.**
1. Tapping an illegal card shows the explanation verbatim: *"Hearts were led. Your Jack of diamonds is a heart right now — you have to follow."*
2. Coach mode pulses the top-ranked legal move with a one-sentence rationale, is dismissible per hint, and auto-retires after 3 correct unassisted repetitions.
3. The first-run hand uses the fixed deal from `script.ts`, guaranteeing a left-bower moment in trick 1 and a march; a persistent "?" opens the rules panel context-first.
4. Keyboard: ←/→ traverse legal cards, Shift+←/→ reach illegal ones, ↑/Enter lift, Enter commit, Esc deselect, T trump, S score, L last trick; focus mirrors into the 3D hover state.
5. Two `aria-live` regions — polite game state, and a separate **default-off** banter region with a toggle. A screen-reader run confirms banter never interleaves with a trump call.
6. `<canvas role="img" aria-label=…>` carries a summary regenerated from the public view.
7. The large-index variant ships as an accessibility setting from the same build script; all HUD text is DOM and stays unclipped at 200 % OS text scale.
8. On a 390×844 notched portrait viewport: HUD respects `safe-area-inset` over a `100dvh` canvas, the reflow places score/trump/turn/talk without overlap, and an orientation change re-lays out without losing the in-flight hand.

---

### M10 — Hardening, soak, and deploy · 4 d

**Deliverable.** Every CI gate green, soak on a flaky mobile link, production deploy with a build-time
`envoy.version`, and `docs/LEDGER.md` completed.

**Files.** `.github/workflows/ci.yml` (final gate set), `apps/web/playwright.config.ts` + `apps/web/e2e/*.spec.ts`,
`docs/LEDGER.md`, deploy manifests, `.env.example` (final).

**Exit criteria.**
1. All required gates green (§6): typecheck, four lint rules, svelte-check, 10 000-game redaction fuzz, adversarial persona corpus, cache-prefix assertion, queue-consumer check, crash-window recovery suite, Better Auth footgun smoke test, Playwright smoke.
2. A 50-game soak on a throttled 3G/lossy link produces zero stuck cards, zero ghost cards, and a desync rate under 1 %.
3. `envoy.version` is a build-time constant from `RIVET_RUNNER_VERSION`; a deploy performed mid-hand does not drain the table and play continues.
4. Measured p95 decision latency, fallback rate (< 3 %) and cost per 10-point game are recorded in `docs/LEDGER.md` and within the M4-frozen budgets.
5. `docs/LEDGER.md` names every deletion with its price and its v2 upgrade path (§9).

---

## 4. Testing strategy, by layer

Five layers, five different tools, five different failure modes. Each layer's tests must be runnable alone.

### 4.1 Pure rules engine — Vitest unit + `fast-check` property tests

No mocks, no fakes, no I/O. `apply` is a pure function, so a test is a literal `GameState` in, a literal
`ApplyResult` out. Table-driven for the A8 scoring rows and the trick comparator; property-based for anything with
a combinatorial space (`effectiveSuit`, follow-suit legality, `applyOptimistic` agreement). Coverage on
`reduce.ts`, `legal.ts`, `score.ts` is gated at 100 % of branches — these three files decide what is legal, and an
uncovered branch there is an unenforced rule.

### 4.2 Actor layer — `rivetkit/test` `setupTest` with fake timers

`setupTest` gives in-memory drivers and `vi.useFakeTimers()`, so an entire 10-point match — schedules, tempo gates,
watchdogs and all — runs in milliseconds with no network.

```ts
// apps/game/src/__tests__/table-run-loop.spec.ts
import { setupTest } from "rivetkit/test";
import { expect, test, vi } from "vitest";
import { registry } from "../registry.js";
import { DEFAULT_CFG, SCRIPTED_PERSONAS, mintTestJwt } from "./fixtures.js";
import type { PublicGameView, Step } from "@euchre/core";

test("eldest orders up, dealer must discard, hand plays out", async (testCtx) => {
  const { client } = await setupTest(testCtx, registry);

  const table = client.euchreTable.getOrCreate(["table", "t1"], {
    createWithInput: {
      ownerUserId: "u1",
      seed: "fixture-order-up",
      cfg: DEFAULT_CFG,
      personas: SCRIPTED_PERSONAS,
    },
    getParams: async () => ({ token: await mintTestJwt("u1") }),
  });

  const conn = table.connect();
  const syncs: { v: number; view: PublicGameView; steps: Step[] }[] = [];
  conn.on("sync", (p: { v: number; view: PublicGameView; steps: Step[] }) => syncs.push(p));
  await conn.ready;

  await vi.advanceTimersByTimeAsync(4_000); // cut window + deal + AI bids ahead of the human
  const view = syncs.at(-1)!.view;
  expect(view.phase).toBe("bid_round_1");
  expect(view.turnSeat).toBe(0);
  expect(view.hand).toHaveLength(5);

  const res = await conn.submitMove("orderUp", "cm-1");
  expect(res.ok).toBe(true);

  await vi.advanceTimersByTimeAsync(4_000);
  const after = syncs.at(-1)!.view;
  expect(after.trump).not.toBeNull();
  expect(after.makerSeat).toBe(0);
  // V12: no step ever names the dealer's discard.
  expect(syncs.flatMap((s) => s.steps).some((s) => s.t === "dealerDiscarded")).toBe(true);
  expect(JSON.stringify(syncs)).not.toContain('"dealerDiscard"');
});
```

The recovery suite (`recovery.spec.ts`) is the same shape with an induced restart between each pair of adjacent
lifecycle points: mid-`apply`, after save but before fan-out, after fan-out but before `complete()`, and — the one
that actually stalls a game — after `complete()` but before the new `turnId` is minted and the schedule armed.
`auth.spec.ts` covers the forged internal publish, the `playerProfile` IDOR and the origin reject.

### 4.3 AI layer — `MockLanguageModelV4`, zero network

The model arrives through registry config → actor create input, so tests swap it without touching an env var. Any
code path constructing a provider from `process.env` inside `createVars` is a defect and this test is what catches it.

```ts
// apps/game/src/__tests__/ai-ladder.spec.ts
import { MockLanguageModelV4 } from "ai/test";
import { expect, test } from "vitest";
import { runDecisionLadder } from "../actors/ai-seat/decide.js";
import { legalFixture, viewFixture, rankingFixture } from "./fixtures.js";

const say = (text: string) => async () => ({
  content: [{ type: "text" as const, text }],
  finishReason: { unified: "stop" as const, raw: undefined },
  usage: {
    inputTokens: { total: 420, noCache: 420, cacheRead: 0, cacheWrite: 0 },
    outputTokens: { total: 24, text: 24, reasoning: 0 },
  },
  warnings: [],
});

test("illegal id, then malformed JSON, then timeout — each lands on the right rung", async () => {
  const model = new MockLanguageModelV4({
    doGenerate: [
      // 1: syntactically valid, semantically illegal — rejected by z.enum, falls back.
      say('{"moveId":"play:AS","rationale":"lead the ace","confidence":0.7}')(),
      // 2: unparseable — NoObjectGeneratedError, falls back.
      say("sure! here you go: play the nine")(),
      // 3: legal — accepted.
      say('{"moveId":"play:9C","rationale":"throw off","confidence":0.6}')(),
    ],
  });

  const legal = legalFixture(); // ["play:9C", "play:TC", "play:KH"]
  const a = await runDecisionLadder({ model, view: viewFixture(), legal, ranking: rankingFixture() });
  expect(a.source).toBe("fallback");
  expect(legal.some((m) => m.id === a.moveId)).toBe(true);

  const b = await runDecisionLadder({ model, view: viewFixture(), legal, ranking: rankingFixture() });
  expect(b.source).toBe("fallback");

  const c = await runDecisionLadder({ model, view: viewFixture(), legal, ranking: rankingFixture() });
  expect(c).toMatchObject({ source: "llm", moveId: "play:9C" });
});
```

The timeout rung is exercised with a `doGenerate` that awaits a never-resolving promise plus fake timers advanced
past 2200 ms; the escalation rung with a bid-kind request whose first call aborts. `adversarial-persona.spec.ts`
runs the injection corpus through the same entry point and asserts three zeroes: illegal moves, schema deviations,
card references. `cache-prefix.spec.ts` counts tokens in the assembled Layer 0+1 prefix.

### 4.4 End-to-end headless fuzz — the "no illegal state is reachable" harness

This is the single highest-value test in the project. It plays complete games through the real reducer, asserting
after **every** move that (a) the V1–V12 invariants hold and (b) no payload built for seat *s* contains a card that
seat *s* may not know. It is the same harness the binding decisions call the 10 000-game redaction fuzz; the
invariant assertions ride along for free because the state is already in hand.

```ts
// packages/euchre-core/src/__tests__/redaction.fuzz.spec.ts
import { expect, test } from "vitest";
import {
  advance, apply, checkInvariants, createGame, legalMoves, project, projectSteps,
  type CardId, type GameState, type Seat,
} from "../index.js";

const GAMES = Number(process.env.FUZZ_GAMES ?? 10_000);
const SEATS: readonly Seat[] = [0, 1, 2, 3];
const CARD_RE = /^[9TJQKA][SHDC]$/;

/** Structural walk: collect only exact CardId-shaped leaf strings. A raw substring scan of the
 *  serialized payload false-positives on user-authored persona names such as "AK". */
function cardsIn(value: unknown, out: Set<string> = new Set()): Set<string> {
  if (typeof value === "string") { if (CARD_RE.test(value)) out.add(value); return out; }
  if (Array.isArray(value)) { for (const v of value) cardsIn(v, out); return out; }
  if (value && typeof value === "object") { for (const v of Object.values(value)) cardsIn(v, out); }
  return out;
}

/** Everything seat `s` must never see. The up-card is subtracted: it is public by construction even
 *  after the dealer takes it into hand, and even when it is the card the dealer discards. */
function forbiddenFor(state: GameState, s: Seat): Set<CardId> {
  const h = state.hand;
  const forbidden = new Set<CardId>();
  for (const other of SEATS) if (other !== s) for (const c of h.hands[other]) forbidden.add(c);
  for (const c of h.kitty.slice(1)) forbidden.add(c);
  if (h.dealerDiscard) forbidden.add(h.dealerDiscard);
  if (h.upCard) forbidden.delete(h.upCard);
  return forbidden;
}

test(`${GAMES} games: no invariant violation and no redaction leak`, () => {
  for (let g = 0; g < GAMES; g++) {
    let state = createGame({ gameId: `fuzz-${g}`, ownerUserId: "u", seed: `fuzz-${g}` });
    let guard = 0;

    while (state.status === "active") {
      expect(guard++).toBeLessThan(4_000); // termination is itself an invariant
      const seat = state.hand.turnSeat;

      const out = seat === null
        ? advance(state)
        : (() => {
            const moves = legalMoves(state, seat);
            expect(moves.length).toBeGreaterThan(0);
            const pick = moves[(g * 31 + guard * 17) % moves.length]!;
            return apply(state, seat, pick.move);
          })();

      checkInvariants(out.state);

      for (const s of SEATS) {
        const wire = { view: project(out.state, s), steps: projectSteps(out.steps, s) };
        const leaked = [...cardsIn(wire)].filter((c) => forbiddenFor(out.state, s).has(c as CardId));
        expect(leaked, `seat ${s} leak in game ${g} move ${guard}`).toEqual([]);
      }
      state = out.state;
    }

    expect(state.status).toBe("complete");
    expect(Math.max(...state.score)).toBe(state.cfg.gameTo);
  }
});
```

Budget: 10 000 games at ≤ 12 ms each (four seats × two projections × a structural walk per move) is ~120 s wall on
four Vitest workers, which is the accepted PR-gate cost. Set `FUZZ_GAMES=250` locally for a fast loop; CI always
runs the full 10 000. The replay path is covered by the same harness parameterised over
`replay({ seed, moves })` frames instead of live `project()` output.

### 4.5 Browser smoke — Playwright

> **Spec note:** the stack list in `01-ARCHITECTURE.md` does not pin a browser test runner. Add
> `@playwright/test@1.62.0` as an `apps/web` devDependency, with `apps/web/playwright.config.ts` and
> `apps/web/e2e/`. It is a devDependency of one app, so it does not touch the runtime dependency graph or the
> `drizzle-orm` duplication that `.npmrc` protects.
>
> **Spec note:** Playwright needs a signed-in session without a production auth bypass. `email.ts` gains one
> branch: when `EMAIL_SINK_FILE` is set (never in production — CI asserts it is unset there), the magic-link URL is
> appended to that file instead of being sent. The E2E test reads the file. No code path skips verification.

```ts
// apps/web/playwright.config.ts
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 90_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : [["list"]],
  use: { baseURL: "http://localhost:4173", trace: "on-first-retry", video: "retain-on-failure" },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "mobile", use: { ...devices["Pixel 7"] } },
  ],
  webServer: [
    { command: "pnpm --filter @euchre/game start", port: 6420, reuseExistingServer: !process.env.CI },
    { command: "pnpm --filter @euchre/web preview --port 4173", port: 4173, reuseExistingServer: !process.env.CI },
  ],
});
```

```ts
// apps/web/e2e/smoke.spec.ts
import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";

const SINK = process.env.EMAIL_SINK_FILE ?? ".e2e/magic-links.txt";

test("sign in, get dealt a hand, play one legal card", async ({ page }) => {
  const email = `e2e-${Date.now()}@example.test`;

  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByRole("button", { name: "Send magic link" }).click();
  await expect(page.getByText("Check your email")).toBeVisible();

  await expect
    .poll(async () => (await readFile(SINK, "utf8")).includes(encodeURIComponent(email)), { timeout: 15_000 })
    .toBe(true);
  const url = (await readFile(SINK, "utf8")).trim().split("\n").at(-1)!;

  await page.goto(url);
  await page.getByRole("button", { name: "Continue" }).click(); // SafeLinks interstitial
  await expect(page).toHaveURL(/\/play/);

  // The accessible hand list is the stable selector — it exists at M2 and never goes away.
  const hand = page.getByRole("list", { name: "Your hand" }).getByRole("button");
  await expect(hand).toHaveCount(5, { timeout: 30_000 });

  const legal = hand.and(page.locator(":not([aria-disabled='true'])")).first();
  await legal.click(); // lift
  await legal.click(); // commit
  await expect(page.getByRole("status")).toContainText(/plays|trick/i);
});
```

Three specs, deliberately: `smoke.spec.ts` (above), `a11y.spec.ts` (keyboard-only traversal of a full hand plus an
axe scan of the HUD), and `snapshot-mode.spec.ts` (`?render=snapshot` disables animation; the same hand reaches an
identical final state). Playwright never asserts on 3D pixels — draw-call and frame budgets are measured by the
instrumented M6 harness, not by screenshots.

---

## 5. Local development loop

`pnpm dev` runs both processes. `apps/game` reloads on save via `bun --watch`; live actors are inspected at
`http://localhost:6420/inspector/{state,queue,connections,workflow-history,database/rows}`. `apps/web` is Vite on
`:5173` and connects to the gateway **directly** — SvelteKit is never on the realtime path. Without
`ANTHROPIC_API_KEY`, `config.ts` injects a scripted `MockLanguageModelV4`; without `RESEND_API_KEY`, magic links
print to the console. Neither absence blocks development.

---

## 6. CI

One workflow, `.github/workflows/ci.yml`, on every push and pull request. Node 24.11.0 for `apps/web` jobs, Bun
1.3.14 for `apps/game` jobs, pnpm 10.19.0 throughout, `--frozen-lockfile` always.

```yaml
jobs:
  static:      # typecheck · eslint (4 custom rules) · svelte-check · pnpm why drizzle-orm assertion
  unit:        # vitest --project core --project game --project web --project lint-rules + coverage thresholds
  fuzz:        # FUZZ_GAMES=10000 vitest --project core -t "no invariant violation"  (maxWorkers 4)
  adversarial: # vitest --project game -t "adversarial|cache-prefix|queue-consumers|recovery|auth"
  e2e:         # playwright install --with-deps chromium && pnpm --filter @euchre/web test:e2e
```

`static`, `unit`, `fuzz` and `adversarial` are required for merge from M1, M2, M1 and M5 respectively; `e2e`
becomes required at M6. Jobs share a pnpm store cache keyed on `pnpm-lock.yaml` and a Playwright browser cache
keyed on the `@playwright/test` version. The `static` job also greps `apps/web/src/lib/server/auth.ts` for
`sveltekitCookies` as the final array element and `hooks.server.ts` for `svelteKitHandler` — the two Better Auth
footguns that fail silently at runtime. Nothing is a warning: every gate is pass/fail.

---

## 7. Definition of done (whole project)

The project is done when all of the following are simultaneously true on `main`:

1. **Play.** A signed-in user completes a 10-point match against three LLM-driven opponents, in a browser, on both a desktop and a 390×844 phone, including at least one loner, one euchre, one march and one stick-the-dealer, with no manual intervention.
2. **Correctness.** Every M1 exit criterion holds, and the 10 000-game fuzz reports zero invariant violations and zero leaks — including through the replay path.
3. **Secrecy.** No foreign hand, buried kitty card or dealer discard is representable in any type that crosses the wire, live or in replay. The forged-publish, IDOR and origin tests pass, and the forged-publish test demonstrably fails when the `internalToken` check is removed.
4. **Liveness.** No sequence of crashes, disconnects, LLM hangs or deploys leaves a match unable to advance. The reconciler test covers all four crash windows; the 4000 ms watchdog terminates any AI turn; `onAbandon` terminates any human absence.
5. **Cost and latency.** Measured p95 decision latency, fallback rate < 3 %, and cost per 10-point game are recorded in `docs/LEDGER.md` and inside the frozen M4 budgets.
6. **Performance.** ≤ 60 draw calls at peak, ≤ 13 MB texture memory, sustained 60 fps on a Pixel 6a-class device, zero idle frame cost.
7. **Accessibility.** The game is completable keyboard-only and screen-reader-only via the `HandA11y` list, with two separated live regions and a 200 %-text-scale-clean HUD.
8. **Teachability.** A player who has never played euchre completes the scripted first hand and can then finish a match with coach mode on.
9. **CI.** Every gate in §6 is required and green; `pnpm -r typecheck`, `lint`, `check` and `test` pass on a clean checkout with only `pnpm install`.
10. **Documentation.** `docs/SPEC.md`, `docs/RULES.md` and `docs/LEDGER.md` are current, and `docs/LEDGER.md` prices every deletion in §9.

---

## 8. Risk register

| # | Risk | Impact | Early warning | Mitigation |
|---|---|---|---|---|
| R1 | **Hidden-information leak** via a new payload, a new step, or paraphrased banter ("I've got the big one"). | Product-fatal: it stops being euchre. | The fuzz gate fails; the banter rejection log passes ~1 %. | Structural redaction, `no-broadcast-state`, the fuzz gate, public-state-only banter plus `screenBanter`, and a pre-launch review of logged utterances. Above ~1 % soft leaks, flip banter default to off — the game is complete without it. |
| R2 | **Stalled match** — crash in the ack-then-arm window, hung LLM, dead `aiSeat`. | The match is bricked and nothing errored. | A match `active` with no armed schedule; `onAbandon` firing on a live user. | The tested `onWake` reconciler; the unconditional 4000 ms `onAiTimeout`; `wait: true` between actors forbidden; every cross-actor send carries an `AbortSignal`. |
| R3 | **`drizzle-orm` collision** (rivetkit ^0.44 vs apps/web 0.45.2) resolved away by hoisting or a rivetkit bump. | Better Auth's adapter or actor SQLite breaks at runtime, not build. | The `pnpm why` assertion fails. | `node-linker=isolated`, committed lockfile, and the assertion as a required gate rather than a comment. |
| R4 | **LLM cost or latency overrun.** | Unshippable economics, or laggy play. | M4 metering numbers. | Nothing frozen before measured (M4 exit 6); forced short-circuit at `legal.length === 1`; `difficulty` adds no tokens; per-game breaker degrades a seat to heuristic-only. |
| R5 | **Vite 8 / Threlte incompatibility** (`vite-plugin-svelte#1313`, `THREE.Box3 undefined`). | The 3D layer breaks at runtime with an opaque error. | Threlte smoke failure right after a dependency bump. | `vite@7.3.6` and `@sveltejs/vite-plugin-svelte@7.2.0` pinned below latest; plain `vitePreprocess()`; the scene mounts in the Playwright smoke so a bump cannot land silently. |
| R6 | **M6 overruns** — atlas pipeline, picking and portrait layout are the usual sinks. | The largest milestone slips the release. | Day 5 of M6 with picking still wrong. | The M2 DOM game is already shippable. Cut order: portrait polish → deck swap → ContactShadows → the 3D layer entirely. This is why the a11y list is milestone zero of the UI. |
| R7 | **Prompt injection via the persona field** (untrusted by design). | An opponent that cheats, leaks or emits malformed output. | The adversarial gate fails. | Per-decision `z.enum` over the legal set; the table re-derives and re-validates; hard-delimited 2000-char Layer 1; the corpus is a CI gate, not a review. |
| R8 | **Better Auth silent misconfiguration** — plugin order, `resolve(event)`, `$env/static/private`, 300 s default expiry. | Sign-in "succeeds" and the user stays logged out. | The footgun grep in `static`. | All four asserted mechanically in CI; `expiresIn: 900` set explicitly. |
| R9 | **Deploy drains a live hand.** | A match dies mid-trick on every deploy. | `envoy.version` differing across restarts of one build. | Build-time `RIVET_RUNNER_VERSION`, runner mode with `drainOnVersionUpgrade: false`, and an M10 criterion that deploys mid-hand. |
| R10 | **Actor state bloat** — `c.state` is loaded whole on every wake. | Wake latency grows until turns visibly lag. | `meter()` reporting a growing serialized state size. | Explicit caps on every collection (`trickLog` 5, `bids` 8, `recentMoveIds` 32, `trumpSeen` 7, `episodeBuffer` 12); history in `c.db`; `consumer-or-delete` against the 1000-message cap. |

---

## 9. Deliberately not in v1

Each of these is a purchase, not an oversight. `docs/LEDGER.md` records the price and the upgrade path for every
line; the summary is here so the boundary is visible from the plan.

- **A second human seat.** No matchmaker, no lobby, no seat negotiation. Price: single-player-versus-AI only. Path: a `matchmaker` actor plus a `seats` map keyed by `userId` — the table already stamps the acting seat from `c.conn.state`, so the authority model is unchanged.
- **A human turn timer.** Deleted on purpose, and written down so it is not re-added: one human, nobody waiting. Price: an idle game waits 90 s for a nudge and 240 s for auto-play. Path: none wanted.
- **Post-commit undo.** The lift *is* the undo. Price: a misclick past commit is final. Path: none — undo would desynchronise the AI actors and teach hesitancy.
- **`DataArrayTexture` / KTX2 / instanced cards.** Price: mip bleed is prevented by padding rather than by construction, and GPU memory is ~13 MB instead of ~4 MB. Path: `atlas.ts` isolates UV baking; a layer index is a change to that file and the material.
- **Server-authored beat script.** Only `trick_resolve`, `hand_score` and the cut window are server-held. Price: no cross-client cinematic choreography. Path: a `Step` subtype carrying an explicit beat, consumed by `director.svelte.ts`.
- **Rule variants `defendAlone`, `farmersHand`, `railroading`, `superEuchre`, `dealerMustHoldTrump`.** Engine flags exist, default off, absent from the UI. Price: no house rules. Path: a Settings row plus the reducer branch each implies.
- **Configurable `gameTo` and `stickTheDealer`.** Both implemented and tested — `stickTheDealer: false` is what exercises the throw-in and the deal-moves-left misdeal — neither exposed. Price: 10 points and stick-the-dealer for everyone. Path: a Settings row.
- **Cross-user analytics.** History lives in per-user actor SQLite. Price: no leaderboard or aggregate stats without an export job. Path: a nightly export from the `rollup` cron into the existing Postgres.
- **Voice / TTS.** The canonical lines exist as `chat` events with `kind: "call"`; nothing speaks them. Price: silence beyond the sound cues. Path: `TalkLog` is already the single source of spoken text.
- **Multi-provider LLM support.** Anthropic only. Price: an outage degrades every seat to heuristic play — a working game, by design. Path: the model is injected at registry level; switching touches `config.ts` and the `modelParams()` allowlist, and invalidates only the cache-prefix test.
