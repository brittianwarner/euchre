# Euchre

Single-player euchre against three AI opponents, playable in the browser. One
SvelteKit app — no separate backend to stand up.

## Running it locally

```sh
bun install
bun run dev
```

Open **http://localhost:5173** → *Deal a hand*.

With no `.env` at all you still get a full game:

- `RIVET_ENDPOINT` unset → RivetKit starts its own local engine on `:6420` and
  every match actor runs in-process against that.
- `OPENROUTER_API_KEY` unset → the three AI seats fall back to a heuristic
  player instead of calling an LLM. Add the key (see `.env.example`) to get
  real model-driven opponents.

Copy `.env.example` to `.env` and fill in only what you need; every variable
in it documents what it does and whether it's required.

## How the pieces fit

One SvelteKit app, one Vercel deployment, no second process:

- **Frontend** — SvelteKit (Svelte 5 runes) for routing/UI, [Threlte](https://threlte.xyz)
  (Three.js) for the 3D table, camera and card layout.
- **Realtime state** — [Rivet](https://rivet.dev) actors, mounted *inside this
  same deployment* at `/api/rivet/*` (`src/routes/api/rivet/[...all]/+server.ts`).
  There is no independent backend to deploy or scale — Rivet Cloud calls back
  into this one Vercel function.
  - `euchreTable` — the authoritative match: rules, turn order, scoring.
  - `aiSeat` — **one actor per AI opponent** (three per match), each with its
    own persona, memory and failure domain. A hung model call stalls one
    chair, never the table; the table's watchdog auto-plays for it.
  - `playerProfile` — durable per-user history/stats, keyed off an email you
    type in (see "What's not built" below — this is deliberately not an
    account system).
- **AI decisions** — [OpenRouter](https://openrouter.ai), model
  `google/gemini-3.6-flash` by default, one call per bid/play decision.
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

## Deploying to Vercel + Rivet Cloud

There is exactly one deployable: this SvelteKit app, built with
`@sveltejs/adapter-vercel` (`svelte.config.js`) and declared to Vercel with
`vercel.json`'s `{"framework": "sveltekit"}`. `bunx vite build` emits the Rivet
catch-all (`/api/rivet/[...all]`) as an ordinary Node serverless function
alongside every page — `vercel deploy` needs no extra configuration beyond
environment variables.

1. **Create a Rivet Cloud namespace** at the [Rivet dashboard](https://dashboard.rivet.dev)
   and grab both a secret (`sk_`) and publishable (`pk_`) API token
   (*Settings → Advanced → Cloud API Tokens*).
2. **Set environment variables on the Vercel project.** At minimum:
   `RIVET_ENDPOINT`, `RIVET_PUBLIC_ENDPOINT`, `ALLOWED_ORIGINS` (your deployed
   origin — connections are refused without it once `RIVET_ENDPOINT` is set),
   `EUCHRE_INTERNAL_TOKEN`, and `OPENROUTER_API_KEY`. Full list, with the
   reasoning and failure mode for each, is in `.env.example` — nothing here
   should be configured from memory instead of that file.
3. **Deploy** (`vercel deploy` or push to the connected Git repo).
4. **Point Rivet at the deployment.** In the Rivet dashboard, set the
   provider URL to your Vercel deployment's origin so Rivet Cloud's calls to
   `GET /api/rivet/start` land on it. If Vercel Deployment Protection is on,
   add a bypass secret and forward it as the `x-vercel-protection-bypass`
   header from Rivet's provider settings, or every such call gets a 401.

The browser always opens its WebSocket directly to Rivet Cloud; Rivet Cloud
then makes ordinary HTTPS requests back to `/api/rivet/*` on this deployment.
Nothing in this app ever terminates a WebSocket itself, which is why a
stock Vercel Node function is sufficient — no persistent-connection
infrastructure to run or scale.

## What's *not* built

- **No magic-link auth.** There's a `better-auth` + Resend scaffold under
  `src/lib/server/auth/**`, but it isn't wired into any route or the actor
  registry — it's inert. The actual identity mechanism for `/games` and
  `/settings` is deliberately lighter, at the user's request: you type an
  email, it's normalized and hashed into a stable id, and that id becomes
  your `playerProfile` key. Nothing verifies the email belongs to you — it's
  a nickname for "this browser's history," not an account, and the UI says
  so.
- **`/play` doesn't yet use that identity.** Creating and connecting to a
  match still runs on an M2-era placeholder: every match is owned by, and
  every stat recorded against, a single fixed local identity, in every
  environment — not the email-derived id above. A real per-player token for
  match ownership (`docs/01-ARCHITECTURE.md` §6.1 describes the intended
  shape) hasn't landed yet. Match ids are unguessable UUIDs, which is the
  only thing standing in for per-player auth today.
