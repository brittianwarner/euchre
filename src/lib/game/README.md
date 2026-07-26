# `$lib/game`

Client game state for the live table.

| File | Role |
|---|---|
| `table.svelte.ts` | `$state` class: `view`, sync, `submitMove`, resync |

The play page constructs `TableStore`, binds the Rivet `useActor` handle, and passes `store.view` into the DOM UI components under `$lib/ui`.
