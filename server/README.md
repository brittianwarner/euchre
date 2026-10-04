# Railway deployment

Project `euchre` (`8a81234f-6775-4d79-8bab-5db9da547d57`), production environment
`4e5631d6-9139-47e0-aaf3-2e41dad8d801`:

| Service | Role | Network/storage |
| --- | --- | --- |
| `euchre` | SvelteKit UI, Bun WebSocket gateway, normal Rivet envoy | Public HTTPS on port 3000 |
| `rivet-engine` | Rivet scheduling and durable actor storage | Private port 6420, health 6421, persistent `/data` volume |

The topology follows `~/goods/layerr/apps/rivet-engine` and its actor-host proxy.
The engine is pinned to `rivetdev/engine:2.3.23@sha256:e6d1cbfe85847bc7c3c6ed7e5b3dc6cf9aaed32e8e42d6778ac9423bd928f2bb`, matching RivetKit 2.3.23.
Keep one engine replica with its attached RocksDB volume. It has no public domain.
The app currently also has one replica, 60s overlap and 120s drain on deployment.

## Configuration

Engine: `PORT=6421`, `RIVET__FILE_SYSTEM__PATH=/data`, an independently generated
`RIVET__AUTH__ADMIN_TOKEN`, `RIVET__FEATURES__GUARD_GATEWAY_V3__MODE=on`, and
`RIVET__FEATURES__GUARD_GATEWAY_V3__PERCENTAGE=100`. Healthcheck: `/health`.

App: `NODE_ENV=production`, `PORT=3000`, `APP_URL=https://euchre.sh`,
`ORIGIN=https://euchre.sh`, and `ALLOWED_ORIGINS` containing the comma-separated
origins `https://euchre.sh`, `https://www.euchre.sh`, and the Railway preview origin;
`RIVET_ENDPOINT=http://rivet-engine.railway.internal:6420`,
`RIVET_NAMESPACE=euchre`, `RIVET_POOL=default`, `RIVET_PUBLIC_ENDPOINT=<origin>/rivet`,
`RIVET_TOKEN=${{rivet-engine.RIVET__AUTH__ADMIN_TOKEN}}`, `RIVET_ENGINE_SELFHOSTED=true`,
`RIVETKIT_RUNTIME_MODE=envoy`, `RIVET_INSPECTOR_DISABLE=1`, `RIVET_LOG_LEVEL=warn`,
and `RIVET_EXPOSE_ERRORS=0`. Supply independent random secrets of at least 32 characters
for `EUCHRE_ACTOR_JWT_SECRET` and `EUCHRE_INTERNAL_TOKEN`, and an `OPENROUTER_API_KEY`
for Jev opponents. Do not put secrets in URLs, source control, client code or logs.

Startup bootstraps the namespace and normal pool before accepting traffic. Actor
version upgrades drain at one actor per 60s; normal shutdown allows 90s to save
state. `/health` becomes ready only after the envoy has connected to the engine.

The relay only exposes existing Euchre table connections. The session endpoint
verifies the signed guest cookie and table ownership before minting a 15-minute
JWT with gateway access to that actor only. The browser renews credentials on
reconnect; each mounted game has its own token cache. Actor-level authentication
also verifies ownership. Administration, inspector, actor creation and profile
actors are not exposed through the relay.

The incoming host must match a configured origin. SvelteKit request URLs and
gateway discovery retain that origin, including on `www`, so origin checks and
host-only guest cookies agree. Keep each existing game on the hostname where it
was created. Adding a domain requires adding its HTTPS origin to the allowlist
and redeploying; it does not require sharing guest cookies between hosts.

## Release

`.railway/railway.ts` records the deployed service and volume graph using the
current Railway infrastructure format. Secrets use `preserve()` and stay in
Railway. Run `railway config plan` before infrastructure changes; preserve the
engine's volume and verify the intended project and environment.

Run checks and build before uploading from the repository root:

```sh
bun run check
bun run test:gateway
bun run build
railway up --project 8a81234f-6775-4d79-8bab-5db9da547d57 --environment 4e5631d6-9139-47e0-aaf3-2e41dad8d801 --service 71db2501-0a70-4eca-983e-acbb64479a59 --detach
```

Confirm Railway reports both services online, then test the public site: create
a game, play cards, reload the same game, and create another game without a full
page reload. Check that an unrelated guest cannot obtain a token for the first
game, and that `/rivet/actors` and `/api/rivet/start` return 404. Build/health
success alone does not prove WebSocket or AI behavior.
