# 01 — System Architecture

This document specifies the runtime topology, every actor, the realtime protocol, the
hidden-information redaction boundary, connection authentication, durability, failure recovery,
scaling, and deployment. It is binding on implementation.

Companions: `00-OVERVIEW.md` (product scope), `02-GAME-RULES-ENGINE.md` (the pure reducer this
document treats as a black box), `03-AI-AGENTS.md` (the decision ladder inside `aiSeat`),
`04-FRONTEND-UX.md` (Threlte layer and interaction), `05-IMPLEMENTATION-PLAN.md` (milestones M0–M10).

Euchre, for the reader who has never played it: four players in two fixed partnerships sit at a
table, partners opposite each other. A 24-card deck (9 T J Q K A in four suits) is dealt five to a
player; four cards are left over, the top one turned face up. Players bid to name a *trump* suit,
one suit that outranks the other three for the hand. Five tricks are then played; the trick-taking
team scores 1, 2, or 4 points; first team to 10 wins. Two rules drive the entire architecture: the
Jack of the same colour as trump *becomes* a trump card (so "which suit is this card" is a
computed property, never a stored one), and **each player's five cards are secret**. Secrecy is
what makes this a distributed-systems problem rather than a UI problem.

---

## 1. Topology

Two long-lived server processes plus the browser. SvelteKit (`apps/web`) owns identity, pages,
and form actions. The RivetKit registry (`apps/game`) owns all game state and all realtime
traffic. **The browser's WebSocket goes to the Rivet gateway directly; SvelteKit is never on the
realtime path** and therefore never has to proxy a WebSocket `Upgrade` (which `adapter-node` does
not do cleanly).

```
┌───────────────────────────────── BROWSER ──────────────────────────────────────┐
│  SvelteKit app, hydrated                                                       │
│  ┌──────────────────────────┐        ┌──────────────────────────────────────┐  │
│  │ Threlte / three.js scene │        │ rivetkit/client                      │  │
│  │ director.svelte.ts       │◄───────┤ createClient<typeof registry>(...)   │  │
│  │ HandA11y DOM fallback    │───────►│ conn.on('sync'|'thinking'|'chat'|…)  │  │
│  └──────────────────────────┘        │ conn.submitMove(moveId, clientMoveId)│  │
│                                      └──────────────┬───────────────────────┘  │
└──────────┬──────────────────────────────────────────┼──────────────────────────┘
   HTTPS   │  page loads · form actions                │  WSS  realtime, direct
  (cookie) │  POST /api/rivet-token                    │  (15-min JWT in conn params)
           ▼                                           ▼
┌──────────────────────────────┐          ┌───────────────────────────────────────────┐
│ apps/web · node 24.11.0      │  JWKS    │ apps/game · bun 1.3.14                    │
│ adapter-node                 │◄─────────┤ registry.start() · gateway :6420          │
│ better-auth 1.6.25           │  GET     │                                           │
│  magicLink + jwt +           │ /api/    │  ┌─────────────────────────────────────┐  │
│  sveltekitCookies (LAST)     │ auth/    │  │ euchreTable  ["table", gameId]      │  │
│  svelteKitHandler in hooks   │ jwks     │  │  serialized run loop                │  │
└───┬──────────────────┬───────┘          │  │  c.state = GameState (whole match)  │  │
    │ resend@6.18.0    │ postgres@3.4.9   │  │  c.db     = per-hand journal        │  │
    ▼                  ▼                  │  └──┬──────────────┬───────────────────┘  │
┌─────────┐   ┌──────────────────────┐    │     │ decide       │ recordHand           │
│ Resend  │   │ Neon Postgres        │    │     │ aiSay        │ recordMatch          │
│ magic   │   │ better-auth's 5      │    │     ▼              ▼                      │
│ links   │   │ tables + jwks ONLY   │    │  ┌────────────────┐  ┌──────────────────┐ │
└─────────┘   │ (no game data)       │    │  │ aiSeat × 3     │  │ playerProfile    │ │
              └──────────────────────┘    │  │ ["table",gid,  │  │ ["user", userId] │ │
                                          │  │  "seat","1|2|3"]│ │ c.db SQLite      │ │
                                          │  │ workflow()      │ │ history·personas │ │
                                          │  └────────┬────────┘  └──────────────────┘ │
                                          └───────────┼────────────────────────────────┘
                                                      │ HTTPS · ai@7.0.37
                                                      ▼
                                          ┌──────────────────────────────┐
                                          │ Anthropic API                │
                                          │ claude-haiku-4-5  (plays)    │
                                          │ claude-opus-5     (bids)     │
                                          └──────────────────────────────┘
```

Ports: `apps/web` on 5173 (dev) / 3000 (prod); Rivet gateway on 6420. `apps/game` holds **no**
database credentials, **no** Better Auth secret, and **no** user records — it verifies identity
statelessly against the public JWKS. `apps/web` holds **no** LLM key and never touches game state.
The two blast radii do not overlap.

---

## 2. The registry

```ts
// apps/game/src/registry.ts
import { setup } from "rivetkit";
import { euchreTable } from "./actors/euchre-table/index.js";
import { aiSeat } from "./actors/ai-seat/index.js";
import { playerProfile } from "./actors/player-profile/index.js";
import { BUILD_VERSION } from "./config.js";

export const registry = setup({
  use: { euchreTable, aiSeat, playerProfile },
  endpoint: process.env.RIVET_ENDPOINT,
  token: process.env.RIVET_TOKEN,
  namespace: process.env.RIVET_NAMESPACE ?? "development",
  maxIncomingMessageSize: 1_048_576,
  envoy: { version: BUILD_VERSION, poolName: "default" },
});
```

```ts
// apps/game/src/config.ts  — the one file a provider swap touches
import { createAnthropic } from "@ai-sdk/anthropic";
import type { LanguageModel } from "ai";

/** envoy.version is typed `number` in rivetkit 2.3.9 — an integer, not a semver string. */
export const BUILD_VERSION: number = Number.parseInt(
  process.env.RIVET_RUNNER_VERSION ?? "1",
  10,
);

export const CONFIG = {
  appUrl: process.env.APP_URL ?? "http://localhost:5173",
  allowedOrigins: (process.env.ALLOWED_ORIGINS ?? "http://localhost:5173").split(","),
} as const;

export type ModelFactory = (modelId: string) => LanguageModel;

const anthropic = createAnthropic({ apiKey: process.env.ANTHROPIC_API_KEY });
export const defaultModelFactory: ModelFactory = (modelId) => anthropic(modelId);
```

> **Spec note:** the synthesis called `envoy.version` a "build-time constant" without a type. The
> installed `rivetkit@2.3.9` schema declares it `z.number().optional()`. `RIVET_RUNNER_VERSION`
> must therefore be an integer that increases per deploy (CI uses the build epoch seconds). A
> string, or `Date.now()` evaluated at process start, drains and reschedules every actor on every
> restart.

`registry.start()` is the only boot path (`apps/game/src/index.ts` is three lines). It boots the
local engine plus the envoy and owns SIGTERM. It is deliberately **not** mounted inside SvelteKit:
Vite HMR would re-evaluate the registry module and thrash actor versions, and `envoy.version` must
be stable across a dev session.

---

## 3. Actors

Three actor kinds. There is no lobby actor and no matchmaker: every key is sharded by `gameId` or
`userId`, so there is no global hot spot and nothing to scale as a singleton.

### 3.1 Storage policy — what lives where, and why

| Store | Persisted | Loaded | Used for | Why not elsewhere |
|---|---|---|---|---|
| `c.state` | yes (auto at action end, `stateSaveInterval` 10 s, or `saveState({immediate:true})`) | **whole, on every wake** | the live match: deck order, four hands, kitty, bidding, current trick, score, turn nonce | It must be small and bounded. Every collection in it has an explicit cap (`trickLog` ≤ 5, `bids` ≤ 8, `recentMoveIds` ≤ 32) precisely because it is deserialized in full on each wake. |
| `c.vars` | no | rebuilt per wake in `createVars` | seeded RNG closure, cached remote JWKS, `LanguageModel` instances | Non-serializable. The *seed* lives in `c.state` and the *closure* is rebuilt from it — that split is the reason a restart reproduces the same shuffle. |
| `c.db` (SQLite + Drizzle) | yes | queried, never loaded whole | per-hand journal, match history, persona versions, episodes | 30–40 hands of `{seed, moves[]}` in `c.state` would bloat every single wake. In SQLite it is paginated, queryable, and capped at 10 GiB/actor. Migrations are generated by `drizzle-kit` and **committed**. |
| `c.kv` | — | — | **not used anywhere** | Deprecated in 2.3.9; the type definition says so verbatim. Any use is a review reject. |

### 3.2 `euchreTable` — key `["table", gameId]`

The single authority on one 10-point match. It owns the deck order, all four hands, the kitty, the
cut, both bidding rounds, the dealer's discard, going alone, follow-suit legality, trick
resolution, the server-held read pauses, scoring, turn sequencing, AI dispatch, the talk arbiter,
and per-connection redaction. It contains **zero euchre logic** — every mutation is exactly one
call into the pure `@euchre/core` reducer, so the actor file is plumbing. It holds no LLM client,
no API key, no user record, and no cross-match data.

State is `GameState` from `packages/euchre-core/src/types.ts`, held whole (the full shape is in
`02-GAME-RULES-ENGINE.md`; the fields the *architecture* depends on are these):

```ts
// the transport- and durability-bearing subset of GameState
interface GameStateRuntimeFields {
  readonly schema: 1;
  v: number;                    // monotonic version; the reducer never touches it, the actor bumps it
  readonly internalToken: string; // minted in createState; the actor↔actor boundary
  seed: string;                 // createVars rebuilds the RNG closure from this
  turnId: string;               // server turn nonce, regenerated on every advance
  turnDeadlineAt: number | null;
  timerId: string | null;       // schedule id: nudge
  watchdogId: string | null;    // schedule id: onAiTimeout
  revealId: string | null;      // schedule id: releaseAiMove
  pending: {                    // an early AI decision parked behind the persisted pacing floor
    readonly seat: Seat; readonly turnId: string;
    readonly requestedAt: number; readonly revealAt: number;
    decision: LegalMoveId | null; source: AIDecisionSource | null;
  } | null;
  recentMoveIds: string[];      // ring buffer, cap 32 — client double-click dedupe
  appliedSeq: number;           // server idempotency; same c.state as the mutation it guards
  status: "active" | "complete" | "abandoned";
}
```

`c.db` holds the append-only per-hand journal: `(handNo, seed, dealerSeat, deckOrder, moves[],
trump, makerSeat, aloneSeat, tricksWon, result, delta, aiRationales, talkLines)`, plus one
`match_meta` row written in `onCreate` containing the three snapshotted `PersonaConfig`s.

> **Spec note:** the binding actor list does not put personas in `c.state`, but `dispatchAi` needs
> them to reconstruct `createWithInput` if an `aiSeat` actor was destroyed or never created (a
> crash between `createState` and `onCreate`). Storing them in the `match_meta` row resolves this
> without bloating the whole-loaded `c.state` and keeps the "personas are snapshotted at create and
> never re-read" invariant intact — the row is written once and only read on the cold path.

**Actions.** Every action is read-only with exactly one exception.

| Action | Signature | Notes |
|---|---|---|
| `snapshot` | `(c) => PublicGameView` | Redacted from `c.conn.state.seat`. The reconnect resync path. |
| `lastTrick` | `(c) => Trick \| null` | Powers the last-trick viewer. |
| `submitMove` | `(c, moveId: LegalMoveId, clientMoveId: string) => Promise<{ ok: true; v: number }>` | **The only connection-reachable mutation.** Mutates nothing itself. |
| `meter` | `(c) => TableMeter` | Dev/inspector only. |
| `onTurnOpen` | `(c, turnId: string) => Promise<void>` | Schedule target. Enqueue-only. |
| `onNudge` | `(c, turnId: string) => Promise<void>` | Schedule target. Enqueue-only. |
| `onAbandon` | `(c, turnId: string) => Promise<void>` | Schedule target. Enqueue-only. |
| `onAiTimeout` | `(c, turnId: string, seat: Seat) => Promise<void>` | Schedule target. Enqueue-only. |
| `releaseAiMove` | `(c, turnId: string) => Promise<void>` | Schedule target. Enqueue-only. |
| `onTempoGate` | `(c, turnId: string) => Promise<void>` | Schedule target. Enqueue-only. |
| `flushProfile` | `(c) => Promise<void>` | Schedule target. Retry of the `playerProfile` write. |

Schedule targets fire **actions**, not queues; every one of them does nothing but `c.queue.send(…)`
after a stale-nonce check. The schedule → action → queue bridge is the only legal path from a timer
into the mutation loop.

```ts
// apps/game/src/actors/euchre-table/index.ts (excerpt)
onTempoGate: async (c, turnId: string): Promise<void> => {
  if (c.state.turnId !== turnId) return;                       // stale timer, drop
  await c.queue.send("tick", {
    kind: "tempo", turnId, seq: c.state.appliedSeq + 1,
    internalToken: c.state.internalToken,
  });
},
```

**Queues.** `canPublish` is typed by supplying the third generic of `queue<TMessage, TComplete,
TContext>`; this gives a typed `c.conn` without importing the actor's own definition (which would
be circular).

```ts
// apps/game/src/actors/euchre-table/queues.ts
import { queue } from "rivetkit";
import { HUMAN_SEAT, type LegalMove, type LegalMoveId, type RuleCode, type Seat } from "@euchre/core";

export interface TableConnState {
  readonly userId: string;
  readonly seat: Seat;
  readonly role: "player";
  readonly since: number;
}
/** The shape a canPublish / canSubscribe hook actually receives. */
export type Guard = { readonly conn?: { readonly state: TableConnState } };

export interface MoveMsg {
  readonly turnId: string;
  readonly seq: number;
  readonly moveId: LegalMoveId;
  readonly by: { readonly kind: "human"; readonly clientMoveId: string };
  readonly source: "human";
}
export type MoveAck =
  | { readonly ok: true; readonly v: number }
  | { readonly ok: false; readonly code: RuleCode; readonly legal: readonly LegalMove[] };

export interface InternalMsg { readonly turnId: string; readonly seq: number; readonly internalToken: string; }
export interface AiDecisionMsg extends InternalMsg { readonly seat: Seat; readonly moveId: LegalMoveId; readonly source: "llm" | "fallback" | "forced"; readonly rationale: string; }
export interface AiSayMsg extends InternalMsg { readonly seat: Seat; readonly msgId: string; readonly delta?: string; readonly text?: string; readonly final: boolean; }
export interface TickMsg extends InternalMsg { readonly kind: "tempo" | "releaseAi" | "aiTimeout" | "nudge" | "abandon" | "turnOpen"; }
export interface StartHandMsg extends InternalMsg { readonly handNo: number; }

const externalDenied = (c: Guard): boolean => { if (c.conn === undefined) return true; return false; };

export const tableQueues = {
  /** THE single human mutation path. */
  move: queue<MoveMsg, MoveAck, Guard>({
    canPublish: (c) => {
      if (c.conn?.state.role === "player" && c.conn.state.seat === HUMAN_SEAT) return true;
      return false;
    },
  }),
  aiDecision: queue<AiDecisionMsg, undefined, Guard>({ canPublish: externalDenied }),
  aiSay:      queue<AiSayMsg,      undefined, Guard>({ canPublish: externalDenied }),
  tick:       queue<TickMsg,       undefined, Guard>({ canPublish: externalDenied }),
  startHand:  queue<StartHandMsg,  undefined, Guard>({ canPublish: externalDenied }),
};
```

**`canPublish` is not the boundary, and the spec says so out loud.** `canPublish` runs on inbound
publish per `handle.send(...)`. A browser using the *stateless HTTP handle* has no `c.conn`, so
`c.conn === undefined` is `true` for a forged browser publish and `externalDenied` **fails open**.
It blocks a WebSocket-connected browser and nothing else. The actual boundary on every actor→actor
queue is the unguessable `internalToken` minted in `createState`, delivered to each `aiSeat` as
create input, and constant-time-compared inside the run loop **before any state read**:

```ts
import { timingSafeEqual } from "node:crypto";

function tokenOk(expected: string, got: unknown): boolean {
  if (typeof got !== "string") return false;
  const a = Buffer.from(expected, "utf8");
  const b = Buffer.from(got, "utf8");
  if (a.length !== b.length) return false;
  return timingSafeEqual(a, b);
}
```

`apps/game/src/__tests__/auth.spec.ts` asserts that deleting this check makes a forged-publish test
*pass* — proving the token, not the guard, is the boundary.

**Events.**

```ts
// apps/game/src/actors/euchre-table/events.ts
import { event } from "rivetkit";
import type { PublicGameView, Seat, Step } from "@euchre/core";
import type { Guard } from "./queues.js";

const playersOnly = (c: Guard): boolean => { if (c.conn?.state.role === "player") return true; return false; };

export const tableEvents = {
  sync:      event<{ v: number; view: PublicGameView; steps: readonly Step[] }, Guard>({ canSubscribe: playersOnly }),
  thinking:  event<{ seat: Seat; on: boolean; extended: boolean }>(),
  chat:      event<{ msgId: string; seat: Seat; kind: "call" | "banter" | "system"; text: string; final: true }>(),
  chatDelta: event<{ msgId: string; seat: Seat; delta: string }>(),
  presence:  event<{ seat: Seat; online: boolean }>(),
};
```

**Schedules.** All armed via `c.schedule.after(ms, actionName, ...args)`, which returns an id
stored in `c.state` so it can be cancelled on turn advance. Schedules persist across restart,
upgrade, and crash, and are re-armed on wake by the runtime.

| Schedule | Delay | Purpose |
|---|---|---|
| `onTurnOpen(turnId)` | end of a server-held pause | the next actor may act |
| `onTempoGate(turnId)` | 700–1000 ms (1100 ms if the trick seals a euchre); 1400–2400 ms at `hand_score` | the information-bearing read pause |
| `onAiTimeout(turnId, seat)` | 4000 ms from dispatch | hard cap; enqueues an auto-play tick |
| `releaseAiMove(turnId)` | until `pending.revealAt` | releases a parked early AI decision |
| `onNudge(turnId)` | 90 s | partner speaks a nudge line; no state change |
| `onAbandon(turnId)` | 240 s | auto-plays `rankMoves()[0]` so a solo game terminates |
| `flushProfile()` | 30 s | retry of the `playerProfile.recordMatch` write |
| self-reap | 10 min after `game_over` | `c.destroy()`, after the durable copy exists |

### 3.3 `aiSeat` — key `["table", gameId, "seat", "1" | "2" | "3"]`

One actor per AI opponent — three per match, never one per request and never one per decision. It
owns exactly one persona (snapshotted at game creation, never re-read mid-game), its own private
hand knowledge, its per-hand card-counting memory, its own `LanguageModel` instances, its own token
budget and circuit breaker, its own latency meter, and **its own failure domain**: one seat's LLM
outage degrades one seat.

`c.state` holds `{ gameId, seat, teamId, persona, memory, budget, meter, lastTurnId, episodeBuffer }`
— small, JSON, and every map and array explicitly capped (`trumpSeen` ≤ 7, `leadHistory` ≤ 5,
`bidHistory` ≤ 8, `humanRead` ≤ 8, `episodeBuffer` ≤ 12). It survives sleep so card counting is not
lost on a mid-hand restart. `c.vars` holds the non-serializable model instances, built in
`createVars` from a factory injected at registry level. There is no `c.db`: everything worth keeping
is journalled by the table and mirrored into `playerProfile` at hand and match end.

`run` is a **workflow**, so the replay history *is* the durable record of each decision's steps — a
crash after the `llm` step commits replays it for free instead of re-billing the API call.

```ts
// apps/game/src/actors/ai-seat/index.ts (structure; the ladder lives in 03-AI-AGENTS.md)
import { actor, queue } from "rivetkit";
import { Loop, workflow } from "rivetkit/workflow";
import type { AIDecideRequest, AIDecision } from "@euchre/core";

export const aiSeat = actor({
  state: {} as AiSeatState,
  createState: (_c, input: AiSeatInput): AiSeatState => initialAiState(input),
  createVars: (c, driverCtx: { modelFactory?: ModelFactory }): AiSeatVars => ({
    play: (driverCtx.modelFactory ?? defaultModelFactory)(c.state.persona.modelId),
    bid:  (driverCtx.modelFactory ?? defaultModelFactory)(c.state.persona.bidModelId),
  }),
  queues: {
    decide:   queue<AIDecideRequest, undefined, Guard>({ canPublish: externalDenied }),
    resetHand: queue<InternalMsg,    undefined, Guard>({ canPublish: externalDenied }),
    handEnd:   queue<InternalMsg,    undefined, Guard>({ canPublish: externalDenied }),
    gameEnd:   queue<InternalMsg,    undefined, Guard>({ canPublish: externalDenied }),
  },
  actions: {
    // EXPLICIT return annotations are mandatory — see §5.
    getPersona:     async (c): Promise<PersonaView>       => toPersonaView(c.state.persona),
    getStatus:      async (c): Promise<AiStatus>          => ({ ...c.state.meter, degraded: c.state.budget.degraded }),
    getDecisionLog: async (c): Promise<DecisionRecord[]>  => c.state.log.slice(-40),
  },
  run: workflow(async (ctx) => {
    await ctx.loop("turns", async (loop) => {
      const m = await loop.queue.next("await-work", {
        names: ["decide", "resetHand", "handEnd", "gameEnd"] as const,
      });
      if (m.name === "gameEnd") { await loop.step("meter", async (s) => flushMeter(s)); return Loop.break(undefined); }
      if (m.name === "resetHand") { await loop.step("seen", async (s) => resetMemory(s)); return; }
      if (m.name === "handEnd")   { await loop.step("meter", async (s) => flushEpisodes(s)); return; }
      const decision: AIDecision = await decideLadder(loop, m.body as AIDecideRequest);
      await loop.step("reply", async (s) => {
        const table = s.client<typeof registry>().euchreTable.getOrCreate(["table", s.state.gameId]);
        await table.send("aiDecision", decision, { signal: AbortSignal.timeout(2_000) });
      });
    });
  }),
});
```

The six step names — `'seen' | 'rank' | 'llm' | 'validate' | 'reply' | 'meter'` — are a **public
interface**. Renaming one diverges every in-flight workflow replay. Renames go through
`ctx.removed(name, "step")`; the `protected-workflow-step-names` ESLint rule enforces it.

> **Spec note:** in `rivetkit@2.3.9`'s workflow API, actor data (`state`, `vars`, `db`, `client()`,
> `broadcast`) is reachable **only** inside a `ctx.step(...)` callback — the `WorkflowContext` class
> deliberately does not expose them. Orchestration code between steps is pure control flow. Every
> code path above respects this; a violation throws at runtime, not at compile time.

### 3.4 `playerProfile` — key `["user", userId]`

The user's durable identity inside the actor system, and the per-user coordinator. `c.state` is
deliberately tiny — `{ schema: 1, activeGameId, dailyTokens, lifetime }` — because everything
paginated or queried lives in `c.db`: `matches`, `hand_journal`, `personas`, `persona_versions`,
`episodes`, `phrasebook`. `activeGameId` is how `/play` resumes an interrupted match.

SQLite rather than `c.state` because history is queried, not loaded whole. SQLite rather than
Postgres because the web app reads it *through the actor* and it never joins to auth rows; the
honest cost is that cross-user analytics needs an export job, recorded in `docs/LEDGER.md`.

Actions: `listMatches(cursor)`, `getMatch(id)`, `getReplay(id)`, `getSettings()`,
`recentEpisodes(role, limit)`, `activeGame()`, `tokenBudget()`, `rollup()`. Queues:
`recordMatch`, `recordHand`, `savePersona`, `remember`, `forget`, `noteTokens`. One cron:

```ts
onWake: async (c): Promise<void> => {
  await c.cron.every({ name: "rollup", interval: 86_400_000, action: "rollup" });
},
```

`getReplay` re-derives frames **server-side** through `@euchre/core`. The raw seed never reaches a
client, and the buried three kitty cards and the dealer's discard stay masked in every frame.

---

## 4. Lifecycle of one match

```
CREATE   SvelteKit /play/+page.server.ts (trusted, server-side — the browser NEVER creates a game)
         profile.activeGame() → resume, or:
         client.euchreTable.create(["table", gameId], { input: { ownerUserId, seed, cfg, personas } })
           ├─ createState  mints internalToken (crypto.randomUUID() × 2), builds hand 1's HandState
           ├─ onCreate     writes match_meta (personas) to c.db, then
           │               client.aiSeat.create(["table", gameId, "seat", n], { input: { persona, internalToken } })
           │               for n = 1, 2, 3, then c.queue.send("startHand", …)
           ├─ onMigrate(isNew = true)  → createVars (RNG from seed, JWKS cache) → onWake → run
           └─ profile.send("recordHand"/"recordMatch") deferred to hand/game end

PLAY     browser connects → onBeforeConnect (origin) → createConnState (JWT) → onConnect → snapshot()
         loop, per hand:  cutting → deal → bid_round_1 → [dealer_discard] → [bid_round_2]
                          → trick_play ⇄ trick_resolve (×5) → hand_score → next deal
         every mutation:  queue message → reducer → saveState({immediate}) → pushSync → complete()

ARCHIVE  score reaches 10 → phase game_over
         → profile.send("recordMatch")  (retried by flushProfile on failure)
         → final pushSync to every connection
         → aiSeat.send("gameEnd") × 3, then each aiSeat handle .destroy() after its meter flush
         → profile clears activeGameId
         → c.schedule.after(600_000, "selfReap")   the recap stays live for 10 minutes

SLEEP    30 s idle with the run loop blocked in iter() → actor sleeps; WebSockets hibernate and are
         transparently migrated (client sees no interruption). Schedules survive and re-arm on wake.
         Wake order: onMigrate(false) → createVars → onWake (reconciler) → run.
```

---

## 5. Actor-to-actor call graph, and the TypeScript circular-inference caveat

```
                 create ×3, decide, resetHand, handEnd, gameEnd
   euchreTable ─────────────────────────────────────────────────────► aiSeat (×3)
        ▲                                                                │
        │                        aiDecision, aiSay                       │
        └────────────────────────────────────────────────────────────────┘

   euchreTable ── recordHand, recordMatch, noteTokens ──► playerProfile
   aiSeat      ── remember ─────────────────────────────► playerProfile
   SvelteKit   ── create, savePersona, forget, listMatches, getReplay ──► playerProfile, euchreTable
```

Every edge is a **queue message without `wait: true`**. `wait: true` between actors is forbidden
outright: it blocks the sender's run loop until the receiver finishes, and a watchdog cannot rescue
a loop that is blocked on the very send that would have armed it. Replies come back as their own
queue message (`aiDecision`, `aiSay`). Every cross-actor send carries an `AbortSignal`:

```ts
// apps/game/src/actors/euchre-table/dispatch-ai.ts
import type { registry } from "../../registry.js";   // TYPE-ONLY import: no runtime cycle
import { legalMoves, project, rankMoves, type AIDecideRequest, type Seat } from "@euchre/core";

export const AI_HARD_CAP_MS = 4_000;

export async function dispatchAi(c: TableCtx, seat: Seat): Promise<void> {
  const s = c.state;
  const kind = phaseToKind(s.hand.phase);
  const req: AIDecideRequest = {
    turnId: s.turnId,
    internalToken: s.internalToken,
    seat,
    kind,
    view: project(s, seat),                  // redacted — cannot carry a foreign hand
    legal: legalMoves(s, seat),
    ranking: rankMoves(s, seat),
    deadlineAt: Date.now() + AI_HARD_CAP_MS,
  };

  const now = Date.now();
  s.pending = { seat, turnId: s.turnId, requestedAt: now, revealAt: now + revealFloor(kind, s), decision: null, source: null };
  s.watchdogId = await c.schedule.after(AI_HARD_CAP_MS, "onAiTimeout", s.turnId, seat);
  await c.saveState({ immediate: true });     // the dispatch is durable BEFORE it is sent

  const handle = c
    .client<typeof registry>()
    .aiSeat.getOrCreate(["table", s.gameId, "seat", String(seat)], {
      createWithInput: await coldStartInput(c, seat),   // from the match_meta row
    });

  c.broadcast("thinking", { seat, on: true, extended: false });
  await handle.send("decide", req, { signal: AbortSignal.timeout(2_000) });
}
```

**The circular-inference caveat.** `registry.ts` imports the actors; the actors need
`typeof registry` for `c.client<typeof registry>()`. That is a module cycle. Two things keep it
sound, and both are mandatory:

1. **`import type`, never a value import**, for `registry` inside actor files. Type-only imports are
   erased, so there is no runtime cycle. A value import produces `undefined` at module-init time on
   whichever side loses the race.
2. **Explicit return-type annotations on every cross-called action.** Without them, TypeScript tries
   to infer `aiSeat.getPersona`'s return type through `euchreTable`'s type, which is being inferred
   through `aiSeat`'s — the inference collapses and you get `TS2322`/`TS2722`, or, more confusingly,
   `c.state` silently degrading to `unknown` inside an unrelated actor. This *will* bite a
   table ↔ agent pair. Annotate:
   `getPersona: async (c): Promise<PersonaView> => …`, `snapshot: (c): PublicGameView => …`.
   `svelte-check` and `tsc --noEmit` in CI catch a missing annotation only after it has already
   poisoned another file, which is why the rule is "annotate every one, unconditionally".

---

## 6. The realtime protocol, and redaction

### 6.1 Protocol table

| Event | Emitter | Payload | Delivery | Per-recipient redaction |
|---|---|---|---|---|
| `sync` | `euchreTable` | `{ v: number; view: PublicGameView; steps: readonly Step[] }` | **per-connection** `conn.send` | `view = project(state, conn.state.seat)`; `steps = projectSteps(steps, seat)`. The only transport that ever carries card identity. `canSubscribe` = players only. |
| `thinking` | `euchreTable` | `{ seat, on, extended }` | broadcast | none needed — no card identity. `extended: true` once the model has passed the 2200 ms abort and the gate re-armed at +400 ms, so the UI escalates instead of freezing. |
| `chat` | `euchreTable` (relayed from `aiSeat`, or engine-generated) | `{ msgId, seat, kind: 'call'\|'banter'\|'system', text, final: true }` | broadcast | `call`/`system` are engine-authored verbatim table script ("Order it up.", "I assist.", "March!", "Euchre!", "We're in the barn."). `banter` is generated from `publicStateOnly(view)` and then passes `screenBanter()`. |
| `chatDelta` | `euchreTable` (relayed) | `{ msgId, seat, delta }` | broadcast | same screen applied to the accumulated buffer at each flush **and** at final; a rejected line is retracted client-side by `msgId`. Batched at ~80 ms / ~40 chars so a three-seat storm stays ≥10× under the 1000-message queue cap. |
| `presence` | `euchreTable` | `{ seat, online }` | broadcast | none needed. |

`sync.steps` are the transitions since that connection's last `v`. On reconnect `steps` is empty and
the client snaps without animating. **Missed events are never replayed by the transport** — a
genuine network drop loses them — which is why `v` is monotonic and `conn.onOpen` always refetches
`snapshot()`.

### 6.2 Redaction is structural, not disciplinary

`PublicGameView` **has no field capable of holding a foreign hand, a buried kitty card, or the
dealer's discard**. A leak is therefore a compile error, not a review miss. `project()` is the only
producer of that type; `pushSync` is the only sender.

```ts
// packages/euchre-core/src/project.ts
import { legalMoves } from "./legal.js";
import type {
  CardId, GameState, LegalMove, PublicGameView, Seat, Step, Trick,
} from "./types.js";

const NO_LEGAL: readonly LegalMove[] = Object.freeze([]);
const NO_CARDS: readonly CardId[] = Object.freeze([]);

/**
 * THE redaction boundary. `state` is server-only; the return value is the only shape that
 * may reach a browser or an aiSeat. Total, pure, and allocation-only — it never reads
 * hands[s] for s !== seat, never reads kitty[1..3], and never reads dealerDiscard.
 */
export function project(state: GameState, seat: Seat): PublicGameView {
  const h = state.hand;
  const sitting = h.sittingSeat;

  // Your own five cards. A loner's sitting partner holds nothing, publicly or privately.
  const own: readonly CardId[] = seat === sitting ? NO_CARDS : h.hands[seat];

  // Counts only — a count is not card identity, and the client needs it to fan card backs.
  const handCounts: readonly [number, number, number, number] = [
    sitting === 0 ? 0 : h.hands[0].length,
    sitting === 1 ? 0 : h.hands[1].length,
    sitting === 2 ? 0 : h.hands[2].length,
    sitting === 3 ? 0 : h.hands[3].length,
  ];

  // 4 face-down cards on the kitty until the dealer takes the up-card into hand (a round-1
  // call), after which 3 remain. Derived from public fields only; kitty[] itself is not read.
  const takenUp = h.trump !== null && !h.upCardTurnedDown;
  const kittyCount = takenUp ? 3 : 4;

  const legal: readonly LegalMove[] =
    state.status === "active" && h.turnSeat === seat && seat !== sitting
      ? legalMoves(state, seat)
      : NO_LEGAL;

  const trick: PublicGameView["trick"] = {
    index: h.trick.index,
    leadSeat: h.trick.leadSeat,
    ledSuit: h.trick.ledSuit,
    plays: h.trick.plays,          // already-played cards are public by definition
  };

  const lastTrick: Trick | null =
    h.trickLog.length > 0 ? h.trickLog[h.trickLog.length - 1] : null;

  // Derived, never stored: a seat is "thinking" iff it has an in-flight decision for this turn.
  const p = state.pending;
  const busy = (s: Seat): boolean =>
    p !== null && p.turnId === state.turnId && p.seat === s && p.decision === null;
  const thinking: readonly [boolean, boolean, boolean, boolean] = [busy(0), busy(1), busy(2), busy(3)];

  return {
    v: state.v,
    gameId: state.gameId,
    you: seat,
    phase: h.phase,
    cfg: state.cfg,
    handNo: state.handNo,
    dealerSeat: h.dealerSeat,
    turnSeat: h.turnSeat,
    turnId: state.turnId,
    turnDeadlineAt: state.turnDeadlineAt,

    hand: own,
    handCounts,
    kittyCount,

    // The up-card stays publicly IDENTIFIED even after being turned down: every player saw it,
    // and it is load-bearing for counting the seven trump. Suppressing it would be a bug.
    upCard: h.upCard,
    upCardTurnedDown: h.upCardTurnedDown,
    turnedDownSuit: h.turnedDownSuit,

    trump: h.trump,
    makerSeat: h.makerSeat,
    aloneSeat: h.aloneSeat,
    sittingSeat: h.sittingSeat,

    trick,
    lastTrick,
    tricksWon: h.tricksWon,
    trickLog: h.trickLog,

    score: state.score,
    bids: h.bids,
    seats: state.seats,
    thinking,

    legal,
    status: state.status,
    winnerTeam: state.winnerTeam,
  };
}

function assertNever(x: never): never {
  throw new Error(`unhandled Step variant: ${JSON.stringify(x)}`);
}

/**
 * The animation channel is a redaction surface too. Exhaustive by construction: there is no
 * `default` branch, so adding a Step variant that carries private data fails to compile here
 * before it can leak.
 */
export function projectSteps(steps: readonly Step[], seat: Seat): readonly Step[] {
  const out: Step[] = [];
  for (const s of steps) {
    switch (s.t) {
      case "cutOffered": case "cutTaken": case "upCardTurned": case "bid":
      case "turnedDown": case "trumpSet": case "cardPlayed": case "trickWon":
      case "handScored": case "throwIn": case "gameWon":
        out.push(s); break;
      case "dealt":
        // Packets carry { seat, count } — never a CardId. The client animates card backs and
        // fills its own five from `view.hand`. Strictly stronger than blanking foreign cards.
        out.push(s); break;
      case "dealerDiscarded":
        // The Step type has NO card field at all, so V12 is a compile error, not a review item.
        out.push(s); break;
      default:
        return assertNever(s);
    }
  }
  return out;
}
```

> **Spec note:** the synthesis described `projectSteps` as "blanking the card on any `dealt` packet
> not addressed to this seat", and named the projector `toPublicView`. The binding shared-types
> block is authoritative on both points and wins: the exported name is `project()`, and the `dealt`
> step carries `{ seat, count }` with no card field, so there is nothing to blank — the type makes
> the leak unrepresentable rather than merely filtered. `projectSteps` is retained as the
> mandatory, exhaustively-switched gate on the animation channel: it is the only function permitted
> to hand a `Step[]` to a connection, and its missing `default` branch is what will fail the build
> the day someone adds a private step variant.

```ts
// apps/game/src/actors/euchre-table/fanout.ts — the ONLY sender of card data
import { project, projectSteps, type Step } from "@euchre/core";

export function pushSync(c: TableCtx, steps: readonly Step[]): void {
  const s = c.state;
  for (const conn of c.conns.values()) {
    conn.send("sync", {
      v: s.v,
      view: project(s, conn.state.seat),
      steps: projectSteps(steps, conn.state.seat),
    });
  }
}
```

`c.broadcast` may never be called with anything derived from `hands`, `kitty`, or `dealerDiscard` —
enforced by the `no-broadcast-state` ESLint rule and by `redaction.fuzz.spec.ts`, which plays
10,000 seeded games and asserts that for every payload produced for seat *s*, the serialized JSON
contains no `CardId` held by another seat, buried in the kitty, or discarded by the dealer —
including every replay frame. That test is a required CI gate.

---

## 7. Connection authentication

WebSockets carry no custom headers, and `c.request.headers` explicitly does not work for
`.connect()`. All connection auth therefore arrives in connection **params**, supplied by
`getParams` (never a static `params`, which would capture one token forever and start failing 15
minutes later).

```ts
// apps/web/src/routes/api/rivet-token/+server.ts
import { error, json } from "@sveltejs/kit";
import { auth } from "$lib/server/auth";
import type { RequestHandler } from "./$types";

export const POST: RequestHandler = async ({ request, locals }) => {
  if (!locals.user) error(401, "unauthenticated");
  const { token } = await auth.api.getToken({ headers: request.headers });
  return json({ token });
};
```

```ts
// apps/web/src/lib/client/rivet.ts
import { createClient } from "rivetkit/client";
import type { registry } from "../../../../game/src/registry";

export const rivet = createClient<typeof registry>(
  import.meta.env.PUBLIC_RIVET_ENDPOINT ?? "http://localhost:6420",
);

async function freshToken(): Promise<string> {
  const res = await fetch("/api/rivet-token", { method: "POST", credentials: "include" });
  if (!res.ok) throw new Error("unauthenticated");
  const body: { token: string } = await res.json();
  return body.token;
}

export function joinTable(gameId: string) {
  // getParams re-runs on every connect AND every reconnect, so expiry self-heals.
  const handle = rivet.euchreTable.getOrCreate(["table", gameId], {
    getParams: async () => ({ token: await freshToken() }),
  });
  return handle.connect();
}
```

```ts
// apps/game/src/auth/verify.ts — no DB credentials, no shared secret
import { createRemoteJWKSet, jwtVerify } from "jose";

export type Jwks = ReturnType<typeof createRemoteJWKSet>;
export interface PlayerClaims { readonly userId: string; readonly email: string; }

export function makeJwks(appUrl: string): Jwks {
  return createRemoteJWKSet(new URL(`${appUrl}/api/auth/jwks`));
}

export async function verifyPlayer(jwks: Jwks, token: string, appUrl: string): Promise<PlayerClaims> {
  const { payload } = await jwtVerify(token, jwks, {
    issuer: appUrl,
    audience: "euchre-actors",
    clockTolerance: 5,
  });
  const userId = payload.id;
  const email = payload.email;
  if (typeof userId !== "string" || typeof email !== "string") throw new Error("jwt_missing_claims");
  return { userId, email };
}
```

```ts
// apps/game/src/actors/euchre-table/index.ts — connection hooks
import { UserError } from "rivetkit";
import { HUMAN_SEAT } from "@euchre/core";
import { CONFIG } from "../../config.js";
import { verifyPlayer } from "../../auth/verify.js";
import type { TableConnState } from "./queues.js";

interface ConnParams { readonly token: string }

// … inside actor({ … })
onBeforeConnect: (c, _params: ConnParams): void => {
  // Rivet has no WebSocket CORS. The origin allowlist is manual and mandatory.
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
  // Authorization, not just authentication.
  if (claims.userId !== c.state.ownerUserId) {
    throw new UserError("Not your game", { code: "forbidden" });
  }
  return { userId: claims.userId, seat: HUMAN_SEAT, role: "player", since: Date.now() };
},
```

**All three actors** declare `onBeforeConnect` and `createConnState`, not just the table — a
browser can address `playerProfile` directly. `playerProfile.createConnState` additionally asserts
`c.key[1] === claims.userId` (the IDOR gate), and every action re-checks `c.conn.state` rather than
trusting the key it was reached through. The JWKS object is cached in `c.vars` so verification is
one in-memory JWK lookup per connect, not an HTTP round trip.

The client handles expiry without a page reload:

```ts
conn.onError((e) => { if (e.code === "invalid_token") void conn.dispose().then(() => joinTable(gameId)); });
```

---

## 8. Durability

**Every state mutation is a durable queue message processed by one serialized run loop.** Actions
run in parallel and are not durable; two concurrent `playCard` actions would interleave their
`await`s and corrupt the trick. Queue messages are persisted before ack, survive sleep and restart,
and are redelivered if the loop dies before `message.complete()`.

The ordering rule, with no exceptions:

```ts
// apps/game/src/actors/euchre-table/index.ts — the run loop
run: async (c): Promise<void> => {
  await reconcile(c);                                   // close the ack-then-arm window first
  for await (const raw of c.queue.iter({ completable: true })) {
    const m = raw as TableMessage;                      // iter() is AsyncIterable<any> in 2.3.9
    try {
      switch (m.name) {
        case "move":       await onMove(c, m); break;
        case "aiDecision": await onAiDecision(c, m); break;
        case "aiSay":      await onAiSay(c, m); break;
        case "tick":       await onTick(c, m); break;
        case "startHand":  await onStartHand(c, m); break;
      }
    } catch (err) {
      if (err instanceof RuleError) {
        // A rules violation is a TYPED REJECTION, never an unacked message.
        await m.complete({ ok: false, code: err.code, legal: err.legal });
      } else {
        throw err;                                      // genuine fault: leave it for redelivery
      }
    }
  }
},
```

```ts
async function onMove(c: TableCtx, m: MoveMessage): Promise<void> {
  const s = c.state;
  if (m.body.turnId !== s.turnId) throw new RuleError("stale_turn", legalMoves(s, HUMAN_SEAT));
  if (m.body.seq <= s.appliedSeq || s.recentMoveIds.includes(m.body.by.clientMoveId)) {
    await m.complete({ ok: true, v: s.v });             // idempotent replay: a no-op by construction
    return;
  }

  // The acting seat comes from c.conn.state (stamped in submitMove), never from the payload.
  const out = apply(s, HUMAN_SEAT, decodeMoveId(m.body.moveId, s, HUMAN_SEAT));

  commit(c, out.state, m.body.seq, m.body.by.clientMoveId);
  await c.saveState({ immediate: true });               // 1. PERSIST
  pushSync(c, out.steps);                               // 2. FAN OUT
  await m.complete({ ok: true, v: c.state.v });         // 3. ACK
  await advanceTurn(c);
}
```

`await c.saveState({ immediate: true })` sits between the mutation and **both** the fan-out and the
ack, on every queue handler. Plain `c.saveState()` blocks until the next throttled flush (up to
10 s) and is never what you want here. The `save-before-complete` ESLint rule fails the build if
`m.complete(` appears in a function that writes state without an intervening immediate save.

What survives each crash window:

| Crash point | What survives | How play resumes |
|---|---|---|
| Mid-trick, before `saveState` | the message (unacked) | redelivered; reducer re-applies; result identical because the deck order was persisted before the deal |
| Mid-trick, after `saveState`, before `complete()` | state **and** the message | redelivered; `appliedSeq` (in the same `c.state` as the mutation) makes reapplication a no-op **by construction, not by luck** |
| Mid-bid | full `HandState` incl. `bids`, `passes`, `turnSeat` | the bidding sequence continues at the same seat |
| Mid AI call | `pending{seat, turnId, revealAt}` + the workflow history in `aiSeat` | a step that already committed replays from history and is **not re-billed**; a step that had not committed re-runs |
| During the AI pacing floor | `pending.decision` and `pending.revealAt` | the `onWake` reconciler releases it; the move lands late, never lost |
| **After `complete()`, before the new `turnId` is minted / schedule armed / AI dispatched** | consistent state, **no queued message, no armed timer** | this is a permanent stall unless something fixes it — see §9 |

---

## 9. Failure and recovery

### 9.1 The crash-between-ack-and-arm window

The `onWake` reconciler is a required, individually tested component, not a safety net.
`onMigrate(isNew)` runs the `c.state.schema` ladder (Zod `.default()` fill-in) **before**
`onWake`; `createVars` rebuilds the RNG closure from `c.state.seed` and the cached JWKS — which is
exactly why the seed lives in state and the closure does not.

```ts
// apps/game/src/actors/euchre-table/reconcile.ts
import { HUMAN_SEAT } from "@euchre/core";

/** Re-derives the expected pending action from (phase, turnSeat, turnId, pending). */
export async function reconcile(c: TableCtx): Promise<void> {
  const s = c.state;
  if (s.status !== "active") return;
  const now = Date.now();
  const tok = s.internalToken;

  // 1. A parked AI decision. Either its floor has passed (release now) or re-arm the timer.
  if (s.pending !== null && s.pending.turnId === s.turnId && s.pending.decision !== null) {
    if (s.pending.revealAt <= now) {
      await c.queue.send("tick", { kind: "releaseAi", turnId: s.turnId, seq: s.appliedSeq + 1, internalToken: tok });
    } else {
      s.revealId = await c.schedule.after(s.pending.revealAt - now, "releaseAiMove", s.turnId);
      await c.saveState({ immediate: true });
    }
    return;
  }

  // 2. A server-held tempo phase whose gate never fired.
  if (s.hand.phase === "trick_resolve" || s.hand.phase === "hand_score") {
    await c.queue.send("tick", { kind: "tempo", turnId: s.turnId, seq: s.appliedSeq + 1, internalToken: tok });
    return;
  }

  // 3. An AI seat whose turn is open with no in-flight dispatch.
  const seat = s.hand.turnSeat;
  if (seat !== null && seat !== HUMAN_SEAT && s.pending === null) { await dispatchAi(c, seat); return; }

  // 4. The human's turn: re-arm the nudge/abandon ladder.
  if (seat === HUMAN_SEAT) await armHumanLadder(c);
}
```

`apps/game/src/__tests__/recovery.spec.ts` kills the process in each window explicitly, including
this one, and asserts the game advances afterwards.

### 9.2 AI timeout and LLM failure — one retry budget, owned by the ladder

`generateObject` runs with `maxRetries: 0` and `abortSignal: AbortSignal.timeout(2200)`, inside
`ctx.step({ name: "llm", timeout: 2600, maxRetries: 0 })`. `abortSignal` is a wall-clock budget
across the whole call including SDK retries, so stacking `maxRetries` inside it is inert and is
banned. On abort or `NoObjectGeneratedError` the ladder makes at most **one** escalation attempt
(bid path only, `claude-opus-5`, 1400 ms). At 4000 ms from dispatch the table's `onAiTimeout` fires
unconditionally and auto-plays `rankMoves()[0]`. No tail can outrun the cap. Between 2200 ms and
4000 ms the table broadcasts `thinking { extended: true }` rather than freezing.

An `aiSeat` that is unreachable, destroyed, or wedged hits the same 4000 ms watchdog. The game
cannot stall on a dead LLM, a dead AI actor, or a dead browser.

**An illegal LLM move is structurally impossible and defended twice anyway.** The output schema is
`z.enum(legalMoveIds)` rebuilt per decision from the table-supplied legal set, *and* the table
independently re-derives `legalMoves()` and re-validates the returned id before applying it (belt
and braces against rules-package version skew). A rejected move falls back to the heuristic top-1
and is **never retried as the same message** — retrying a poisoned message is how you build a poison
pill. Rejections are logged with the persona id and counted in `meter.illegalAttempts`.

Per-game token budget lives in `aiSeat` state. On exceed, `degraded = true` and that seat runs
heuristic-only for the rest of the match behind a quiet "playing on instinct" HUD chip. That is a
circuit breaker, not a dashboard.

### 9.3 Player disconnect, reconnect, and resync

There is **no 30-second human turn timer**, and this is written down so it is not re-added: there is
one human and nobody is waiting on them; a turn timer means the game plays itself while the user
reads the rules panel. The ladder is 90 s partner nudge → 240 s auto-play of the engine's top move,
which guarantees an abandoned solo game terminates. `status` flips to `"abandoned"` after two
consecutive abandon fires; the match is still journalled.

Reconnect: WebSockets hibernate across actor sleep and are transparently migrated, so a sleeping
actor is invisible to the client. On a genuine drop the client reconnects, `getParams` re-mints the
JWT, and **missed events are gone** — which is why `conn.onOpen` always calls `snapshot()` and the
client hard-resyncs from `v` with `steps: []` (snap, do not animate).

Two tabs: both connect as seat 0 and both receive `sync`. The second submit is a no-op — it fails
the `turnId` nonce and returns `UserError("stale_turn")` with `metadata.legal`, which the client
uses to snap back. `turnId` (server nonce) and `recentMoveIds` (32-entry ring buffer, client
double-click dedupe) are two different mechanisms for two different problems and are never
conflated.

### 9.4 Server-held tempo is a security property

`trick_resolve`, `hand_score`, and the cut window are **real server-held phases** gated by
`c.schedule.after(…, "onTempoGate", turnId)`. A hacked client that skips every local delay still
cannot observe the next card early, **because the next card has not been computed**. All
presentation timing is client-side (the `{brisk 0.7, table 1.0, slow 1.3}` multiplier in
`04-FRONTEND-UX.md`); only information-bearing pauses are server phases.

---

## 10. Scaling and cost

Concurrency is bounded by actor count, and actor count is bounded by concurrent matches: **4 actors
per live match** (1 table + 3 seats) plus 1 long-lived `playerProfile` per user. There is no
singleton in the system, so the scaling story is horizontal by construction — 1,000 concurrent
matches is 4,000 sharded actors with no shared write path and no coordination.

Idle cost is near zero: an actor sleeps after 30 s idle whenever its run loop is blocked in
`iter()`, and its WebSocket hibernates rather than dropping. A `playerProfile` between sessions is
a sleeping row of SQLite.

Per-actor limits that bind here: queue 1,000 messages / 64 KiB per message (the `chatDelta` batching
at ~80 ms keeps a three-seat storm ≥10× under it, and the arbiter fails closed); WebSocket incoming
64 KiB soft; action timeout 60 s (irrelevant — no action awaits an LLM, they are all read-only);
SQLite 10 GiB per actor; 1,200 req/min per actor per IP.

LLM cost is the only meaningful variable cost. The corrected ceiling is **~8 decisions per seat per
hand** (2 bids + 1 discard + 5 plays) = 24 per hand ≈ **700–840 calls per 10-point game** before the
forced short-circuit — not the 1,500 an earlier count claimed by multiplying an already-all-seats
figure by three again. `legal.length === 1` skips the model entirely (very common once follow-suit
bites) and is free. Prompt caching pays only on the Opus bid path: `claude-haiku-4-5` has a
**4096-token minimum cacheable prefix** and silently caches nothing below it, so no `cacheControl`
breakpoint is emitted on the play path at all. `cache-prefix.spec.ts` asserts both facts.

**No number in this system is frozen before it is measured.** The tempo floors, the 4000 ms
watchdog, and the per-game token budget are frozen only after the M4 metering harness reports
measured p50/p95 latency and `usage.inputTokenDetails.cacheReadTokens` / `cacheWriteTokens` over
≥200 real decisions. Measured cost per game, measured p95, and the fallback rate (target <3%) are
recorded in `docs/LEDGER.md`.

---

## 11. Deployment

**Local dev.** `pnpm dev` runs both processes concurrently: `bun --watch src/index.ts` in
`apps/game` (gateway on `http://localhost:6420`, `registry.start()` boots the engine) and `vite dev`
in `apps/web` on 5173. `RIVETKIT_STORAGE_PATH` points at a gitignored directory so actor state
survives a restart of the dev loop. Inspector endpoints are open in dev:
`/inspector/state`, `/inspector/queue?limit=50`, `/inspector/connections`,
`/inspector/workflow-history`, `/inspector/database/rows?table=`. With no `RESEND_API_KEY` the magic
link is logged to the console, so local sign-in never blocks.

The workspace uses pnpm's **default nested `node_modules` linker** — this is load-bearing, not an
oversight: `rivetkit@2.3.9` pins `drizzle-orm ^0.44.x` while `apps/web` needs `0.45.2` for the
Better Auth adapter. CI asserts both are present via `pnpm why drizzle-orm`.

**Production.** `apps/game` deploys as a **long-lived Bun runner process** (`registry.start()`,
runner mode). `apps/web` deploys on `adapter-node` behind the same domain. The browser connects to
the Rivet gateway directly; the web tier is never on the realtime path and never proxies an
`Upgrade`. This is deliberately **not** serverless: serverless flips `drainOnVersionUpgrade` to
`true`, which would drain a live hand mid-trick and force the whole recovery table to be re-derived.

**Versioning and upgrades.** `envoy.version` is the build-time integer from
`RIVET_RUNNER_VERSION` (§2). In runner mode `drainOnVersionUpgrade` defaults to `false`, so a
deploy mid-match lets both versions coexist: in-flight tables continue on the old runner until they
sleep, and new tables are allocated to the new one. A deploy performed mid-hand does not drain the
table and play continues — this is an M10 exit criterion, verified, not assumed.

State-shape migrations run in `onMigrate` via the `c.state.schema` ladder with Zod `.default()`
fill-in; anything harder than a field default belongs in SQLite. Actor SQLite migrations are
generated per actor folder (`find src/actors -name drizzle.config.ts -exec drizzle-kit generate
--config {} \;`), **committed**, and applied automatically by `db({ schema, migrations })` inside a
SQLite savepoint, so they are atomic.

Environment: `apps/game` needs `RIVET_ENDPOINT`, `RIVET_TOKEN`, `RIVET_NAMESPACE`,
`RIVET_RUNNER_VERSION`, `APP_URL`, `ALLOWED_ORIGINS`, `ANTHROPIC_API_KEY`. `apps/web` needs
`BETTER_AUTH_SECRET`, `BETTER_AUTH_URL`, `DATABASE_URL`, `RESEND_API_KEY`, `EMAIL_FROM`,
`PUBLIC_RIVET_ENDPOINT`. Neither list contains the other's secrets, and that separation is the
security model, not a convenience.
