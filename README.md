# Euchre

Single-player euchre to 10 against three AI seats, built as one SvelteKit app with Rivet actors.

## Developing

```sh
bun install
bun run dev
```

Open [http://localhost:5173](http://localhost:5173) → **Deal a hand**.

Local play (no `RIVET_ENDPOINT`) uses:

- magic connection token `dev` / user `dev-user`
- heuristic AI inside `euchreTable` (no Anthropic key)
- fast table tempo for quicker matches

## Scripts

| Command | Purpose |
|---|---|
| `bun run dev` | Vite + local Rivet engine |
| `bun run test` | Vitest (engine suite) |
| `bun run scripts/e2e-play.ts` | Headless engine fuzz / narration |
| `bun run check` | `svelte-check` |

## Layout

| Path | Role |
|---|---|
| `src/lib/euchre` | Pure rules engine |
| `src/lib/actors/euchre-table` | Authoritative match actor |
| `src/lib/client` | Browser Rivet context |
| `src/lib/ui` | DOM play surface (M2) |
| `src/routes/play` | Create + play a match |
| `docs/` | Binding product / architecture specs |

See `docs/06-REVISED-ARCHITECTURE.md` for hosting and actor topology.
