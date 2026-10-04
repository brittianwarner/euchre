# Euchre

[Play at euchre.sh](https://euchre.sh)

Single-player euchre against three AI opponents, playable in the browser. One
SvelteKit app — no separate backend to stand up.

## Running it locally

```sh
bun install
bun run dev
```

Open **http://localhost:5173** → _Deal a hand_.

With no `.env` at all you still get a full game:

- `RIVET_ENDPOINT` unset → RivetKit starts its own local engine on `:6420` and
  every match actor runs in-process against that. Local state is isolated in `.rivetkit/`.
- `OPENROUTER_API_KEY` unset → the three AI seats fall back to a heuristic
  player instead of calling an LLM. Add the key (see `.env.example`) to get
  real Jev-driven opponents.

Copy `.env.example` to `.env` and fill in only what you need; every variable
in it documents what it does and whether it's required.

Your hand uses separate, full-card buttons: click or tap a card, then **Play**
(or **Discard**). Double-click plays directly. Arrow keys move focus; Enter or
Space selects. Card names sit beneath the original traditional artwork. The
table fits the visible viewport; shorter phones use one row of cards, while
taller phones use two rows. Remaining cards keep their positions after a play.

A completed trick highlights its winning card, pauses for reading, then gathers
the cards into the winner's stack. **Settings → Wait for me after each trick**
holds the actual game until **Continue**, including after a reload. Reduced-motion
preferences skip card flights. The decorative Threlte felt loads after the game
UI and renders only on demand.

## How the pieces fit

One Railway app service and one private Rivet engine:

- **Frontend** — SvelteKit (Svelte 5 runes) for routing/UI, [Threlte](https://threlte.xyz)
  (Three.js) for the 3D table, camera and card layout.
- **Realtime state** — Rivet actors run in the persistent app process. The private
  engine stores state on its volume. Browsers use the authenticated app gateway.
  - `euchreTable` — the authoritative match: rules, turn order, scoring.
  - `aiSeat` — **one actor per AI opponent** (three per match), each with its
    own persona, memory and failure domain. A hung model call stalls one
    chair, never the table; the table's watchdog auto-plays for it.
  - `playerProfile` — durable per-user history/stats, keyed off an email you
    type in (see "What's not built" below — this is deliberately not an
    account system).
- **AI decisions** — [OpenRouter](https://openrouter.ai), model
  `typesafe/jev-1.13` exclusively for bid/play decisions, through the Decisions API.
  Conventional LLMs handle only optional table talk (`OPENROUTER_MODEL_TALK`).
  Falls back to a heuristic player with zero external calls when no API key
  is configured, so the game is always playable offline.
- **Rules engine** — `src/lib/euchre/**`, a pure, actor-agnostic reducer with
  no I/O; every actor and every test harness drives the same engine.

## Testing & verification

```sh
bun run check          # svelte-check across the whole project
bun run test           # vitest — unit/component suite
```

Two headless harnesses drive the real rules engine outside of vitest:

```sh
bun run scripts/e2e-play.ts             # narrate one game, then fuzz 300 more,
                                         # re-deriving every game invariant from
                                         # the rules rather than the implementation
bun run scripts/ai-live.ts              # play real hands through the actual
                                         # decide() ladder (needs OPENROUTER_API_KEY
                                         # or ANTHROPIC_API_KEY, else it only
                                         # exercises the heuristic fallback)
```

For the 3D scene, static checks don't catch visual bugs — screenshot it:

```sh
bunx vite dev --port 5199 &
node scripts/shot-table.mjs /tmp/shots 5199 1440 900   # landscape
node scripts/shot-table.mjs /tmp/shots 5199 390 844    # portrait
```

then look at the PNGs. `/cardtest` (`http://localhost:5199/cardtest`) renders
`Card.svelte` alone, so a "cards don't show up" report can be traced to the
component or to the scene without guessing.

## Deploying to Railway

See [server/README.md](server/README.md) for the private engine topology, required
variables, pinned image, release commands and live acceptance checks. The app
serves SvelteKit and the authenticated WebSocket gateway in one Bun process.
A separate private Rivet engine keeps actor state on a persistent volume.

## What's _not_ built

- **No magic-link auth.** There's a `better-auth` + Resend scaffold under
  `src/lib/server/auth/**`, but it isn't wired into any route or the actor
  registry — it's inert. The actual identity mechanism for `/games` and
  `/settings` is deliberately lighter, at the user's request: you type an
  email, it's normalized and hashed into a stable id, and that id becomes
  your `playerProfile` key. Nothing verifies the email belongs to you — it's
  a nickname for "this browser's history," not an account, and the UI says
  so.
- **Guest play uses signed browser identity.** An HTTP-only cookie owns each match;
  actor tokens are signed, expire after two hours, and reject another browser.
  Set `EUCHRE_ACTOR_JWT_SECRET` (at least 32 characters) in production. Guest
  play and the older email-nickname history screens remain separate identities.

## Visual assets

Blender 5.2 authored the walnut base, padded rail, brass piping and rounded card
stock. `assets/euchre-table.blend` (hero) and `assets/game-table.blend` (runtime) are editable; `scripts/build-table.py` rebuilds
both small runtime GLBs and the Cycles-rendered landing image. Live gameplay is
rendered by Threlte on demand, with capped pixel ratio and one bounded 1024px shadow map. Idle scenes do not redraw.

Traditional faces use Adrian Kennard's CC0 artwork, distributed by
[letele/playing-cards](https://github.com/letele/playing-cards), from
[the configurable original deck](https://www.me.uk/cards/makeadeck.cgi?view).
The license is retained in `static/art/cards/LICENSE.txt`. The burgundy back is
original vector artwork. Run `node scripts/raster-card-art.mjs` before Blender
when changing card artwork.

The source workspace Svelte adapter is synchronized with Layerr revision
`186866bc9055dadf7b4f89b5d0783a8f386da843`; it includes its local framework bridge.

Browser acceptance with a locally configured OpenRouter key:

```sh
EUCHRE_FAST_TEMPO=1 bun run dev
node scripts/e2e-browser.mjs
```

The browser harness plays a complete match, reloads into the same game, checks
phone overflow, collects browser errors and measures idle draws. It requires live
Jev calls from every AI seat. `scripts/actor-security.mjs <gameId>` checks forged
connection and publish rejection against the local engine.
