<!--
  Play routes — human vs three heuristic AIs (M2 local path).
-->

# `/play`

Browser entry for a live `euchreTable` match.

| Path | Role |
|---|---|
| `+page.server.ts` | Creates a table actor (`gameTo: 5` locally) and redirects to `/play/[gameId]` |
| `+layout.svelte` | Provides the shared Rivet kit (`getRivet` / `rivetContext`) |
| `+layout.ts` | `ssr = false` — Rivet client is browser-only |
| `[gameId]/+page.svelte` | Connects with magic token `dev`, renders bid/hand/trick UI |

## Local flow

1. Open `/` → **Deal a hand**, or go to `/play`.
2. Server creates `euchreTable` with `ownerUserId = dev-user`.
3. Client connects via `/api/rivet` metadata → WebSocket to the local engine (`:6420`).
4. Moves use `submitMove`; AI seats are decided in-table with `topMove` (no `aiSeat` actor yet).

Auth for local play is the magic token `dev` (see `src/lib/actors/auth/verify.ts`).
