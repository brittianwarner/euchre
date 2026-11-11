# `/play`

Browser entry for a live `euchreTable` match. The Rust server
(`apps/server`) owns the table actor; this route only creates a table and
connects to it.

| Path                    | Role                                                                                                         |
| ----------------------- | ------------------------------------------------------------------------------------------------------------ |
| `+page.ts`              | `POST /api/tables` with the house rules from `/new` (`?stick=&natural=`), then redirects to `/play/[gameId]` |
| `+layout.ts`            | `ssr = false` (the whole app is a static SPA; the Rivet client is browser-only)                              |
| `+layout.svelte`        | Provides the shared Rivet kit (`getRivet` / `rivetContext`)                                                  |
| `[gameId]/+page.ts`     | Passes `gameId` through                                                                                      |
| `[gameId]/+page.svelte` | Connects to `euchreTable` and renders the table                                                              |

## Flow

1. `/new` → **Deal me in**, or open `/play` directly.
2. `POST /api/tables` creates the table with the player's personas, sets the
   guest cookie when needed and records the table in the recent-tables cookie.
3. `[gameId]` asks `POST /api/table-session/{gameId}` for the actor id, the
   connect token and the gateway token (`src/lib/client/rivet.ts`), then opens
   the actor socket at `<origin>/rivet` (D3) with JSON encoding (D19).
4. Moves go through `submitMove`, the review gate through `reviewTrick`. The
   server drives the AI seats.

A reload asks for a fresh table session and resumes the same table.
