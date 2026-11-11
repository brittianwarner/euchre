# Railway configuration

`.railway/railway.ts` describes the whole Railway project `euchre`: the `euchre` server (root `Dockerfile`), the private `rivet-engine` (3.1.0), the `rivet-data` volume, and every service variable. The runbook is [docs/DEPLOY.md](../docs/DEPLOY.md).

The file imports `railway/iac` from the `railway` package, a root devDependency. `bun install` at the repo root installs it.

## Commands

```bash
bunx railway config plan    # preview the changes; safe, changes nothing
railway config apply        # a human only, after reading the plan
```

## Rules

- Omitting a service, volume or variable here deletes it on apply.
- Secrets are `preserve()`, so Railway keeps the value it holds and the file never contains it. Set a new secret in Railway before the plan that first lists it.
- Agents edit this file and may run `railway config plan`. They never run `railway config apply`, `railway up`, `railway add` or `railway down`.
- Stop if the plan shows a Destroy line you did not expect.
- Do not add a per-service `railway.json`; this file manages every service.
- `railway config pull --include-variables` decrypts secrets into the file. Never run it here.

Type-check the file from the repo root (needs `bun install`):

```bash
bun apps/web/node_modules/typescript/bin/tsc --noEmit --strict --target es2022 --module preserve --moduleResolution bundler .railway/railway.ts
```
