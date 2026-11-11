# Deploying Euchre

This runbook covers the Railway project `euchre`: what runs, the one-time move from the TS app to the Rust server, plan review, smoke tests and rollback.

Rules that always hold:

- `.railway/railway.ts` is the source of truth. Omitting a service, volume or variable deletes it on apply.
- Agents may read and edit `.railway/railway.ts` and may run `railway config plan`. Only a human runs `railway config apply` (after reading the plan) and `railway up` (sections 3.3 and 8). Nobody runs `railway add` or `railway down`, or deletes a service.
- The `euchre` service has no git source. Railway builds only what `railway up` uploads, so merging a branch deploys nothing. The `rivet-engine` service deploys its pinned image when `railway config apply` changes it.
- Secrets never go in git. In `railway.ts` they are `preserve()`, so Railway keeps the value it already holds.

## 1. What runs

| Service | Source | Network | Health | Notes |
|---|---|---|---|---|
| `euchre` | root `Dockerfile`, uploaded with `railway up`: the Rust binary `euchre-server`, the merged static web build at `/app/build/client` (Svelte plus the Rune wasm app, section 10); desktop downloads come from the `euchre-downloads` bucket (section 9) | public: `euchre.sh`, `www.euchre.sh` | `GET /health` on `PORT` 3000 | 1 replica, `overlapSeconds 60`, `drainingSeconds 120` |
| `rivet-engine` | `rivetdev/engine:3.1.0@sha256:fd679281d8a357d20081a6639933d1fcaecdc6fc087604f7a05ee0905a8db731` | private only: `rivet-engine.railway.internal:6420` | `GET /health` on the guard port 6420 | 1 replica, `overlapSeconds 0`, volume `rivet-data` at `/data` |

The Rust server reaches the engine at `RIVET_ENDPOINT` and creates its namespace and runner pool on boot. Browsers never reach the engine directly. They connect through the server's `/rivet/**` gateway with a short-lived gateway token.

The image runs as a non-root user (uid 10001) from `/app`. It has no secrets baked in, no `curl` and no Docker `HEALTHCHECK`: Railway runs its own healthcheck against `/health` on `PORT`. Its base images are pinned by digest in the `Dockerfile`.

## 2. Variables

### 2.1 `euchre` (names that `ServerConfig::from_env` reads)

| Variable | In `railway.ts` | Value |
|---|---|---|
| `NODE_ENV` | literal | `production`. This turns on the deployment guards: three strong, distinct secrets, the engine token and namespace, and `APP_URL` in `ALLOWED_ORIGINS`. |
| `PORT` | literal | `3000` |
| `EUCHRE_TRUST_FORWARDED_FOR` | literal | `1`. The Railway edge appends `X-Forwarded-For`; without this, every player shares one rate-limit bucket. |
| `APP_URL` | `preserve()` | `https://euchre.sh` |
| `ALLOWED_ORIGINS` | `preserve()` | Every origin that serves the app, comma-separated (for example `https://euchre.sh,https://www.euchre.sh`). Any other `Host` gets `421 Unknown host`. |
| `RIVET_ENDPOINT` | `preserve()` | `http://rivet-engine.railway.internal:6420`, with no userinfo |
| `RIVET_NAMESPACE` | `preserve()` | `euchre` |
| `RIVET_POOL` | `preserve()` | `default` |
| `RIVET_TOKEN` | `preserve()` | the engine admin token (reference `${{rivet-engine.RIVET__AUTH__ADMIN_TOKEN}}`) |
| `EUCHRE_ACTOR_JWT_SECRET` | `preserve()` | secret, at least 32 characters |
| `EUCHRE_INTERNAL_TOKEN` | `preserve()` | secret, at least 32 characters, distinct |
| `EUCHRE_IDENTITY_SECRET` | `preserve()` | **new** secret, at least 32 characters, distinct (section 3.1) |
| `OPENROUTER_API_KEY` | `preserve()` | AI decisions. Without it, the seats play the heuristic. |
| `EUCHRE_DOWNLOADS_S3_*` (five) | bucket references | desktop downloads from the `euchre-downloads` bucket (section 9.2) |

Optional variables, left unset: `RUST_LOG` (the default is `info` with actor and envoy noise turned down), `BUILD_NUMBER` (envoy version; the default is unix seconds at boot), `EUCHRE_TRUSTED_PROXIES`, `EUCHRE_JWT_ISSUER`, `OPENROUTER_MODEL_TALK`, `OPENROUTER_BASE_URL`, `ANTHROPIC_API_KEY` and `ANTHROPIC_BASE_URL`. Keep `EUCHRE_ALLOW_UNSIGNED_EMAIL` (it accepts the old plain-email cookie, D2) and `EUCHRE_FAST_TEMPO` (it shortens every pause, for testing) unset in production. Never set `RIVET_EXPOSE_ERRORS=1`: the server refuses to boot with it.

`NO_PROXY=127.0.0.1,localhost,::1` (plus the engine host, if external) is needed only where an HTTP proxy is set, such as some local machines. The rivetkit actor client honours `HTTP_PROXY` and cannot be configured otherwise. Railway sets no proxy.

### 2.2 `rivet-engine`

| Variable | Value | Why |
|---|---|---|
| `PORT` | `6420` (was `6421`) | Railway's healthcheck port. In 3.x the API peer (6421) binds to loopback and has no `/health`; the guard serves `/health`. |
| `RIVET__AUTH__ADMIN_TOKEN` | `preserve()` | required by 3.x |
| `RIVET__STORAGE__LOG_DIRECTORY` | `/data/v3-bucket/log` | the write log on the volume; it must exist before boot (the start command creates it) |
| `RIVET__OBJECT_STORAGE__PRIMARY__S3__BUCKET`, `__REGION`, `__ACCESS_KEY_ID`, `__SECRET_ACCESS_KEY` | `ref(rivetObjects, …)` | cold storage in the private `rivet-objects` bucket |
| `RIVET__OBJECT_STORAGE__PRIMARY__S3__ENDPOINT` | `https://${{rivet-objects.BUCKET}}.t3.storageapi.dev` | the S3 client has no addressing-style key, so the endpoint carries the bucket host |
| `AWS_VIRTUAL_HOSTED_STYLE_REQUEST` | `true` | read by the engine's S3 client (same as Layerr's engine) |
| `RIVET__OBJECT_STORAGE__PRIMARY__S3__PREFIX` | `euchre-v3/` | a new prefix means a fresh engine |
| `RIVET__STORAGE__BALANCING__ENABLED` | `false` | single node |
| `RIVET__STORAGE__AUTO_PROVISION_PARTITIONS` | `1` | single node |
| `RIVET__STORAGE__LOG_SHARD__MAX_DISK_BYTES` | `4294967296` (4 GiB) | fixed log budget; it must fit the free space on `/data` |
| `RIVET__STORAGE__COLD_STORE__DISK_CACHE_BYTES` | `0` | unset, the engine sizes a cold-read cache from free disk and refuses to boot on the 10 GB volume (4 GiB reserve + 4 GiB log); 0 reads cold data from the bucket |
| `RIVET__TELEMETRY__ENABLED` | `false` | |
| `RIVET__STORAGE__STOP_TIMEOUT_MS` | `80000` | graceful stop within Railway's 120 s drain (default 8 min) |
| `RIVET__STORAGE__STOP_TEARDOWN_MS` | `20000` | default 1 min |
| `RIVET__RUNTIME__GUARD_SHUTDOWN_DURATION` | `60` | seconds; default 9 min |
| `RIVET__RUNTIME__FORCE_SHUTDOWN_DURATION` | `110` | seconds; default 9 min 50 s. It must exceed stop plus teardown (100 s). |

The start command is `/bin/sh -c "mkdir -p /data/v3-bucket/log && exec /usr/bin/rivet-engine"`. The 3.x image's entrypoint is the engine itself; the 2.3 `start` subcommand is gone.

Moved to the bucket on 2026-11-11 with a fresh start: earlier engine 3 data (`/data/v3/log`, `/data/v3/objects`) stays on the volume, unread. To go back to it, restore `RIVET__STORAGE__LOG_DIRECTORY=/data/v3/log` and `RIVET__OBJECT_STORAGE__PRIMARY__FILE_SYSTEM__PATH=/data/v3/objects`, drop the S3 and `AWS_VIRTUAL_HOSTED_STYLE_REQUEST` variables, apply, then redeploy `euchre`. A fresh engine also needs a `euchre` redeploy: the server creates its namespace and runner pool only at boot (`created=true` in its log).

### 2.3 Every change on apply

New secret, which must be set by hand before the plan (section 3.1):

- `euchre`: `EUCHRE_IDENTITY_SECRET`

Removed from `euchre`. Each of these was read only by the TS server or by SvelteKit:

| Variable | Why it goes |
|---|---|
| `ORIGIN` | SvelteKit adapter-node only |
| `RIVETKIT_RUNTIME_MODE` | the Rust server always runs as an envoy (`serve_with_config`) |
| `RIVET_ENGINE_SELFHOSTED` | documentation only; always self-hosted |
| `RIVET_EXPOSE_ERRORS` | must stay unset (the production guard refuses `1`) |
| `RIVET_INSPECTOR_DISABLE` | not read by rivetkit 3.1.0 |
| `RIVET_LOG_LEVEL` | not read; logging is `RUST_LOG` |
| `RIVET_PUBLIC_ENDPOINT` | never served: browsers get `<origin>/rivet` from `/api/rivet/metadata`, using the request `Host`. The server only checks the variable for leaked tokens. |

Removed from `rivet-engine`. Engine 3.1.0 refuses to boot with any of these, with the error `failed to deserialize config: unknown field ...`:

| Variable | Why it goes |
|---|---|
| `RIVET__FEATURES__GUARD_GATEWAY_V3__MODE` | gateway v3 is the only gateway in 3.x; `features` is no longer a config key |
| `RIVET__FEATURES__GUARD_GATEWAY_V3__PERCENTAGE` | same |
| `RIVET__FILE_SYSTEM__PATH` | the 2.3 RocksDB driver is gone; replaced by the two storage paths above |

Added to `rivet-engine`: every row of section 2.2 except `PORT` and `RIVET__AUTH__ADMIN_TOKEN` (ten variables), plus the start command. `PORT` changes from `6421` to `6420`.

Added to `euchre`: `EUCHRE_TRUST_FORWARDED_FOR`. `NODE_ENV` and `PORT` become literals; if the live values are already `production` and `3000`, the plan shows no change for them.

## 3. One-time move from TS to Rust (engine 2.3.23 to 3.1.0)

### 3.1 Before the window

1. **Save the current variables.** In the Railway dashboard, open each service, then Variables, then Raw Editor. Copy both blocks into the team password manager, not git. Rollback needs these values, because the apply deletes the variables listed in section 2.3.
2. **Create `EUCHRE_IDENTITY_SECRET`.** Generate it with:

   ```sh
   openssl rand -base64 48 | tr -d '\n'
   ```

   Do not reuse either existing secret. Set it on the `euchre` service in the dashboard, under Variables, and do not deploy yet. The dashboard is the safest path. With the CLI, generate and set it in one command, so the value never reaches the screen or shell history:

   ```sh
   V=$(openssl rand -base64 48 | tr -d '\n') && railway variables --service euchre --skip-deploys --set "EUCHRE_IDENTITY_SECRET=$V"; unset V
   ```

   The value is still visible in the process list while that command runs, so do not run it on a shared machine. The TS server ignores the variable, so a redeploy would not break anything.
3. **Check the existing values** against the Rust guards:
   - `EUCHRE_ACTOR_JWT_SECRET` and `EUCHRE_INTERNAL_TOKEN` are each at least 32 characters, and all three secrets differ.
   - `APP_URL` is in `ALLOWED_ORIGINS`, and `ALLOWED_ORIGINS` lists `https://www.euchre.sh` too.
   - `RIVET_NAMESPACE` is set, and `RIVET_ENDPOINT` points at port 6420.
4. **Check the volume.** Open `rivet-data`, then Usage. The 2.3 data stays at the volume root, and engine 3 needs 4 GiB free for its log, plus room for cold objects. If more than about 5 GB is used, raise `sizeMB` in `railway.ts` first (for example to `20000`). Increasing it is a non-destructive online resize.
5. **Check the image builds** (section 7) from the commit you will deploy.

### 3.2 Engine data: same volume, fresh storage (D10)

Engine 3.x does not read 2.3 storage. Rivet's own path is a side-by-side migration (`migrate_from_v2`) into a separate cluster with its own storage. We do not migrate: D10 drops the TS tables, and the Rust actors could not read TS actor state anyway.

Railway allows one volume per service, and omitting the volume would delete it. So `rivet-data` stays mounted at `/data`. The 2.3 RocksDB files stay untouched at its root for rollback, and engine 3 writes only under `/data/v3`.

What players see: live TS games, recent tables and email-nickname history do not carry over. The first Rust boot creates the `euchre` namespace and `default` pool on the empty engine (`created=true` in the log).

Two rules for the life of this storage:

- **Never change the engine's guard port (6420) or add a topology config.** Engine 3 ties its token-signing ring to them. After such a change, every `POST /auth/tokens` fails, no browser can join a table, and only going back fixes it. To move ports, wipe `/data/v3` and start fresh.
- **Clean up the 2.3 files only after the rollback window.** Once nobody will roll back, delete everything at the volume root except `v3` from a shell on the engine service (`railway ssh --service rivet-engine`). Do this only on purpose and by hand.

### 3.3 The window

The TS app needs the 2.3 engine, and the Rust server needs 3.1.0, so the move is one outage. It starts when the config lands and ends when the Rust image is built and healthy, so it lasts about as long as the image build. Time the build in section 7 first. The `euchre` service has no git source, so the config and the code are two separate human steps. Run them in order, without pausing:

1. Merge the Rust branch. Then, at the repo root, check out the merged commit with a clean tree (`git status` shows nothing to commit), linked to project `euchre`, environment `production`.
2. Run `railway config plan` and review it with the checklist in section 4.
3. A human runs `railway config apply`. Both services redeploy: the engine on 3.1.0 with fresh storage, and `euchre` with the new variables. That `euchre` deploy is still the last TS upload, which cannot run against engine 3, so the site is down from here.
4. Straight away, a human uploads the Rust build from the same checkout:

   ```sh
   railway up --project 8a81234f-6775-4d79-8bab-5db9da547d57 --environment 4e5631d6-9139-47e0-aaf3-2e41dad8d801 --service 71db2501-0a70-4eca-983e-acbb64479a59 --detach
   ```

   These are the `euchre` project, `production` environment and `euchre` service ids, as in the TS release command. Railway builds the root `Dockerfile` and switches traffic once `/health` passes. Follow the build in the dashboard, under the `euchre` service's Deployments. If the build fails, the site stays down: fix it and upload again, or roll back (section 6).
5. Run the smoke tests in section 5.

## 4. Plan review checklist

Stop and fix the file if any line is not expected. Plan output redacts values, so a changed `PORT` shows as changed without its value.

- **`rivet-engine`:** the image changes to the 3.1.0 digest above, and the start command is added. `PORT` changes. Three variables are deleted and ten are added (section 2.3). Nothing else changes.
- **`euchre`:** seven variables are deleted and `EUCHRE_TRUST_FORWARDED_FOR` is added (section 2.3). `EUCHRE_IDENTITY_SECRET` is unchanged, because you set it in 3.1. If the plan shows it as new or empty, stop and set it.
- **`rivet-data`:** no change. A resize is fine if you raised `sizeMB`. A detach, delete or region change is not.
- **Domains:** `railway.ts` declares none, as before, and generated Railway domains never appear in it. If the plan shows a custom domain (`euchre.sh`, `www.euchre.sh`) being removed, stop. Add `domains: [{ domain: "euchre.sh", port: 3000 }, { domain: "www.euchre.sh", port: 3000 }]` to the `euchre` service and plan again.
- **Destroys:** there must be no other Destroy line. No service and no volume is destroyed.

## 5. Smoke tests

Wait until both services are healthy. Then check the logs:

- The engine log shows `resolved storage disk capacities` and no errors.
- The `euchre` log shows `engine namespace and runner pool ready namespace=euchre ... created=true`, then `euchre server ready`.

From any machine:

```sh
O=https://euchre.sh
curl -fsS $O/health; echo                                  # ok
curl -fsS $O/api/rivet/metadata; echo                      # {"clientEndpoint":"https://euchre.sh/rivet","clientNamespace":"euchre"}
T=$(curl -fsS -X POST -H "Origin: $O" $O/api/device/enroll | sed -E 's/.*"sessionToken":"([^"]+)".*/\1/')
G=$(curl -fsS -X POST -H "Authorization: Bearer $T" -H 'content-type: application/json' -d '{}' $O/api/tables | sed -E 's/.*"gameId":"([^"]+)".*/\1/')
echo "game $G"                                              # a uuid (201 Created)
curl -fsS -o /dev/null -w '%{http_code}\n' -X POST -H "Authorization: Bearer $T" \
  -H 'content-type: application/json' -d '{}' $O/api/table-session/$G   # 200: engine token minting works
curl -sk -o /dev/null -w '%{http_code}\n' -H 'Host: evil.example' $O/  # not 200: 421 from the server, or 404 from Railway's edge
curl -s -o /dev/null -w '%{http_code}\n' -X POST -H 'Origin: https://evil.example' $O/api/device/enroll  # 403
curl -s -o /dev/null -w '%{http_code}\n' $O/rivet/actors     # 404
```

The `table-session` call returns a gateway token. A 200 proves the engine's `POST /auth/tokens` works on this storage.

Then play in a browser at `https://euchre.sh`:

- Deal a hand, bid, and play at least one trick. All three AI seats act; with `OPENROUTER_API_KEY` set, they use Jev.
- Reload. The page returns to the same game.
- Start another game from the menu.
- Open `https://www.euchre.sh`. It loads and can start a game.

## 6. Rollback

A code rollback alone does not work: the TS app needs engine 2.3.23, its old variables, and its own image. Roll back all three together, config first:

1. Restore the previous `.railway/railway.ts` from git (the commit before the Rust move). The deleted variables no longer exist in Railway, so `preserve()` has nothing to keep. Replace these entries with literals, using the values you saved in 3.1:
   - `rivet-engine`: `PORT: "6421"`, `RIVET__FILE_SYSTEM__PATH: "/data"`, `RIVET__FEATURES__GUARD_GATEWAY_V3__MODE: "on"` and `RIVET__FEATURES__GUARD_GATEWAY_V3__PERCENTAGE: "100"`.
   - `euchre`: `ORIGIN` (same as `APP_URL`), `RIVETKIT_RUNTIME_MODE: "envoy"`, `RIVET_ENGINE_SELFHOSTED: "true"`, `RIVET_EXPOSE_ERRORS: "0"`, `RIVET_INSPECTOR_DISABLE: "1"`, `RIVET_LOG_LEVEL: "warn"` and `RIVET_PUBLIC_ENDPOINT` (`https://euchre.sh/rivet`).

   That file has no start command and none of the 3.x storage variables, so the apply removes them. 2.3 boots with its image default (`start`).
2. Run `railway config plan`. Expect the reverse of section 4: the engine image goes back to 2.3.23, the ten 3.x storage and runtime variables are deleted, and the TS variables are added. Then a human runs `railway config apply`. The engine comes back on 2.3; `euchre` redeploys the Rust image with the TS variables and stays down.
3. Bring back the TS image. Either redeploy the last TS deployment from the `euchre` service's Deployments list in the dashboard, which reuses its image, or check out the last TS commit in a separate worktree and run the step 4 `railway up` command from there. That rebuilds the TS root `Dockerfile` (Bun and SvelteKit adapter-node). Either way it runs with the variables set in step 2.
4. Revert the Rust merge on the main branch too, so a later `railway up` from it does not bring the Rust server back by accident.
5. Smoke test the TS app: `/health`, then a browser game.

The 2.3 data was never touched, so TS games and history come back as they were at the move. Anything played on the Rust deployment stays in `/data/v3`, and a later move forward uses it again.

`EUCHRE_IDENTITY_SECRET` and `EUCHRE_TRUST_FORWARDED_FOR` can stay. The TS app ignores them.

## 7. Building and checking the image locally

From the repo root, with Docker running, build the image and boot it against a throwaway 3.1.0 engine configured like Railway's. Everything here is named `euchre-local-*`, so cleanup removes only these.

```sh
docker build -t euchre-local .
docker network create euchre-local-net
ADMIN=$(openssl rand -hex 24)
docker run -d --name euchre-local-engine --network euchre-local-net \
  -e RIVET__AUTH__ADMIN_TOKEN=$ADMIN \
  -e RIVET__STORAGE__LOG_DIRECTORY=/data/v3/log \
  -e RIVET__OBJECT_STORAGE__PRIMARY__FILE_SYSTEM__PATH=/data/v3/objects \
  -e RIVET__STORAGE__BALANCING__ENABLED=false -e RIVET__STORAGE__AUTO_PROVISION_PARTITIONS=1 \
  -e RIVET__STORAGE__LOG_SHARD__MAX_DISK_BYTES=1073741824 \
  --entrypoint /bin/sh \
  rivetdev/engine:3.1.0@sha256:fd679281d8a357d20081a6639933d1fcaecdc6fc087604f7a05ee0905a8db731 \
  -c 'mkdir -p /data/v3/log /data/v3/objects && exec /usr/bin/rivet-engine'
docker run -d --name euchre-local-app --network euchre-local-net -p 3000:3000 \
  -e NODE_ENV=production -e APP_URL=http://localhost:3000 -e ALLOWED_ORIGINS=http://localhost:3000 \
  -e EUCHRE_ACTOR_JWT_SECRET=$(openssl rand -hex 24) -e EUCHRE_INTERNAL_TOKEN=$(openssl rand -hex 24) \
  -e EUCHRE_IDENTITY_SECRET=$(openssl rand -hex 24) \
  -e RIVET_ENDPOINT=http://euchre-local-engine:6420 -e RIVET_TOKEN=$ADMIN \
  -e RIVET_NAMESPACE=euchre -e RIVET_POOL=default \
  euchre-local
until curl -fsS http://localhost:3000/health; do sleep 2; done; echo   # ok, once the runner connects
docker logs euchre-local-app 2>&1 | grep -E 'namespace and runner pool ready|server ready'
```

Then run the section 5 commands with `O=http://localhost:3000`, and open `http://localhost:3000` in a browser to play a hand. Clean up with:

```sh
docker rm -f euchre-local-app euchre-local-engine && docker network rm euchre-local-net
```

The image build fails on purpose in two cases:

- The `channel` in `rust-toolchain.toml` differs from `ARG RUST_VERSION`.
- The web build has no `index.html`. The server needs the adapter-static SPA with fallback `index.html` (D4).

## 8. Releasing after the move

Every later release is a human upload of a clean, checked commit. Nothing deploys on merge.

1. Check out the commit to ship with a clean tree, then run from the repo root:

   ```sh
   bun install --frozen-lockfile
   bun run check
   bun run test
   bun run build
   ```

2. Desktop apps are released separately, straight to the bucket (section 9.4). A code release does not touch them.
3. Build and boot the image (section 7) if the `Dockerfile`, `.dockerignore` or a base image changed.
4. If the release changes variables or the engine, edit `.railway/railway.ts`, run `railway config plan`, and have a human run `railway config apply` first.
5. A human runs the section 3.3 step 4 `railway up` command from that checkout.
6. Run the smoke tests in section 5, and the download checks in section 9.7.

To undo a release, redeploy the previous deployment from the `euchre` service's Deployments list, or upload the previous commit the same way.

## 9. Desktop downloads

euchre.sh offers the Mac, Windows and Linux apps. The files live in the private Railway bucket `euchre-downloads`, not in the image. The server reads the release manifest from the bucket and sends each download to a short-lived presigned bucket URL, so a release needs no code deploy. The apps are unsigned for now.

### 9.1 Routes

| Route (`GET` and `HEAD`) | Answer |
|---|---|
| `/download/manifest.json` | `200` JSON in the manifest's shape (9.3), listing the platforms that pass the manifest rules. `Cache-Control: no-cache`, `ETag` (the body's sha256), and `304` on a matching `If-None-Match`. No manifest: `{"version":null,"platforms":{}}`. |
| `/download/mac`, `/download/windows`, `/download/linux` | `302` to a presigned bucket URL for `files/<file>`, valid for 1 hour, with `Cache-Control: no-store`. `404` when that platform is not listed. |
| `/download/files/<file>` | The same `302`, only for names in the current manifest. Anything else is `404`. |

The presigned URL tells the bucket to answer with `Content-Disposition: attachment; filename="<file>"` and the content type for the extension. The bucket sends the bytes and handles ranges and resumed downloads. A `HEAD` gets a URL signed for `HEAD`.

Other methods get `405`. Any other path under `/download/` gets a plain-text `404`, never the SPA page. `/download` itself, with no further path, is the SPA. The `Host` check (`421`), `nosniff` and `Referrer-Policy` apply here as everywhere.

Content types come from the extension: `.dmg` is `application/x-apple-diskimage`, `.zip` is `application/zip`, `.tar.gz` and `.tgz` are `application/gzip`, `.exe` is `application/vnd.microsoft.portable-executable`, and anything else (including `.AppImage`) is `application/octet-stream`.

The server's own per-network stream caps and its 1 GiB-an-hour budget apply only to disk mode (9.6). In bucket mode the bucket carries the bytes, and anyone holding a presigned link can use it for its hour.

### 9.2 Variables

The `euchre` service reads five variables. Their values come from the bucket's credentials (`railway bucket credentials -b euchre-downloads`).

| Variable | Value |
|---|---|
| `EUCHRE_DOWNLOADS_S3_BUCKET` | the bucket's name |
| `EUCHRE_DOWNLOADS_S3_ENDPOINT` | the S3 endpoint, for example `https://t3.storageapi.dev` (a host, no path) |
| `EUCHRE_DOWNLOADS_S3_REGION` | for example `auto` |
| `EUCHRE_DOWNLOADS_S3_ACCESS_KEY_ID` | access key ID |
| `EUCHRE_DOWNLOADS_S3_SECRET_ACCESS_KEY` | secret access key; never logged |

- **All five set:** bucket mode. The boot log says `desktop downloads: serving from the bucket`.
- **None set:** disk mode (9.6), for local runs and tests.
- **Some set, or a bad endpoint or bucket name:** the server still boots, logs one warning naming the variables (never their values), and offers no downloads: the empty manifest and `404`s.

The server uses virtual-host URLs: `https://<bucket>.<endpoint host>/<key>`.

### 9.3 Bucket layout and manifest

```
manifest.json
files/Euchre-0.1.0-mac.dmg
files/Euchre-0.1.0-windows.zip
files/Euchre-0.1.0-linux.AppImage
```

`manifest.json`:

```json
{
  "version": "0.1.0",
  "platforms": {
    "mac": { "file": "Euchre-0.1.0-mac.dmg", "bytes": 20971520, "sha256": "3dc7…9b90" },
    "windows": { "file": "Euchre-0.1.0-windows.zip", "bytes": 15728640, "sha256": "59fd…774a" },
    "linux": { "file": "Euchre-0.1.0-linux.AppImage", "bytes": 26214400, "sha256": "6f60…6b" }
  }
}
```

- **Platform keys:** exactly `mac`, `windows` and `linux`, and any of them may be missing.
- **`version`:** a semver such as `0.1.0` or `0.2.0-beta.1`.
- **`file`:** one plain file name: letters, digits, `.`, `_`, `-` and `+`, at most 128 bytes, not starting with `.`. The object is `files/<file>`.
- **`bytes`:** the file's size, a whole number above 0.
- **`sha256`:** 64 hex digits (`shasum -a 256 <file>`).

An entry that breaks a rule is left out, and the others are still served. A manifest over 64 KiB, or one that is not JSON or has a bad `version` or `platforms`, is refused whole.

The server fetches the manifest at boot, without holding up boot, and again about every 60 seconds. A fetch that fails (network, timeout, HTTP error, or a refused manifest) keeps the last good manifest and logs `desktop downloads: cannot fetch manifest.json from the bucket: …` once. A `404` means the bucket has no manifest, and no downloads are offered. Each change in what is served logs `desktop downloads loaded version=… platforms=[…]`.

Files are uploaded as cached for a year. Put the version in every file name, and never reuse a name for different bytes.

### 9.4 Releasing

```sh
scripts/release-desktop.sh --upload
```

The script builds the three apps into `downloads/` (gitignored), writes `downloads/manifest.json`, then runs `scripts/upload-downloads.ts`, which:

1. checks every file against the manifest (size and sha256),
2. uploads each file to `files/<file>`, skipping files already there at the same size,
3. uploads `manifest.json` last, so the site never names a file that is not there yet,
4. reads back each object's size.

To upload a folder that is already built: `railway bucket credentials -b euchre-downloads --json | bun run scripts/upload-downloads.ts downloads`.

The site serves the new release within about a minute, with no deploy and no restart. Before each release, keep the live manifest so you can roll back:

```sh
curl -fsS https://euchre.sh/download/manifest.json > manifest-<old version>.json
```

### 9.5 Rolling back a release

Old files stay in the bucket; nothing deletes them. To go back, upload the previous release's manifest as `manifest.json`:

```sh
railway bucket credentials -b euchre-downloads --json | bun -e '
const c = JSON.parse(await Bun.stdin.text());
const s3 = new Bun.S3Client({ accessKeyId: c.accessKeyId, secretAccessKey: c.secretAccessKey,
  endpoint: c.endpoint, bucket: c.bucketName, region: c.region });
await s3.write("manifest.json", await Bun.file(process.argv[1]).text(),
  { type: "application/json", headers: { "Cache-Control": "no-cache" } });
' manifest-<old version>.json
```

Every server picks it up within about a minute. Check it with 9.7.

### 9.6 Disk mode (local runs and tests)

With no `EUCHRE_DOWNLOADS_S3_*` variable set, the server reads the folder `EUCHRE_DOWNLOADS_DIR` (default `downloads`, relative to the working directory): the files and `manifest.json` side by side. It applies the rules in 9.3, and also drops a platform whose file is missing or whose size on disk differs from `bytes`. It reads the folder again at most every 5 seconds. `/download/{platform}` redirects to `/download/files/<file>`, and the server streams the file itself, with `ETag: "<sha256>"`, ranges, at most 8 streams per network and about 1 GiB an hour per network. The image has no downloads folder, so production always uses the bucket.

### 9.7 Checks after a release

```sh
O=https://euchre.sh
curl -fsS $O/download/manifest.json; echo            # the expected version and platforms
for p in mac windows linux; do curl -sS -o /dev/null -w "$p %{http_code}\n" $O/download/$p; done   # 302 each
curl -sSIL $O/download/mac | grep -iE '^(HTTP|content-length|content-disposition)'   # 302, then 200 from the bucket
curl -sSL -o /dev/null -w '%{http_code}\n' -H 'Range: bytes=0-99' $O/download/mac   # 206
curl -fsSL $O/download/mac | shasum -a 256           # matches the manifest
```

Then check the `euchre` log: one `desktop downloads loaded` line with the expected platforms, and no `desktop downloads:` warnings.

## 10. Two shells: Rune and Svelte

The image's `/app/build/client` holds two web apps. `apps/desktop/scripts/web-site.sh` merges them (Dockerfile stages `web`, `rune-web`, `site`).

| File | What | Served for |
|---|---|---|
| `index.html` | Svelte build (adapter-static fallback): the homepage and the classic table | `/`, `/index.html`; app routes when the browser is classic |
| `rune.html` | Rune loader (`apps/desktop/web/rune.html`, filled by `web-build.sh`) | `/new`, `/play`, `/play/<id>`, `/games`, `/games/<id>`, `/settings` and any other extensionless path, unless classic |
| `_rune/immutable/<hash>/` | `app.js`, `app_bg.wasm`, bridge snippet, Inter faces, with `.br`/`.gz` | one-year immutable cache |
| `_rune/immutable/art-<arthash>/` | card faces and hero, by absolute URL | one-year immutable cache |

`static_files.rs` picks the shell. Both shell responses are `no-cache` with `Vary: Cookie` (and `Accept-Encoding`). With no `rune.html` on disk (a build without Rune) every route gets `index.html`.

The `euchre_ui` cookie decides: a value starting with `classic` gets the Svelte page at the same URL.

| Value | Set by | Lifetime |
|---|---|---|
| `classic-auto` | the loader: no WebGPU, no or software adapter, game failed to start, no first frame in 10 s, device lost | 7 days |
| `classic-small` | the loader: viewport under 720x520 | session |
| `classic` | Rune Settings, "Classic table" | 400 days |

The loader sets the cookie and reloads once. If cookies are blocked, or a second automatic fallback happens in the same tab session, it swaps the Svelte page in place instead, so there is no reload loop. Play now creates the table, then does a full page navigation to `/play/<id>`, so the server picks the shell.

Checks after a deploy:

```sh
O=https://euchre.sh
curl -s $O/ | grep -c 'rune.html'                                         # 0: the homepage
curl -sI $O/games | grep -iE '^vary'                                      # Cookie
curl -s $O/games | grep -c '_rune/immutable/'                             # >= 1: the loader
curl -s -H 'Cookie: euchre_ui=classic' $O/games | grep -c '_app/immutable/'   # >= 1: Svelte
curl -sI -H 'Accept-Encoding: br' $O/_rune/immutable/<hash>/app_bg.wasm   # application/wasm, content-encoding: br
```

Rollback: redeploy the previous deployment. To turn Rune off without a deploy, nothing is needed server-side; a browser opts out with the `euchre_ui=classic` cookie.
