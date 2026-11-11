# W4 notes from the W3 server review fixes

Written by lane S1-BOOT on 2026-11-11 while fixing the W3 server review. Each item is a change the W3 lanes could not make, because it lives in `apps/web/**`, `apps/desktop/**` or the deploy config. The server side of every item is already done. `reports/server-boot.md` "Review fixes" has the details.

## Web (`apps/web`)

### 1. Send the gateway token in every build (review P2, web-compat)

The Rust server has one gateway, `/rivet/**`, and it requires the table's gateway token in every mode, dev included (D3 Option A). The TS dev build ran its own serverless loop, so `apps/web/src/lib/client/rivet.ts` sends no `getToken` when `dev` is true. Against the Rust server, every dev-mode table connection is therefore refused.

Three things need to change in `createGameRivet`:

- The endpoint: `/api/rivet/**` is a plain 404 on the Rust server. Only `/api/rivet/metadata` answers, and its `clientEndpoint` is `<origin>/rivet`.
- The token: always pass `getToken`, and drop the `dev ?` branch and the `$app/env` import.
- The encoding: D19 sets `json`.

The exact change:

```ts
// apps/web/src/lib/client/rivet.ts
-import { dev } from '$app/env';
 import { createClient, createRivetKitWithClient } from '@rivetkit/svelte';
 ...
 	const client = createClient<typeof registry>({
-		endpoint: `${window.location.origin}/api/rivet`,
+		endpoint: `${window.location.origin}/rivet`,
+		encoding: 'json',
 		devtools: false,
 		gateway: { skipReadyWait: true },
-		...(dev
-			? {}
-			: {
-					getToken: async ({ forceRefresh }: { forceRefresh: boolean }) => {
-						const value = await credentials(forceRefresh);
-						if (!value.gatewayToken) throw new Error('Missing table gateway token');
-						return value.gatewayToken;
-					}
-				})
+		getToken: async ({ forceRefresh }: { forceRefresh: boolean }) => {
+			const value = await credentials(forceRefresh);
+			if (!value.gatewayToken) throw new Error('Missing table gateway token');
+			return value.gatewayToken;
+		}
 	});
```

`TableSession.gatewayToken` can become required (`gatewayToken: string`). The Rust `POST /api/table-session/{gameId}` always returns it, or answers `503`.

### 2. The recent-tables cookie no longer needs the page load (review P3, web-compat; nothing to change)

In TS, the `/play/[gameId]` server load called `rememberTable`. The Rust server serves the web build as a static SPA, so no `+page.server.ts` runs. `POST /api/table-session/{gameId}` now refreshes `euchre_recent_tables` (that game first) for cookie callers, which covers the same moment.

The web build must not count on `+page.server.ts` for this or for anything else. Bearer callers (the desktop) get no cookie.

### 3. `forgetEpisode` takes real episode ids (D30; nothing to change unless the client validates)

`DELETE /api/episodes/{episodeId}` and the actor now accept ids of the form `role.matchId.handNo.kind`, for example `verna.m1.0.bid`. They refuse the old single-segment pattern. Any client-side id check must allow the dots.

## Desktop (`apps/desktop`)

### 4. Watch `RunningServer::closed()`

When the spawned engine exits, or the actor runner stops, `closed()` resolves with a `ServerFailure` and `/health` turns `503` for good. The embedded backend should then do three things:

- show a plain message ("The game server stopped. Restart to keep playing.");
- call `shutdown().await`;
- offer a restart.

The process does not exit, because the desktop is not `run()`.

After an engine death, the old envoy keeps retrying its dead engine for its own stop budget, about 30 s, and logs `connection failed` at `error`. rivetkit 3.0.7 does not expose its handle, so nothing can stop it sooner. The e2e starts the next server while those retries still run, and it works, but that server uses another port. A restart on the same port inside that window is untested: wait for `shutdown()` to return, then about 30 s more, or accept the logged errors.

### 5. Map the new boot error

`BootError::EnginePortChanged { recorded, requested }` means the data dir's engine ran on another port. `ServerConfig::local(dir)` now reuses the recorded port by itself, so the desktop only sees this error if it calls `with_engine_port`. Do not call it for an existing data dir.

`EnginePortInUse` on the recorded port means another process holds it. Tell the user to close that process. Do not move to a new port.

## Deploy and operations

### 6. `NO_PROXY` for the actor client

Most loopback clients now ignore `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`:

- the server's engine admin client;
- `LocalEngine`'s probes;
- the gateway proxy and the token issuer.

The rivetkit-client 3.0.7 actor client builds `reqwest::Client::new()`, which still honors them, and it cannot be configured. Any environment that sets a proxy must also set `NO_PROXY=127.0.0.1,localhost,::1` (and the engine host, if external). Add this to 05 §6 and to the desktop launch notes.

### 7. Default log level

`euchre-server` now defaults to `info` with rivetkit's per-action and per-message targets held at `warn`, so a busy table no longer floods the log. The envoy's connection logs stay at `info`. `RUST_LOG=info` restores everything. No Railway change is needed.
