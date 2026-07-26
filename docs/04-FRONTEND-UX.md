# 04 — Frontend & UX

**Scope.** Everything that runs in a browser: `apps/web` (SvelteKit 2.70.1 · Svelte 5.56.8 · Vite 7.3.6 · adapter-node 5.5.7) and the 3D layer (`@threlte/core@8.5.16` · `@threlte/extras@9.21.0` · `three@0.185.1`). Rules live in `packages/euchre-core` (`02-GAME-RULES-ENGINE.md`); the authority lives in the `euchreTable` actor (`01-ARCHITECTURE.md`); the opponents live in `aiSeat` actors (`03-AI-AGENTS.md`). Build order is `05-IMPLEMENTATION-PLAN.md`; the frontend spans M2 (DOM-only playable), M6 (Threlte), M7 (tempo/sound), M8 (history), M9 (a11y/mobile).

---

## 1. The Svelte 5 constraint (binding on every file in `apps/web`)

This app is **runes-only**. The following are compile-or-review failures, not preferences:

| Required | Banned |
|---|---|
| `$state` / `$state.raw` | `let x = 0` with implicit reactivity, `writable()` / `readable()` stores of our own authorship |
| `$derived` / `$derived.by` for anything computed | `$effect` that assigns state; `$:` |
| `$props()` with an explicit `interface Props` | `export let`, `$$props`, `$$restProps` |
| `{#snippet}` + `{@render}` | `<slot>`, `<svelte:fragment>`, `$$slots` |
| `createContext<T>()` → `[get, set]` | `setContext` / `getContext` with string keys |
| Keyed `{#each xs as x (x.key)}` | index keys, unkeyed each over card lists |
| Classes with `$state` fields (`TableStore`, `Director`, `SettingsStore`) | shared-module mutable singletons for per-user state |
| `{@attach fn}` | `use:action` |
| `onclick={...}`, `<svelte:window onkeydown={...}>` | `on:click`, `onMount` for listener wiring |
| clsx-style `class={[a, b && c]}` | `class:` directive |

`$effect` is permitted in exactly **two** places in the whole app, both documented inline with the reason: (1) `LiveRegions.svelte`, where an announcement string must be *pushed* into a DOM node the browser only re-reads on mutation; (2) `Stage.svelte`, where the `<canvas aria-label>` summary is written to a node Threlte owns. Everything else — fan layout, legality, camera framing, HUD text — is `$derived`. The 3D layer contains **zero** `$effect`s: `useTask` integrates toward `$derived` targets and writes straight into `Object3D`s.

The only stores in the codebase are the ones libraries hand us (`useThrelte().size`, `authClient.useSession()`). We never author one.

---

## 2. Routes and files

```
apps/web/src/routes/
├── +layout.svelte                       # shell: skip-link, <LiveRegions/>, theme + a11y prefs
├── +layout.server.ts                    # returns { user: locals.user ?? null }
├── login/+page.svelte                   # magic-link request form
├── login/check-email/+page.svelte       # "check your email" confirmation
├── auth/continue/+page.svelte           # SafeLinks/Proofpoint interstitial — requires a real click
├── api/rivet-token/+server.ts           # POST → 15-min JWT, audience 'euchre-actors'
└── (app)/
    ├── +layout.server.ts                # redirect guard for every authed route
    ├── play/+page.server.ts             # resume-or-create; the SERVER creates the game
    ├── play/[gameId]/+page.server.ts    # ownership check + first snapshot
    ├── play/[gameId]/+page.svelte       # the table
    ├── games/+page.server.ts            # paginated match list
    ├── games/+page.svelte
    ├── games/[matchId]/+page.server.ts  # server-derived replay frames  ← see Spec note
    ├── games/[matchId]/+page.svelte     # replay through the SAME director
    ├── settings/+page.server.ts         # load + savePersona/forget form actions
    └── settings/+page.svelte
```

> **Spec note:** the file tree in `00-OVERVIEW.md` lists `(app)/games/[matchId]/+page.svelte` with no sibling `+page.server.ts`. That is a gap, not a decision: `playerProfile.getReplay(id)` must run server-side because the raw seed never reaches a client and the server re-derives frames through `@euchre/core`. This document adds `(app)/games/[matchId]/+page.server.ts`; nothing else in the tree changes.

### 2.1 Auth wiring and route protection

`hooks.server.ts` is the only place a session is read. `svelteKitHandler` intercepts `/api/auth/*` itself — there is deliberately **no** `[...all]` catch-all route in Better Auth 1.6.x.

```ts
// apps/web/src/hooks.server.ts
import { auth } from '$lib/server/auth';
import { svelteKitHandler } from 'better-auth/svelte-kit';
import { building } from '$app/environment';
import type { Handle } from '@sveltejs/kit';

export const handle: Handle = async ({ event, resolve }) => {
  const session = await auth.api.getSession({ headers: event.request.headers });
  if (session) {
    event.locals.session = session.session;
    event.locals.user = session.user;
  }
  return svelteKitHandler({ event, resolve, auth, building });
};
```

```ts
// apps/web/src/routes/(app)/+layout.server.ts
import { redirect } from '@sveltejs/kit';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = ({ locals, url }) => {
  if (!locals.user) redirect(302, `/login?next=${encodeURIComponent(url.pathname)}`);
  return { user: locals.user };
};
```

`$lib/auth-client.ts` exports `createAuthClient({ plugins: [magicLinkClient()] })` for the sign-in call only. Auth *state* always comes from server-loaded `data.user`; `authClient.useSession()` is used nowhere, because a store-shaped auth signal in a runes app is a foot-gun and the server already knows the answer. Route protection is therefore one guard in one file. `/play/[gameId]` adds an ownership check and returns `404`, not `403`, for a game the user does not own, so match ids are not enumerable.

```ts
// apps/web/src/routes/api/rivet-token/+server.ts
import { json, error } from '@sveltejs/kit';
import { auth } from '$lib/server/auth';
import type { RequestHandler } from './$types';

export const POST: RequestHandler = async ({ request, locals }) => {
  if (!locals.user) error(401, 'unauthenticated');
  const { token } = await auth.api.getToken({ headers: request.headers });
  return json({ token });
};
```

The browser mints this token through `getParams` — **never** a static `params` — so every reconnect re-mints and a 15-minute expiry self-heals without a page reload.

```ts
// apps/web/src/lib/client/rivet.ts
import { createClient } from 'rivetkit/client';
import type { registry } from '@euchre/game/registry';

export const rivet = createClient<typeof registry>(
  import.meta.env.PUBLIC_RIVET_ENDPOINT ?? 'http://localhost:6420'
);

async function freshToken(): Promise<string> {
  const res = await fetch('/api/rivet-token', { method: 'POST', credentials: 'include' });
  if (!res.ok) throw new Error('unauthenticated');
  return (await res.json() as { token: string }).token;
}

export function joinTable(gameId: string) {
  return rivet.euchreTable
    .getOrCreate(['table', gameId], { getParams: async () => ({ token: await freshToken() }) })
    .connect();
}
```

---

## 3. Screens

Wireframes are landscape/desktop unless marked. Portrait reflow is §14.

### 3.1 `/login` — magic-link sign-in

```
┌──────────────────────────────────────────────┐
│                  ♠ ♥  EUCHRE  ♦ ♣            │
│                                              │
│      Take a seat. We'll email you a link.    │
│   ┌────────────────────────────────────┐     │
│   │ you@example.com                    │     │  ← type=email, autocomplete=email
│   └────────────────────────────────────┘     │     inputmode=email, required
│   [        Deal me in        ]               │  ← disabled while status==='sending'
│                                              │
│   No password. The link works once and       │
│   expires in 15 minutes.                     │
└──────────────────────────────────────────────┘
```

```svelte
<!-- apps/web/src/routes/login/+page.svelte -->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { authClient } from '$lib/auth-client';

  let email = $state('');
  let status = $state<'idle' | 'sending' | 'error'>('idle');
  const next = $derived(page.url.searchParams.get('next') ?? '/play');
  const busy = $derived(status === 'sending');

  async function send(event: SubmitEvent) {
    event.preventDefault();
    status = 'sending';
    const { error } = await authClient.signIn.magicLink({
      email,
      callbackURL: next,
      newUserCallbackURL: '/play?firstRun=1',
      errorCallbackURL: '/login?error=1'
    });
    if (error) { status = 'error'; return; }
    await goto(`/login/check-email?to=${encodeURIComponent(email)}`);
  }
</script>

<form onsubmit={send}>
  <label for="email">Email</label>
  <input id="email" type="email" bind:value={email} required autocomplete="email" />
  <button disabled={busy}>{busy ? 'Sending…' : 'Deal me in'}</button>
  {#if status === 'error'}<p role="alert">That didn't send. Try again in a moment.</p>{/if}
</form>
```

### 3.2 `/login/check-email` and `/auth/continue`

```
  CHECK YOUR EMAIL                     AUTH/CONTINUE  (the interstitial)
┌────────────────────────────┐       ┌────────────────────────────────┐
│  📬  Sent to               │       │  You're one click from the      │
│      you@example.com       │       │  table.                         │
│                            │       │                                 │
│  Click the link within 15  │       │      [  Take my seat  ]         │
│  minutes. Check spam if it │       │                                 │
│  hasn't arrived.           │       │  (This step stops corporate     │
│  [ Use a different email ] │       │   mail scanners from burning    │
└────────────────────────────┘       │   your one-time link.)          │
                                     └────────────────────────────────┘
```

`/auth/continue` renders **nothing that auto-navigates**: no `<meta refresh>`, no `$effect`-driven `goto`, no prefetch. Outlook SafeLinks, Proofpoint and Gmail all GET the link in the email, and Better Auth consumes the token atomically on first verify. The button is a plain `<a rel="nofollow noreferrer" data-sveltekit-preload-data="off">` pointing at the real `/api/auth/magic-link/verify?…` URL carried in the query string, validated same-origin before render.

### 3.3 `/play` — resume or create

`/play` has no UI of its own; it is a server-side decision that always redirects. This is the mechanism that enforces *the browser never creates a game*.

```ts
// apps/web/src/routes/(app)/play/+page.server.ts
import { redirect } from '@sveltejs/kit';
import { randomUUID } from 'node:crypto';
import { serverRivet } from '$lib/server/rivet';
import { defaultPersonas } from '$lib/server/personas';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => {
  const userId = locals.user!.id;
  const profile = serverRivet.playerProfile.getOrCreate(['user', userId]);

  const active = await profile.activeGame();
  if (active) redirect(302, `/play/${active}`);

  const settings = await profile.getSettings();
  const gameId = randomUUID();
  await serverRivet.euchreTable.create(['table', gameId], {
    input: {
      ownerUserId: userId,
      seed: randomUUID(),
      cfg: settings.cfg,
      personas: defaultPersonas(settings),
      internalToken: `${randomUUID()}${randomUUID()}`
    }
  });
  redirect(302, `/play/${gameId}`);
};
```

A user with an active game resumes it; the "new game" affordance is the table's **Resign & deal fresh** button, which flips the match to `abandoned` server-side before returning to `/play`.

### 3.4 `/play/[gameId]` — the table (landscape ≥1024 px)

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ ♦ TRUMP  DIAMONDS   made by  Ruthie (N)      US 7  ·  THEM 9  (to 10)   ⚙ ? ⏻ │  ← HUD rail, DOM
│ tricks  US 2 / THEM 1                        THEM ARE IN THE BARN             │
├───────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│                        ╭───────────────╮                                      │
│                        │   Ruthie  N   │ ← partner, 3 backs, ●thinking        │
│                        ╰───────────────╯                                      │
│                                                                               │
│   ╭──────╮                  ┌────┐                          ╭──────╮          │
│   │ Cal  │                  │ 9♦ │  ← trick zone            │ Dot  │          │
│   │  W   │            ┌────┐└────┘┌────┐                    │  E   │          │
│   │ ▮▮▮  │            │ K♦ │      │ Q♣ │                    │ ▮▮▮  │          │
│   ╰──────╯            └────┘      └────┘                    ╰──────╯          │
│                                                                               │
│      [pile▮]                    ◈ kitty (3) + turned-down 10♠      [pile▮]    │
│                                                                               │
│              ┌──┬──┬──┬──┬────┐                                               │
│              │J♦│A♦│K♠│9♣│ T♥ │  ← your hand; trump-edged cards glow gold     │
│              └──┴──┴──┴──┴────┘     illegal cards dimmed; lifted card raised  │
│  ╭─ TALK ────────────────────────────╮        ╭─ last trick ─╮   ● YOUR TURN  │
│  │ Ruthie: "Order it up."            │        │ ▫ ▫ ▫ ▫  hold│                │
│  │ Cal:    "Bold, partner."          │        ╰──────────────╯                │
│  ╰───────────────────────────────────╯                                        │
└───────────────────────────────────────────────────────────────────────────────┘
```

Only the felt, cards, seat markers, kitty and trick piles are WebGL. Every glyph you can read — score, trump word, names, talk log, buttons, the "YOUR TURN" chip — is DOM in an absolutely positioned sibling of `<Canvas>` with `pointer-events: none`, restored to `auto` on interactive descendants. That is what keeps text legible at 200 % OS scaling and out of the texture budget. Bidding swaps the drop zone for `BidPanel` / `SuitPicker`; the dealer discard widens the fan to six and is a plain card tap.

```
BID ROUND 1 (up-card 10♠ showing)      BID ROUND 2 (10♠ turned down)
┌───────────────────────────────┐      ┌──────────────────────────────────┐
│ Order it up?   ☐ go alone     │      │ Name trump (not spades)          │
│ [ Pass ] [ Order it up ]      │      │ [ ♥ ] [ ♦ ] [ ♣ ]   ☐ go alone   │
└───────────────────────────────┘      │ [ Pass ]                         │
   dealer's partner sees "I assist"    └──────────────────────────────────┘
   the dealer sees "I take it"            stuck: the Pass button is ABSENT
```

### 3.5 `/games` — history list

```
┌────────────────────────────────────────────────────────────────┐
│  YOUR GAMES                                       [ New game ] │
│  42 played · 24 won · 9 euchres dealt · 3 loners made          │
├────────────────────────────────────────────────────────────────┤
│ ● WON  10–7   12 hands   26 Jul, 14:02   Ruthie/Cal/Dot   →   │
│ ○ LOST  6–10  11 hands   25 Jul, 21:40   "Riverboat" set  →   │
│ ◐ ABANDONED  4–2   5 hands  24 Jul, 09:11                 →   │
│                              [ Load 25 more ]                  │
└────────────────────────────────────────────────────────────────┘
```

Rows come from `playerProfile.listMatches(cursor)`. Abandoned matches appear because hands are journalled at every hand boundary, not only at game over. The header stats come from `lifetime`, recomputed by the nightly `rollup` cron.

### 3.6 `/games/[matchId]` — replay & summary

```
┌────────────────────────────────────────────────────────────────┐
│ ← Games      WON 10–7 · 26 Jul · personas v3 (as played)       │
├──────────────┬─────────────────────────────────────────────────┤
│ HAND 1  ♥ +1 │                                                 │
│ HAND 2  ♠ −2 │        [ the same 3D table, all four hands      │
│ HAND 3  ♦ +4 │          face up — the buried three and the     │
│ ▸HAND 4  ♣ +2│          dealer's discard stay face down ]      │
│ HAND 5  ♥ +1 │                                                 │
│              │  ⏮  ◀  ⏸  ▶  ⏭     ●━━━━━━━━━━○━━  trick 3/5   │
│              │  ×0.5 ×1 ×2                                     │
├──────────────┴─────────────────────────────────────────────────┤
│ WHY DID IT DO THAT   Ruthie played K♦: "Partner led trump, I   │
│ hold the second-best diamond — cheapest card that still wins." │
└────────────────────────────────────────────────────────────────┘
```

The replay renders through the **same** `Director` and scene, fed by `ReplayFrame[]` instead of `sync` events, so later rendering improvements apply retroactively. Rationales come from `aiSeat.getDecisionLog()` mirrored into the journal.

### 3.7 `/settings`

```
┌─────────────────────────────────────────────────────────────────────┐
│ SETTINGS                                                            │
│ ┌ THE TABLE ───────────────────────────────────────────────────────┐│
│ │ House dynamics prompt                            412 / 1000      ││
│ │ ┌───────────────────────────────────────────────────────────────┐││
│ │ │ Friday-night basement euchre. Fast, chirpy, nobody sulks.     │││
│ │ └───────────────────────────────────────────────────────────────┘││
│ └──────────────────────────────────────────────────────────────────┘│
│ ┌ PARTNER (North) — "Ruthie" ──────────────────────────────────────┐│
│ │ Presets: (•) Steady Eddie ( ) Riverboat ( ) The Professor ( ) …  ││
│ │ Difficulty [rookie|casual|(expert)]  Aggression ▁▃▅▇  Risk ▁▃▅   ││
│ │ Chattiness ▁▃▅▇▉                                                 ││
│ │ Prompt                                            1 214 / 2000   ││
│ │ ┌───────────────────────────────────────────────────────────────┐││
│ │ │ You are Ruthie, 61, retired postmaster…                       │││
│ │ └───────────────────────────────────────────────────────────────┘││
│ │ PREVIEW ▾  the exact bytes the model will see                    ││
│ │ ┌───────────────────────────────────────────────────────────────┐││
│ │ │ [L0 rules · immutable · 3 118 tok]                            │││
│ │ │ <<<PERSONA seat=2>>> You are Ruthie… <<<END PERSONA>>>        │││
│ │ │ [L2 state notation · per decision]                            │││
│ │ └───────────────────────────────────────────────────────────────┘││
│ │ Sample lines: "Nice lead." · "That'll do." · "Ours, partner."    ││
│ │ [ Hear them talk ]  (1 model call · 5/min)   [ Save persona ]    ││
│ └──────────────────────────────────────────────────────────────────┘│
│ ┌ WHAT THEY REMEMBER ABOUT YOU ────────────────────────────────────┐│
│ │ Ruthie · 22 Jul · "You went alone at 8–9 and made it."  [Forget] ││
│ └──────────────────────────────────────────────────────────────────┘│
│ ┌ TABLE & ACCESSIBILITY ───────────────────────────────────────────┐│
│ │ Deck  (•) four-colour ( ) two-colour ( ) large index             ││
│ │ Tempo ( ) brisk (•) table ( ) slow      Quick play  [ off ]      ││
│ │ Motion (•) system ( ) reduced   Banter [ on ]  SR banter [ off ] ││
│ │ Coach hints [ on ]   Sound ▇▇▇▁▁   Haptics [ on ]                ││
│ └──────────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────────┘
```

Character counts are `$derived` from the bound value and turn amber at 90 % and red at the cap; `maxlength` enforces it in the DOM and `+page.server.ts` re-enforces it server-side (untrusted text). Presets fill the textarea client-side — choosing one does not save. Every save writes a `persona_versions` row and is scoped by `locals.user.id` in the form action, never by a `userId` in the body. **Saves do not affect a running match** — personas are snapshotted at game creation — and the page says so inline, next to the Save button.

`PREVIEW` is deterministic and free: the assembled three-layer prompt with the user block hard-delimited and highlighted, plus a live token estimate, so a user can see whether their persona pushes the Opus cached prefix past 4 096 tokens. **Hear them talk** is the only Settings control that costs a model call; rate-limited 5/min/user server-side, and it shows the phrasebook fallback if the leak filter rejects the line.

---

## 4. The client realtime layer

Two classes, both `$state`-bearing, both provided through `createContext` at `play/[gameId]/+page.svelte` so nothing leaks between SSR requests.

```ts
// apps/web/src/lib/client/table-context.ts
import { createContext } from 'svelte';
import type { TableStore } from './table.svelte';
export const [getTable, setTable] = createContext<TableStore>();
```

```ts
// apps/web/src/lib/client/table.svelte.ts
import { ActorError } from 'rivetkit/client';
import {
  applyOptimistic,
  type LegalMove, type LegalMoveId, type PublicGameView, type Seat, type Step
} from '@euchre/core';
import type { Director } from '$lib/three/director.svelte';
import { announce } from '$lib/a11y/announce';
import { sound } from '$lib/a11y/sound';
import { haptics } from '$lib/a11y/haptics';

/** Structural view of the handle returned by `joinTable()`; the concrete type satisfies it. */
export interface TableConnection {
  on<T>(name: string, handler: (payload: T) => void): () => void;
  onOpen(handler: () => void): void;
  onClose(handler: () => void): void;
  onStatusChange(handler: (status: string) => void): void;
  snapshot(): Promise<PublicGameView>;
  submitMove(moveId: LegalMoveId, clientMoveId: string): Promise<{ ok: true; v: number }>;
  dispose(): Promise<void>;
}

export interface ChatLine {
  msgId: string; seat: Seat; kind: 'call' | 'banter' | 'system'; text: string; final: boolean;
}
export interface Rejection { code: string; legal: readonly LegalMove[]; at: number }

export class TableStore {
  /** The rendered view. Equals server truth except during an in-flight optimistic move. */
  view = $state.raw<PublicGameView | null>(null);
  chat = $state<ChatLine[]>([]);
  thinking = $state.raw<readonly [boolean, boolean, boolean, boolean]>([false, false, false, false]);
  online = $state(true);
  selectedMoveId = $state<LegalMoveId | null>(null);
  pendingMoveId = $state<LegalMoveId | null>(null);
  hoverKey = $state<string | null>(null);
  rejection = $state.raw<Rejection | null>(null);

  readonly isMyTurn = $derived(this.view?.turnSeat === 0 && this.view.status === 'active');
  readonly legal = $derived<readonly LegalMove[]>(this.view?.legal ?? []);
  readonly legalIds = $derived(new Set(this.legal.map((m) => m.id)));
  readonly trumpBadge = $derived(this.view?.trump ?? null);

  #conn: TableConnection;
  #director: Director;
  #serverView: PublicGameView | null = null;   // NEVER optimistic
  #serverV = -1;
  #offs: Array<() => void> = [];

  constructor(conn: TableConnection, director: Director) {
    this.#conn = conn;
    this.#director = director;
    this.#offs.push(conn.on<{ v: number; view: PublicGameView; steps: Step[] }>('sync', (p) => this.#onSync(p)));
    this.#offs.push(conn.on<{ seat: Seat; on: boolean; extended: boolean }>('thinking', (p) => {
      const next = [...this.thinking] as [boolean, boolean, boolean, boolean];
      next[p.seat] = p.on;
      this.thinking = next;
    }));
    this.#offs.push(conn.on<ChatLine>('chat', (line) => this.#onChat(line)));
    this.#offs.push(conn.on<{ msgId: string; seat: Seat; delta: string }>('chatDelta', (d) => this.#onDelta(d)));
    conn.onOpen(() => void this.#hardResync());
    conn.onClose(() => { this.online = false; });
    conn.onStatusChange((s) => { this.online = s === 'connected'; });
  }

  /** Two-stage tap, stage 1. Idempotent; tapping the lifted card again commits. */
  select(moveId: LegalMoveId): void {
    if (!this.legalIds.has(moveId)) { this.#rejectLocally(moveId); return; }
    this.selectedMoveId = this.selectedMoveId === moveId ? null : moveId;
    if (this.selectedMoveId) haptics.light();
  }

  async commit(moveId: LegalMoveId): Promise<void> {
    const base = this.#serverView;
    if (!base || this.pendingMoveId !== null) return;
    const chosen = base.legal.find((m) => m.id === moveId);
    if (!chosen) { this.#rejectLocally(moveId); return; }

    this.pendingMoveId = moveId;
    this.selectedMoveId = null;
    this.view = applyOptimistic(base, chosen.move);   // pure, shared with the server reducer
    haptics.medium();
    sound.place(moveId);

    try {
      await this.#conn.submitMove(moveId, crypto.randomUUID());
    } catch (error) {
      if (!(error instanceof ActorError)) throw error;
      this.view = this.#serverView;                   // snap back to whatever the server last said
      const legal = (error.metadata as { legal?: readonly LegalMove[] } | undefined)?.legal;
      this.rejection = { code: error.code, legal: legal ?? base.legal, at: Date.now() };
      this.#director.snapBack(moveId);
      sound.error(); haptics.error();
    } finally {
      this.pendingMoveId = null;
    }
  }

  #onSync(p: { v: number; view: PublicGameView; steps: Step[] }): void {
    if (p.v <= this.#serverV) return;                 // duplicate or out-of-order delivery
    const contiguous = p.v === this.#serverV + 1;
    this.#serverV = p.v;
    this.#serverView = p.view;
    this.view = p.view;                               // server always wins, unconditionally
    this.#director.ingest(p.steps, p.view, { animate: contiguous && p.steps.length > 0 });
    announce.fromSteps(p.steps, p.view);
  }

  async #hardResync(): Promise<void> {
    const view = await this.#conn.snapshot();
    this.#serverV = view.v; this.#serverView = view; this.view = view;
    this.pendingMoveId = null; this.selectedMoveId = null;
    this.#director.hardResync(view);                  // teleport, no animation
    this.online = true;
  }

  #rejectLocally(moveId: LegalMoveId): void {
    this.rejection = { code: 'illegal_move', legal: this.legal, at: Date.now() };
    this.#director.snapBack(moveId);
    sound.error(); haptics.error();
  }

  #onChat(line: ChatLine): void { /* replace-by-msgId, cap 60 */ }
  #onDelta(d: { msgId: string; seat: Seat; delta: string }): void { /* append to msgId buffer */ }

  dispose(): void { for (const off of this.#offs) off(); void this.#conn.dispose(); }
}
```

Three properties are load-bearing:

1. **The optimistic view derives from server truth, never from itself.** `applyOptimistic` is applied to `#serverView`, so a chain of taps cannot drift. `optimistic-agreement.spec.ts` pins `applyOptimistic ≡ apply` on the human's legal set (`02-GAME-RULES-ENGINE.md`).
2. **Rollback is "snap to the latest server view", not "undo my move."** A `sync` may have landed between the tap and the rejection; restoring a saved copy would resurrect stale state. `stale_turn` from a second tab takes exactly this path.
3. **`v` monotonicity is the only ordering primitive.** Events are not replayed after a drop, so `onOpen` always refetches a snapshot rather than trusting the stream.

---

## 5. The Threlte scene graph

```
play/[gameId]/+page.svelte
└── Stage.svelte  ............................ .stage { position:relative; height:100dvh; touch-action:none }
    ├── <Canvas renderMode="on-demand" dpr={dprClamp}
    │           toneMapping={THREE.NeutralToneMapping}
    │           colorSpace={THREE.SRGBColorSpace} shadows={false}>
    │   │        ── children mount client-only; no SSR guard exists anywhere below here ──
    │   ├── Rig.svelte
    │   │   └── <T.PerspectiveCamera makeDefault fov={fov} near={0.05} far={6}>    (§14)
    │   ├── Lighting.svelte
    │   │   ├── <T.AmbientLight intensity={0.55}>
    │   │   ├── <T.DirectionalLight intensity={2.1} position={[1.2,2.4,1.0]}>      (castShadow=false)
    │   │   └── <ContactShadows frames={0} scale={1.25} blur={2.4} far={0.12} opacity={0.42}
    │   │                       resolution={512}/>    ← refresh() only when a card lands
    │   ├── Felt.svelte
    │   │   ├── <T.Mesh> table top   (CircleGeometry r=0.45, felt 512² map)
    │   │   └── <T.Mesh> rail        (TorusGeometry, matte leather)
    │   ├── SeatAnchor ×4   <T.Group>  seat 0 S · 1 W · 2 N · 3 E   (empty Object3D anchors)
    │   │   ├── seat 0 → PlayerHand.svelte
    │   │   │            ├── Card.svelte ×n        (n = 5, or 6 during dealer_discard)
    │   │   │            └── HitProxy.svelte ×n    ← the ONLY objects with pointer handlers
    │   │   └── seats 1–3 → OpponentHand.svelte
    │   │                    └── Card.svelte ×n    (faceUp=false, no handlers)
    │   ├── Kitty.svelte
    │   │   ├── <T.Mesh> buried stack (one BoxGeometry, back texture, 3 cards thick)
    │   │   └── Card.svelte  up-card  (faceUp until turned down, then rotated 180° and kept in place)
    │   ├── TrickZone.svelte
    │   │   └── Card.svelte ×0–4  (3 under a loner)
    │   ├── TrickPile.svelte ×2   (one per team; BoxGeometry scaled by tricks won)
    │   └── DealerButton.svelte   (<T.Mesh> puck at the dealer's seat anchor)
    │
    ├── Hud.svelte  ........................... position:absolute; inset:0; pointer-events:none
    │   ├── ScoreBoard · TrumpBadge · TurnIndicator · TalkLog · LastTrick · Celebration
    │   ├── BidPanel | SuitPicker | DiscardPanel   (phase-switched, pointer-events:auto)
    │   └── Coach · RulesPanel · IllegalWhy · Tutorial
    ├── HandA11y.svelte  ...................... visually-hidden focusable <button> per card
    └── LiveRegions.svelte  ................... aria-live: game (polite) + banter (off by default)
```

`<Canvas>` mounts its children client-only, so **no `browser` guard exists anywhere in the 3D layer**. Shadows are off: a directional shadow map re-rendered per frame is the most expensive thing a card table can do, and `ContactShadows` with `frames={0}` reads better for flat cards at a fixed cost. Seat anchors are plain `Object3D`s at ±0.30 m from the felt centre, rotated inward; every layout function returns positions **in anchor-local space**, so portrait re-framing changes only the camera and the seat radius, never per-card maths.

---

## 6. Card rendering

### 6.1 Geometry

One `PlaneGeometry(0.0635, 0.0889)` per *card face* — poker size 2.5 in × 3.5 in, aspect 0.714 (5:7). Two triangles, four vertices. Cards are **not** boxes: box geometry costs 10 extra triangles and a second material for the edge, and at the camera angles we ship a 0.3 mm edge is sub-pixel.

Geometries are cached by atlas cell, not by card instance: 24 faces + 1 back = **25 geometries for the whole application**, created once and disposed only on a deck-variant swap. A mesh renders face-up or face-down by pointing at the face or the back geometry; the material is `side: THREE.DoubleSide`, so a 180° rotation about the card's normal shows the back cell. The back art is authored mirror-symmetric about its vertical axis (asserted by `build-atlas.ts`), making `DoubleSide`'s mirrored sampling invisible. This buys **one draw call per card** instead of the two that back-to-back planes cost.

### 6.2 Atlas layout — exact dimensions

`scripts/build-atlas.ts` runs at build time only (`@resvg/resvg-js@2.6.2` → raster, `sharp@0.35.3` → pack). Runtime `CanvasTexture` uploads are banned.

| Quantity | Value |
|---|---|
| Cells | 25 (24 faces + 1 back), grid **5 × 5**, row-major, `CARD_IDS` order then `BACK` |
| Content rect | **200 × 280 px** (exact 5:7) |
| Gutter | **12 px** edge-clamped bleed on all four sides |
| Cell pitch | 224 × 304 px |
| Atlas | **1120 × 1520 px** PNG-8 (indexed, ≤128 colours) → RGBA8 on the GPU |
| Resident (mip 0) | 1 120 × 1 520 × 4 B = **6.49 MiB** |
| Resident (full mip chain, ×4⁄3) | **8.66 MiB** |
| Felt (separate) | 512 × 512 → 1.00 MiB, 1.33 MiB with mips |
| **Total GPU texture** | **9.99 MiB** against a 13 MiB budget |
| Variants | `four-colour`, `two-colour`, `large-index` — three atlases, one loaded at a time |

The 12 px gutter is bleed-safe through mip level 3 (a level-3 texel spans 8 source texels). A hand card never renders below ~70 CSS px tall — at dpr 1.5, ~105 device px against a 280 px cell, so mip level 1 at worst. Mip bleed is structurally out of reach, which is why a packed atlas is acceptable here and `DataArrayTexture`/KTX2 are deferred (recorded in `docs/LEDGER.md`).

### 6.3 UV math

Three flips textures on upload (`texture.flipY = true`, the default), so image-space y (down) maps to v = 1 − y/H. `PlaneGeometry(w, h)` with one segment emits vertices **TL, TR, BL, BR** with default UVs `[0,1, 1,1, 0,0, 1,0]`. Baking a cell is a substitution plus a half-texel inset, so bilinear sampling at mip 0 never reaches the gutter:

```ts
// apps/web/src/lib/three/atlas.ts
import {
  BufferAttribute, DoubleSide, LinearFilter, LinearMipmapLinearFilter,
  MeshStandardMaterial, PlaneGeometry, SRGBColorSpace, TextureLoader,
  type Texture, type WebGLRenderer
} from 'three';
import type { CardId } from '@euchre/core';

export const CARD_W = 0.0635;   // metres
export const CARD_H = 0.0889;   // metres — 5:7

export interface AtlasCell { readonly x: number; readonly y: number; readonly w: number; readonly h: number }
export interface AtlasManifest {
  readonly variant: 'four-colour' | 'two-colour' | 'large-index';
  readonly image: string;
  readonly width: number;    // 1120
  readonly height: number;   // 1520
  readonly cells: Readonly<Record<CardId | 'BACK', AtlasCell>>;
}

const HALF_TEXEL = 0.5;

export function bakeUv(cell: AtlasCell, atlas: AtlasManifest): Float32Array {
  const u0 = (cell.x + HALF_TEXEL) / atlas.width;
  const u1 = (cell.x + cell.w - HALF_TEXEL) / atlas.width;
  const vTop = 1 - (cell.y + HALF_TEXEL) / atlas.height;
  const vBot = 1 - (cell.y + cell.h - HALF_TEXEL) / atlas.height;
  //             TL          TR          BL          BR
  return new Float32Array([u0, vTop, u1, vTop, u0, vBot, u1, vBot]);
}

export class AtlasResources {
  readonly manifest: AtlasManifest;
  readonly texture: Texture;
  readonly material: MeshStandardMaterial;
  readonly #geoms = new Map<string, PlaneGeometry>();

  private constructor(manifest: AtlasManifest, texture: Texture) {
    this.manifest = manifest;
    this.texture = texture;
    this.material = new MeshStandardMaterial({
      map: texture, side: DoubleSide, roughness: 0.58, metalness: 0,
      envMapIntensity: 0.3, transparent: false
    });
  }

  static async load(renderer: WebGLRenderer, manifest: AtlasManifest): Promise<AtlasResources> {
    const texture = await new TextureLoader().loadAsync(manifest.image);
    texture.colorSpace = SRGBColorSpace;
    texture.generateMipmaps = true;
    texture.minFilter = LinearMipmapLinearFilter;
    texture.magFilter = LinearFilter;
    texture.anisotropy = Math.min(8, renderer.capabilities.getMaxAnisotropy());
    texture.needsUpdate = true;
    return new AtlasResources(manifest, texture);
  }

  geometry(key: CardId | 'BACK'): PlaneGeometry {
    const cached = this.#geoms.get(key);
    if (cached) return cached;
    const geom = new PlaneGeometry(CARD_W, CARD_H);
    geom.setAttribute('uv', new BufferAttribute(bakeUv(this.manifest.cells[key], this.manifest), 2));
    this.#geoms.set(key, geom);
    return geom;
  }

  dispose(): void {
    for (const geom of this.#geoms.values()) geom.dispose();
    this.#geoms.clear();
    this.material.dispose();
    this.texture.dispose();
  }
}
```

Anisotropy caps at **8**, not 16: 8 is the knee of the quality curve for a plane tilted ≤ 55° from the view axis, and 16 measurably costs fragment bandwidth on Mali/Adreno parts. `NeutralToneMapping` (three r162+) is mandatory — Threlte's default `AgXToneMapping` desaturates the felt green and pushes card reds toward brick. Deck-variant swaps call `dispose()` then `load()`, are blocked mid-hand, and must complete in < 300 ms (M6). `TextureLoader` results are **not** auto-disposed by Threlte: `AtlasResources` owns them and `Stage.svelte` disposes in `onDestroy`.

### 6.4 Draw-call budget

| Object | Count | Draws |
|---|---|---|
| Human hand | 5 (6 during dealer discard) | 6 |
| Opponent hands (backs) | 3 × 5 | 15 |
| Up-card | 1 | 1 |
| Buried kitty (one box) | 1 | 1 |
| Trick cards | 0–4 | 4 |
| Trick piles (one box each) | 2 | 2 |
| Felt · rail · dealer button · contact shadows | 4 | 4 |
| **Peak (start of trick 1)** | | **≈ 33** |

Hit proxies use `MeshDiscardMaterial` and contribute **zero** draw calls. Budget is 60; the measured peak is recorded in `docs/LEDGER.md` at M6. Cards are opaque, so there is no back-to-front sort and no overdraw penalty.

---

## 7. Interaction model

**Recommendation: two-stage tap (select-then-confirm), with drag-to-play as an equal citizen.** Tap 1 lifts; tap 2 on the same card, or a tap in the drop zone, commits. Tapping elsewhere or `Esc` deselects.

Why not the alternatives: **tap-to-play** is fastest but has no misclick recovery, and on a perspective table the top of the fan arc is the smallest target on screen — a fat-finger misplay in euchre is unrecoverable, because there is no post-commit undo. **Drag-only** is precise but fatiguing across 5 tricks × ~12 hands, and worse with a mouse than a tap. Two-stage tap gives a free, zero-latency undo (**the lift is the undo**), maps 1:1 onto the keyboard path (`↑` lift, `Enter` commit) and the screen-reader path (focus = lift, activate = commit), and costs experts nothing because `settings.quickPlay` collapses it to one tap.

There is **no post-commit undo window.** It would desynchronise three AI actors mid-decision and it teaches hesitancy. Rejected moves spring back over **180 ms** with a 3 px shake and an error tone.

**Drag.** Pointer-down begins a drag; the card follows the pointer on the hand plane. Release commits if it travelled **≥ 25 % of card height** toward the table centre, otherwise it springs back. The threshold is in world units, so it is dpi-independent.

**Mobile portrait.** `touch-action: none` on `.stage` prevents scroll-hijack. Hit proxies are **1.25× card width × 1.15× card height** — the raycast target is deliberately larger than the art. The proxy's `raycast` is `meshBounds` (bounding-sphere only), `renderOrder` increases left-to-right across the fan, and `interactivity({ filter: (hits) => hits.slice(0, 1) })` plus `stopPropagation()` guarantee the visually-topmost card wins. Long-press (500 ms) opens the card's rule explanation instead of committing.

```svelte
<!-- apps/web/src/lib/three/HitProxy.svelte -->
<script lang="ts">
  import { T } from '@threlte/core';
  import { MeshDiscardMaterial, meshBounds, useCursor } from '@threlte/extras';
  import { CARD_W, CARD_H } from './atlas';
  import type { LegalMoveId } from '@euchre/core';

  interface Props {
    moveId: LegalMoveId;
    legal: boolean;
    order: number;
    onselect: (id: LegalMoveId) => void;
    onhover: (id: LegalMoveId | null) => void;
  }
  let { moveId, legal, order, onselect, onhover }: Props = $props();
  const { onPointerEnter, onPointerLeave } = useCursor(legal ? 'pointer' : 'not-allowed', 'auto');
</script>

<T.Mesh
  renderOrder={order}
  oncreate={(ref) => { ref.raycast = meshBounds; }}
  onpointerenter={(e) => { e.stopPropagation(); onPointerEnter(); onhover(moveId); }}
  onpointerleave={() => { onPointerLeave(); onhover(null); }}
  onclick={(e) => { e.stopPropagation(); onselect(moveId); }}
>
  <T.PlaneGeometry args={[CARD_W * 1.25, CARD_H * 1.15]} />
  <MeshDiscardMaterial />
</T.Mesh>
```

Pointer handlers exist on these proxies and nowhere else, so Threlte's `interactiveObjects` set is the hand — 5 or 6 objects — not the scene graph. `interactive-objects.spec.ts` asserts `context.interactiveObjects.length === handSize`.

**Desktop keyboard path.** `HandA11y.svelte` is the source of truth; the 3D layer mirrors its focus.

| Key | Action |
|---|---|
| `←` / `→` | traverse **legal** cards only |
| `Shift+←` / `Shift+→` | traverse **illegal** cards too, so a learner can ask why (`whyIllegal` copy appears) |
| `↑` or `Enter` | lift (stage 1) |
| `Enter` on a lifted card | commit |
| `Esc` | deselect / close panel |
| `T` | announce trump + maker |
| `S` | announce score + tricks |
| `L` | open the last-trick viewer |
| `Tab` | into the bid panel, talk log, rules panel — native order |

Focus mirrors into the 3D hover state via `onfocus={() => (table.hoverKey = card.key)}`, so keyboard and pointer never diverge — the lift spring is driven from `hoverKey`, never from a separate pointer flag.

---

## 8. Legal-move affordances

| Signal | Legal | Illegal |
|---|---|---|
| Elevation | resting; +12–16 % of card height on hover/focus over **120–160 ms** | sunk 4–6 px |
| Colour | full | desaturate to 35 %, opacity 60 % |
| Rim | 1.5 px warm rim-light on hover | none |
| Trump | **gold edge on every trump card including the left bower**, always, not only on hover | — |
| Cursor | `pointer` | `not-allowed` |
| DOM mirror | `<button>` enabled | `<button aria-disabled="true">` (focusable via `Shift+arrow`, not `Tab`) |
| On activate | lift, then commit | `IllegalWhy` panel with the engine's `whyIllegal` string |

The gold trump edge is the highest-value affordance in euchre and is **not** optional or a hint-mode feature: the left bower changing suit is the rule that confuses every newcomer, and rendering it as trump in the hand removes the confusion without removing the learning.

**"Your turn"** is a persistent 2 px ring on the seat-0 edge of the felt, the DOM `● YOUR TURN` chip, and one light haptic on transition. Never a modal, never a countdown — there is no human turn timer (`01-ARCHITECTURE.md`). Colour is never the only channel: legality is elevation + saturation + cursor + `aria-disabled`; trump is gold edge + HUD badge + announced string.

---

## 9. Animation choreography

### 9.1 The rule: animation never owns truth

`lib/three/director.svelte.ts` is a **presenter**, not a state machine: it consumes `(view, steps)` and produces motion. It has no branch that reads a rule and no path where a completed animation causes a state change. Consequences, all mechanically checked:

- **`mode: 'snapshot'` is a supported, CI-exercised mode.** Drop every animation, apply only `view`, and the table is exactly right — just instant. It is also how the replay scrubber's `⏭` works.
- **Server truth arrives whole.** `#onSync` assigns `this.view = p.view` *before* the director is told anything; a sync landing mid-flight wins and the flight retargets.
- **Information-bearing pauses are server phases, not client sleeps.** `trick_resolve`, `hand_score` and the cut window are held by `c.schedule.after(…, 'onTempoGate', turnId)`. A client that skips every local delay still cannot see the next card early, because it has not been computed.

### 9.2 The tempo table

`lib/three/tempo.ts` holds these constants × the user's `{ brisk 0.7, table 1.0, slow 1.3 }`, clamped so no flight exceeds 500 ms at `slow`.

| Beat | Trigger (`Step`) | Timing |
|---|---|---|
| Deal | `dealt` | **60 ms** flight/card, **50 ms** stagger, 0.08 m arc → **1 010 ms** for 20 cards, in the engine's 3-2/2-3 packet order |
| Up-card flip | `upCardTurned` | **260 ms** slerp; geometry swaps at the 90° crossing |
| Bid prompt in | phase → `bid_round_1/2` | **140 ms** panel rise + fade; focus lands on the first legal button |
| AI bid line | `bid` | bubble **160 ms** in, dwell 1 400 ms, **200 ms** out — gated by the server pacing floor, never simulated |
| Turn-down | `turnedDown` | **300 ms** flip + **120 ms** slide under the kitty; the card stays publicly identified |
| Dealer pickup | `trumpSet` (round 1) | up-card → hand **240 ms**; fan widens to six over **180 ms** |
| Dealer discard | `dealerDiscarded` | **220 ms** face-down flight + **120 ms** fade under the kitty (the step carries no card identity, by type) |
| Card play | `cardPlayed` | **220 ms** ease-out, **0.03 m** arc, mid-flight flip for opponent cards |
| Trick read pause | `trickWon` | **850 ms** (server-held 700–1 000; **1 100 ms** when the trick seals a euchre); winner's ring pulses at +150 ms |
| Trick sweep | after the pause | **420 ms** to the winning pile, 3-card cascade at 40 ms stagger |
| Hand score | `handScored` | **600 ms** delay → **450 ms** counter → celebration **1 400 ms** (point/march) / **2 400 ms** (euchre/loner), server-held |
| Throw-in | `throwIn` | **500 ms** sweep of all 24 cards to the deck |
| Game over | `gameWon` | **300 ms** dim, **900 ms** banner rise, confetti **2 400 ms**, then the recap CTA |

`prefers-reduced-motion` **or** the in-app override sets every *flight* to 0.001 s and kills rotation, arc and confetti, but **preserves the trick read pause and the 600 ms score delay**. Reduced motion must not mean reduced comprehension time.

### 9.3 Playback, backlog and resync

```ts
// apps/web/src/lib/three/director.svelte.ts (shape; full timeline maths in cardMotion.svelte.ts)
import type { PublicGameView, Step, LegalMoveId } from '@euchre/core';
import { beatDuration } from './tempo';
import { CardMotion } from './cardMotion.svelte';

export interface CardView {
  readonly key: string;                 // CardId for known cards, `s{seat}:{slot}` for hidden ones
  faceKey: string;                      // atlas cell: a CardId or 'BACK'
  faceUp: boolean;
  order: number;
}

const MAX_BACKLOG_MS = 2500;

export class Director {
  cards = $state.raw<readonly CardView[]>([]);
  mode = $state<'animated' | 'snapshot'>('animated');

  readonly #motions = new Map<string, CardMotion>();
  #queue: Step[] = [];
  #view: PublicGameView | null = null;

  ingest(steps: readonly Step[], view: PublicGameView, opts: { animate: boolean }): void {
    this.#view = view;
    if (!opts.animate || this.mode === 'snapshot' || document.visibilityState === 'hidden') {
      this.#queue.length = 0;
      this.#snapTo(view);
      return;
    }
    this.#queue.push(...steps);
    const backlog = this.#queue.reduce((ms, s) => ms + beatDuration(s), 0);
    if (backlog > MAX_BACKLOG_MS) {
      const tail = this.#queue.filter((s) => s.t === 'handScored' || s.t === 'gameWon');
      this.#queue = tail;                 // collapse: keep only the beats that carry meaning
      this.#snapTo(view);
    }
    this.#pump();
  }

  hardResync(view: PublicGameView): void {
    for (const motion of this.#motions.values()) motion.cancel();
    this.#queue.length = 0;
    this.#snapTo(view);                   // teleport every card; no animation, no sound
  }

  snapBack(moveId: LegalMoveId): void { /* 180 ms spring + 3 px shake on the rejected card */ }

  #snapTo(view: PublicGameView): void { /* recompute CardView[] + teleport all motions */ }
  #pump(): void { /* drain #queue one beat at a time; each beat resolves on its own timeline */ }
}
```

- **Falls behind** (throttled tab, slow device, a burst of AI plays): projected playback exceeds 2 500 ms → drop everything except `handScored`/`gameWon`, snap to `view`, resume. The user sees an instant catch-up, never a laggy replay of stale beats.
- **Backgrounded tab**: `document.visibilityState === 'hidden'` forces snapshot behaviour; `<svelte:document onvisibilitychange={…} />` restores animated mode **after** one snap, so nothing plays back out of context.
- **Reconnect mid-animation**: `conn.onOpen` → `snapshot()` → `hardResync(view)`. Every in-flight `CardMotion` is cancelled and teleported; a racing `sync` is ignored by the `v <= #serverV` guard.
- **Rejected optimistic move**: `snapBack(moveId)` returns the card to its slot over 180 ms. The authoritative correction already happened; the spring is cosmetic.

Per-frame values live in `CardMotion` instances in a plain `Map`, **never** in `$state`. Exactly one `useTask` exists in the app, with `autoInvalidate: false`, calling `invalidate()` only while a motion is live:

```ts
useTask((delta) => {
  let live = false;
  for (const [key, motion] of motions) {
    if (motion.step(delta)) live = true;
    const obj = objects.get(key);
    if (obj) { obj.position.copy(motion.pos); obj.quaternion.copy(motion.quat); }
  }
  if (live) invalidate();
}, { autoInvalidate: false });
```

> **Spec note:** `cardMotion.svelte.ts` carries the `.svelte.ts` extension in the binding file tree but declares **no runes at all** — the extension is there for consistency with its siblings and costs nothing. Its per-frame values must never enter the reactivity graph, which is precisely why it is a plain class.

24 cards × 60 fps of `$state` writes would be 1 440 component updates per second; this is zero. `svelte/motion`'s `Spring`/`Tween` drive exactly three continuous values — hover lift, selected-card raise, camera dolly — because each runs its own rAF loop and 24 of them is the same mistake in a different shape. (`useSpring` does not exist in `@threlte/extras@9`; do not reach for it.)

---

## 10. AI think-delay pacing

The client **never** simulates AI thinking. It renders exactly what arrives, and the arrival time is a server property.

| Decision | Server-side floor (`revealAt = requestedAt + jitter`) |
|---|---|
| Exactly one legal move | **250–450 ms** (zero model calls; the floor still applies) |
| A genuine choice among plays | **900–1 400 ms** |
| Any bid / discard / go-alone | **900–1 800 ms** |

Enforcement is persisted, not a `setTimeout`: an early decision is parked in `c.state.pending` with its `revealAt` and released by `c.schedule.after(…, 'releaseAiMove', turnId)`, so a crash inside the pacing window still delivers the move on wake. Between 2 200 ms and the 4 000 ms hard cap the table broadcasts `thinking { extended: true }` and the client escalates the seat indicator from a pulsing dot to a three-dot ellipsis — never a freeze, never a spinner. At 4 000 ms `onAiTimeout` auto-plays `rankMoves()[0]`, and the client cannot tell the difference, which is the point (full ladder in `03-AI-AGENTS.md`). A seat whose token budget is exhausted renders a quiet **"playing on instinct"** chip; that is the circuit breaker's only surfacing.

---

## 11. Sound and haptics

`lib/a11y/sound.ts` preloads nine decoded buffers behind one `AudioContext` resumed on the first user gesture. Every cue is duckable to zero; sound is never the only channel for anything.

| Cue | Character | Length |
|---|---|---|
| `deal` | soft riffle, one per packet not per card | 180 ms |
| `place` | felt thud, **pitch-varied ±3 semitones by card id hash** so five plays never sound identical | 120 ms |
| `lift` | cloth brush | 90 ms |
| `trickTake` | short swept collect | 260 ms |
| `trumpCalled` | single struck tone, one per suit (four distinct pitches) | 300 ms |
| `euchre` | descending minor third | 380 ms |
| `march` | ascending fanfare | 400 ms |
| `gameWon` | full cadence | 1 200 ms |
| `error` | dry muted click, deliberately unmusical | 80 ms |

Haptics (`navigator.vibrate`; iOS falls back to silence): **light** on lift, **medium** on commit, **success** (two short) on taking a trick, **error** (one long) on rejection. Both channels honour the Settings toggles and neither fires while the tab is hidden.

---

## 12. Game history UI

`/games` calls `playerProfile.listMatches(cursor)` in `+page.server.ts` and renders a keyed `{#each matches as m (m.id)}` with cursor pagination — no infinite scroll, because a "Load 25 more" button is keyboard-reachable and does not trap focus.

`/games/[matchId]` loads `getReplay(id)` **server-side**; the server re-derives frames through `@euchre/core` and the raw seed never reaches the browser. Replay projection deliberately shows **all four hands** — that is how anyone learns euchre — but the three buried kitty cards and the dealer's discard are masked in every frame, and `redaction.fuzz.spec.ts` covers the replay path explicitly. Transport is `⏮ ◀ ⏸ ▶ ⏭` plus a per-trick scrubber and ×0.5/×1/×2, all feeding the same `Director` via `ingest(frame.steps, frame.view, { animate: speed > 0 })`. Storage is ≤ 3 KB per game because a replay is `{ seed, moves[] }`, not frames.

---

## 13. Accessibility

**Four-colour deck, default on.** ♠ black · ♥ red · ♣ green · ♦ blue, all measured against the card stock `#F5F1E6`:

| Suit | Hex | Contrast vs `#F5F1E6` |
|---|---|---|
| Spades | `#12161A` | 16.4 : 1 |
| Hearts | `#C0192B` | 5.45 : 1 |
| Clubs | `#0E7A3C` | 4.81 : 1 |
| Diamonds | `#1B4FD8` | 5.90 : 1 |

All clear WCAG 2.2 SC 1.4.3 (4.5:1) as text and SC 1.4.11 (3:1) as graphics. Suit is **never** encoded by colour alone — glyph + colour + letter are in the art, in every variant.

> **Spec note:** four colours create a genuine tension with euchre's core rule. A two-colour deck makes the ♠↔♣ / ♥↔♦ pair — exactly what defines the left bower — visible as ink colour; four colours destroy that signal. Resolution: promote the pairing out of the art and into the UI. Whenever trump is set, `TrumpBadge` renders `♥ TRUMP · J♦ is the left bower` explicitly, every trump card in hand carries the gold edge *including* the left bower, and the `T`/`L` announcements name it. The four-colour win (instant suit disambiguation for CVD users, the population most harmed by red/black confusion) is kept; the loss is paid for with an explicit affordance rather than left implicit.

**Two aria-live regions, separated on purpose.** `#live-game` (`aria-live="polite" aria-atomic="true"`) carries every game event; `#live-banter` (**default off**, user-toggleable) carries persona chatter. Banter therefore never interleaves with a trump call — the failure a single region guarantees. Every `Step` maps to one sentence in `lib/a11y/announce.ts`, generated from the same projected steps the animation uses:

| Step | Announcement |
|---|---|
| `dealt` | "Cards dealt. Your hand: jack of diamonds, ace of diamonds, king of spades, nine of clubs, ten of hearts." |
| `upCardTurned` | "Up-card is the ten of spades." |
| `bid` | "Ruthie orders it up." / "Dot assists." / "You pass." |
| `turnedDown` | "Spades turned down. Spades may not be named." |
| `trumpSet` | "Hearts are trump, made by Ruthie. The jack of diamonds is now a heart." |
| `dealerDiscarded` | "The dealer discards face down." |
| `cardPlayed` | "Cal plays the queen of clubs." |
| `trickWon` | "You take the trick. Two of five. Your lead." |
| `handScored` | "March. You score two. Seven to nine — they are in the barn." |
| `throwIn` | "All passed twice. Thrown in; the deal moves left." |
| `gameWon` | "You win, ten to seven." |

`<canvas role="img" aria-label={summary}>` carries a summary regenerated from the public view: trump, maker, alone state, tricks, score, whose turn. `HandA11y.svelte` is a visually-hidden but focusable `<button>` list — **a complete, playable card table on its own**, shipped un-hidden at M2 before three.js exists, and the permanent keyboard and screen-reader path. Card labels read `"Jack of diamonds, left bower, trump, playable"`.

Every glyph is DOM/CSS sized in `rem`, and the HUD is verified unclipped at 200 % OS scale (`clamp()` type scale, `min-height` rather than fixed heights). The **large-index** deck variant (oversized corner pips) is a first-class Settings option built by the same `build-atlas.ts`. `prefers-reduced-motion: reduce` is read at mount and merged with the in-app override; §9.2 defines the exact behaviour.

---

## 14. Responsive breakpoints and the portrait camera

| Breakpoint | Layout |
|---|---|
| `< 480 px` portrait | one-column HUD; score/trump collapse into one top rail under `env(safe-area-inset-top)`; talk log becomes a 2-line ticker with a tap-to-expand sheet; hand pinned above `env(safe-area-inset-bottom)` |
| `480–767 px` | landscape phone: HUD splits left (score/trump) and right (talk/last trick) |
| `768–1023 px` | tablet: full HUD rail, talk log docked bottom-left |
| `≥ 1024 px` | the §3.4 layout; talk log and last-trick viewer both permanent |

The canvas is `100dvh`, not `100vh`, so iOS URL-bar collapse cannot resize the scene mid-hand; `app.html` carries `viewport-fit=cover`.

Camera framing is `$derived`, never `$effect`:

```svelte
<!-- apps/web/src/lib/three/Rig.svelte -->
<script lang="ts">
  import { T, useThrelte } from '@threlte/core';
  const { size } = useThrelte();
  const aspect = $derived($size.width / $size.height);
  const portrait = $derived(aspect < 0.8);
  const fov = $derived(portrait ? 44 : aspect < 1.4 ? 38 : 34);
  const dist = $derived(portrait ? 1.05 : aspect < 1.4 ? 0.85 : 0.72);
</script>

<T.PerspectiveCamera
  makeDefault
  {fov}
  near={0.05}
  far={6}
  position={[0, dist * 0.78, dist * 0.82]}
  oncreate={(ref) => ref.lookAt(0, 0, -0.06)}
/>
```

Portrait pulls back and widens the fov so the hand and the trick zone both fit; the seat radius shrinks from 0.30 m to 0.26 m and opponent fans compress to a 3-card visual stack (the *count* stays a DOM number, so no information is lost). An orientation change re-derives fov, distance and fan layout without touching card identity — the in-flight hand survives.

**The affordance floor is proved, not hoped for.** `layout.ts` solves for the largest card width whose span fits the available CSS width:

```ts
// apps/web/src/lib/three/layout.ts
export const MIN_STRIP_W = 48;   // CSS px — WCAG 2.2 AAA-adjacent target for the exposed strip
export const MIN_STRIP_H = 64;

export interface FanSizing { readonly cardWpx: number; readonly cardHpx: number; readonly overlap: number }

/** Largest card that fits, with overlap relaxed toward 0 until the exposed strip clears the floor. */
export function sizeFan(availablePx: number, n: number, preferredOverlap: number): FanSizing {
  for (let overlap = preferredOverlap; overlap >= 0; overlap -= 0.05) {
    const k = 1 - overlap;
    const cardWpx = availablePx / ((n - 1) * k + 1);
    const strip = cardWpx * k;
    if (strip >= MIN_STRIP_W && cardWpx / 0.714 >= MIN_STRIP_H) {
      return { cardWpx, cardHpx: cardWpx / 0.714, overlap };
    }
  }
  const cardWpx = availablePx / n;                       // overlap 0: the widest possible strip
  return { cardWpx, cardHpx: cardWpx / 0.714, overlap: 0 };
}
```

Cases the unit test pins, sweeping widths 320–430 CSS px at hand sizes 1–6:

- **390 px, n = 5**, 20 px gutters → available 350, overlap 0.30 → card 92 × 129, strip **64 × 129** ✓
- **390 px, n = 6** (dealer discard), overlap 0.30 → card 78 × 109, strip **55 × 109** ✓
- **320 px, n = 6**, available 300, overlap relaxes to **0** → card 50 × 70, strip **50 × 70** ✓ — the binding case

`sizeFan` returns CSS pixels; `layout.ts` converts to world metres via `pxPerMetreAt(depth) = viewportHeightPx / (2·depth·tan(fov/2))` from the live camera, then applies the arc: per-card rotation lerped across **±10°**, arc centre lifted **4 %** of card height, and a **0.4 mm** z-step per card so `renderOrder` and depth agree. The test measures the *projected* unoccluded strip through the real camera; the closed form only seeds the search.

---

## 15. Onboarding

Three modes, all on by default for a new account, all individually dismissible, none of them a rules wall.

1. **Legal-moves-only + "why".** Illegal cards are unplayable and tapping one prints the engine's `whyIllegal` verbatim: *"Hearts were led. Your Jack of diamonds is a heart right now — you have to follow."* The highest-value teaching moment in the product, generated by the rules engine rather than written into the UI.
2. **Coach mode.** `rankMoves()[0]` gets a soft 900 ms pulse and a one-sentence rationale from the Part B heuristics (*"Partner is already winning — throw your worst card."*). Dismissible per hint; **auto-retires after 3 correct unassisted repetitions** of the same situation class, so it disappears without the user hunting for a setting.
3. **First-run scripted hand.** `packages/euchre-core/src/script.ts` supplies a fixed deal guaranteeing a **left-bower moment in trick 1** and a **march** in the same hand, so both signature concepts are learned by doing. It runs against the same actors as a real game — a seeded game, not a mock.

A persistent `?` opens `RulesPanel.svelte` **context-first**: the rule for the current phase at the top, expanded, then a searchable rulebook. It is a `<dialog>` with a focus trap and `Esc` to close, never a route change, so table state is never lost.

---

## 16. Performance budgets

| Budget | Target | Measured at |
|---|---|---|
| Sustained frame rate through a full hand | **60 fps** on a Pixel 6a class device | M6 |
| Idle frame cost | **0** — `renderMode="on-demand"` must not degenerate | M6 |
| Peak draw calls (trick 1) | **≤ 60** (measured ≈ 33) | M6 |
| Resident GPU texture | **≤ 13 MiB** (measured 9.99 MiB) | M6 |
| `dpr` | `[1, 2]`, clamped to `[1, 1.5]` when `navigator.hardwareConcurrency <= 4` | M6 |
| Main-thread work per `sync` | **< 4 ms** at p95 (project + layout + queue push) | M7 |
| `/play` route JS | **≤ 260 KB** gzip (three ≈ 170, Threlte ≈ 22, app ≈ 60) | M10 |
| Time to interactive on throttled 4G | **≤ 2.5 s** to the DOM hand; the atlas streams in behind it | M10 |
| Desync rate over a 50-game lossy-link soak | **< 1 %**, zero stuck or ghost cards | M10 |

**The specific things that would blow them**, each of which is a review-stopper:

- Per-frame values written into `$state` — 24 cards × 60 fps = 1 440 component updates/s. Motion lives in a plain `Map<string, CardMotion>`.
- A `Spring` or `Tween` per card: each spawns its own rAF loop.
- `useTask` without `autoInvalidate: false`, which silently turns on-demand rendering into always-on.
- Pointer handlers on the felt, opponent cards or the trick pile — Threlte then raycasts the whole scene graph instead of five quads.
- Re-enabling shadow maps, or letting the key light move (we ship no shadow map at all).
- `transparent: true` on the card material: back-to-front sorting and overdraw for zero gain on opaque quads.
- Building 52 `CanvasTexture`s from SVG at runtime — 52 uploads, 52 samplers, a visible hitch. `build-atlas.ts` exists to make this impossible.
- `<HTML transform>` nodes or troika `<Text>` in the HUD: a per-frame matrix projection plus CSS write, or an SDF mesh and its own draw call each.
- Constructing geometries or materials inside `{#each}`. 25 geometries and one material exist for the life of the page.
- Unclamped `dpr` on a 3× phone — roughly 2.2× the fragment work.
- Skipping `dispose()` on a deck-variant swap: two resident atlases is 17 MiB.
- Letting the animation queue drive state, which turns every dropped frame into a desync. §9.1 is a budget item, not only a principle.
