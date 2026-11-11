# `$lib/client`

Browser-only Rivet wiring for the euchre app. The actors run in the Rust server
(`apps/server`); this folder only connects to them.

| File                | Role                                                                                                                                                                                                                                                     |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rivet.ts`          | `createGameRivet(gameId)`: one client per table, pointed at `<origin>/rivet` with JSON encoding. It asks `POST /api/table-session/{gameId}` for the actor id, the connect token and the gateway token, and refreshes them before they expire.            |
| `start-playing.ts`  | `startPlaying()`: the home page's Play now. Starts a table with the player's saved settings (`POST /api/tables`, as /new does, minting the guest cookie when needed) and opens `/play/<id>`. One function, so it can later route to another table build. |
| `table-registry.ts` | Type-only `TableRegistry`: the `euchreTable` connect params, events and the actions the play page calls. Nothing in it runs.                                                                                                                             |

The gateway token is sent in every mode, dev included (D3 Option A). The play
page passes `getParams` to `useActor`, so each connect or reconnect gets a fresh
connect token.
