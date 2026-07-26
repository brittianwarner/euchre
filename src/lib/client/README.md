# `$lib/client`

Browser-only Rivet wiring for the euchre app.

| File | Role |
|---|---|
| `rivet.ts` | `createRivetContext` + shared `createClient` pointed at `/api/rivet` |

The play layout calls `rivetContext.set(getRivet())`. Play pages use
`rivetContext.get().useActor(...)` with `withActorParams` for the local-dev
`token: 'dev'` (or a real JWT later).

After changing `file:./rivetkit-svelte`, run `bun install` again — Bun copies
that package into `node_modules`, so a rebuild alone is not enough.
