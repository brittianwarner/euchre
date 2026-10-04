# Packages / rivetkit-svelte

**Parent:** [Root](../../AGENTS.md)

Official Svelte 5 adapter for RivetKit actors. It ships a small ref-counted framework bridge, with Svelte-first ergonomics for app-owned typed context, shared clients, and reactive actor handles.

---

## Workspace

|             |                                                                      |
| ----------- | -------------------------------------------------------------------- |
| Package     | `@rivetkit/svelte`                                                   |
| Scripts     | `build`, `check-types`, `test`                                       |
| Depends on  | `fast-deep-equal`, `esm-env`                      |
| Peer deps   | `rivetkit` ^2.3.23, `svelte` ^5.57.0                        |
| Dev deps    | `vitest`, `jsdom`, `@sveltejs/package`, `svelte-check`, `typescript` |
| Consumed by | `apps/web` and external SvelteKit apps                               |

The package-local framework bridge is derived from Apache-2.0
`@rivetkit/framework-base` and forwards `getParams` through `get()` and
`getOrCreate()`. Standalone installs do not depend on Layerr root patches.

The bridge's reactivity core is a minimal **imperative observable** (a
`Map` of per-actor entries with per-entry listener sets), replacing the
upstream `@tanstack/store` `Store`/`Derived`/`Effect` trio — this package is
Svelte-only, so the framework-agnostic store machinery bought nothing here.
Svelte's own reactive collections (`SvelteMap`, `createSubscriber`) are
deliberately *not* used for the registry: their reads register the
surrounding effect and their writes re-run it, but a framework push must
reach `applyState` → `$state` slots WITHOUT re-running the `useActor()` effect
(see "Never wrap effect contents..." and the closure-based rune state note
below). Plain objects are invisible to Svelte's dependency tracking, which is
exactly the contract the adapter needs. `createSubscriber` is used where a
reactive read is genuinely wanted: the connection inspector's `revision` /
`snapshot()`.

## Architecture

The package supports two primary shapes:

```text
Provider pattern:
  app-local module
    → createRivetContext()
    → layout calls context.set(...) or context.setup(...)
    → descendants call appLocalContext.get().useActor(...)

Shared-client pattern:
  app-local createClient(...)
    → createSharedRivetKit(() => client)
    → shared wrapper reused by ViewModels and provider setup
```

Reactive actor state is powered by package-local ref-counted subscriptions bridged into Svelte runes:

```text
useActor(opts | () => opts)
  → extract(MaybeGetter)
  → internal framework bridge getOrCreateActor()
  → $effect subscription
  → getter-backed object + stable recursive action proxies

createReactiveActor(opts)
  → construct proxy-only handle (no subscription yet)
  → mount() calls the internal framework bridge
  → manual subscription lifecycle; dispose() also releases active mounts
  → getter-backed object + stable recursive action proxies

warmUp(opts)
  → BROWSER guard (no-op during SSR)
  → dedup check (Set<string> keyed by length-prefixed name/key/input hash)
  → client Proxy accessor → getOrCreate(key)
  → handle.resolve() (single HTTP PUT, no WebSocket)
  → fire-and-forget (catch removes from dedup on failure)
  → preloadActor is a deprecated alias of warmUp

preConnect(opts)
  → BROWSER guard (returns inert { dispose } during SSR)
  → createReactiveActor(opts).mount() — opens a real WebSocket eagerly
  → returns { dispose() } the CALLER owns; dispose() unmounts + disposes
  → high-intent tier only; broad hover should use warmUp
```

## Structure

```text
packages/rivetkit-svelte/
├── LICENSE
├── package.json
├── tsconfig.json
├── AGENTS.md
├── README.md
└── src/
    ├── check-types/
    │   ├── noop.svelte                # Keeps svelte-check happy for the package workspace
    │   └── public-options.ts          # Compile-time public option/context contracts
    └── lib/
        ├── index.ts                   # Main barrel exports
        ├── rivetkit.svelte.ts         # createRivetKit, createReactiveActor, useActor, ActionDefaults
        ├── shared.svelte.ts           # createSharedRivetKit, withActorParams, createReactiveConnection
        ├── context.ts                 # createRivetContext
        ├── connection-health.svelte.ts # createConnectionHealth aggregate health
        ├── connection-inspector.svelte.ts # opt-in live registry of package-managed sockets
        ├── errors.ts                  # ActorError guards (isActorError, actorErrorCode/Message, getActionError)
        ├── internal/
        │   ├── framework-base.ts       # ref-counted store + getParams forwarding
        │   ├── types.ts               # Getter, MaybeGetter
        │   └── extract.ts             # extract(MaybeGetter)
        ├── testing/
        │   ├── index.ts
        │   └── test-helpers.svelte.ts
        └── __tests__/
            ├── action-middleware.test.ts # Action middleware interceptor tests
            ├── context.test.ts
            ├── warm-up.test.ts
            ├── reactive-actor.test.ts
            ├── reconnect.test.ts         # reconnect() zombie-socket swap (real framework core + fake zombie client)
            ├── framework-base-getparams.test.ts # runtime getParams forwarding contract
            ├── rivetkit-real-runes.test.ts # compiled Svelte dependency/teardown semantics
            ├── shared.test.ts
            ├── helpers.ts
            └── runes-shim.ts
```

## Public API

### Factory Functions

- `createRivetKit<Registry>(endpoint?, opts?)`
- `createRivetKitWithClient<Registry>(client, opts?)`

Both return `{ useActor, createReactiveActor, warmUp, preloadActor, preConnect, connectionInspector }`
(`preloadActor` is a deprecated alias of `warmUp`; `connectionInspector` is
`null` unless `SvelteRivetKitOptions.connectionInspector` is `true`).

### Context Helper

- `createRivetContext<Registry>()` — typed context helper with `set`, `get`, `has`, `setup`, `setupWithClient`

Apps are expected to create and own their own context instance. The package no longer exports default-context helpers.

### Shared-Client / Mixed-Mode Helpers

- `createSharedRivetKit<Registry>(getClient, opts?)` — lazily reuse one RivetKit wrapper for a shared client factory
- `withActorParams(base, params)` — merge actor options with static or reactive params
- `createReactiveConnection(source)` — bridge raw connection handling into reactive `connStatus` / `error` state

`createReactiveConnection` accepts raw `handle.connect()` sources. Keep its
connection binding typed to the broad `ActorConn<AnyActorDefinition>` surface.
Rivet's generic inference types `on` as an actor action (`never`), so cast
`conn.on` itself to an event-subscribe function before calling — a return-only
cast does not compile. The runtime behavior is event subscription; do not
route actor event subscriptions through action calls.
`disconnect()` retains event registrations for a later reconnect. Preserve the
published compatibility contract that `dispose()` is its reusable alias; do not
make it terminal without a semver-breaking release. Both settle waiters and
detach reactive state before awaiting transport teardown, and concurrent calls
share the current close promise so a slow or rejected close never leaves a stale
connection reusable.

### Action Middleware (`actionDefaults`)

`timeoutByAction` optionally supplies finite positive named-action deadlines;
invalid entries fall back to `timeout`. Deadlines settle the adapter's own
pending counters. Middleware forwards an AbortSignal through the raw SDK
`connection.action({ name, args, signal })`; timeout/disposal abort local
transport waiters, not server execution. Disposal now settles already-dispatched
actions through the configured error policy. Custom connections without raw
`action` retain method forwarding but only local cancellation is guaranteed.

Both `useActor` and `createReactiveActor` accept an `actionDefaults` option (also configurable at the client level via `SvelteRivetKitOptions`). When provided, every proxied action call is wrapped with built-in middleware:

- **Timeout** — configurable per-actor or per-client
- **Error capture** — errors captured to `lastActionError` reactive state (not thrown by default)
- **Loading tracking** — `isMutating`, `pendingActions` counters updated automatically
- **Connection guard** — rejects immediately if disconnected (configurable)
- **Lifecycle callbacks** — `onActionStart`, `onActionSuccess`, `onActionError`, `onActionSettled`

The guard checks both the current connection object and `connStatus`. A stale
connection object with `connStatus !== "connected"` is treated as unavailable.
Idle/connecting waits for `whenConnected` (capped at 30s) then dispatches —
first-paint actions after a token mint must not die on `ACTOR_NOT_YET_CONNECTED`.
A lost socket still fails immediately with `ACTOR_DISCONNECTED`. Guard errors
carry those codes so app ViewModels can distinguish retryable startup from a
lost connection without parsing the message.
Waiting through idle/connecting counts as an in-flight mutation. One timeout
deadline covers readiness plus action dispatch; local timeout does not cancel
actor-side work. Callback failures always settle counters and reject the
returned promise, regardless of `throwOnError`. Concurrent settlements update
latest-action error state only when they own the newest invocation.
Disposal or a true reactive `useActor()` identity change cancels a pending initial-connection
wait through the same failure policy: error/settled callbacks run,
`throwOnError: false` resolves `undefined`, and `throwOnError: true` rejects.
Same-hash option/token refresh retains the current state and waiter.

```typescript
const actor = rivet.createReactiveActor({
  name: "user",
  key: ["user", userId],
  actionDefaults: {
    timeout: 30_000,
    throwOnError: false, // default — errors captured reactively
    onActionError: (err, name) => console.error(name, err),
  },
});

// Direct action call — no manual wrapping needed
await actor.updateProfile({ name: "New" });

// Reactive tracking (all $state-backed)
actor.isMutating; // boolean
actor.pendingActions; // number
actor.lastActionError; // Error | null
actor.lastAction; // string | null
actor.resetActionState(); // clear error state
```

**Cascade:** Client-level `actionDefaults` are shallow-merged with actor-level overrides. Actor-level wins.

**Types:** `ActionDefaults`, `SvelteActorOptions`, and `SvelteRivetKitOptions` are exported from `@rivetkit/svelte`; context `setup` methods accept the full Svelte options type.

### Reactive Actor Primitives

- `useActor<ActorName>(opts: MaybeGetter<ActorOptions>)`
  - component initialization only
  - accepts static options or a getter thunk
  - returns getter-backed reactive metadata plus proxied actor methods
  - exposes `lastError` and `hasEverConnected` in addition to `connection`, `handle`, `connStatus`, `error`, `isConnected`, `hash`, `onEvent`
  - when `actionDefaults` provided: also exposes `isMutating`, `pendingActions`, `lastActionError`, `lastAction`, `resetActionState()`

- `createReactiveActor<ActorName>(opts)`
  - safe in modules and `.svelte.ts` classes; construction is side-effect-light and does not subscribe until `mount()`
  - manual lifecycle via `mount()` and `dispose()`
  - `dispose()` releases active refs, unsubscribes, cancels waiters, clears listeners, detaches the last connection/handle, and resets identity-scoped connection/action state
  - `reconnect()` forces a brand-new connection, disposing the current one even when it is a half-open "zombie" still reporting `connected` (NAT/LB idle cull); drives the framework core's `enabled` toggle (disable → dispose + `idle` → re-enable → create), re-runs `getParams` for a fresh token, and rebinds `onEvent()` listeners. Custom hashes receive options with lifecycle-only `enabled` removed. No-op if never mounted. A plain `dispose()` + `mount()` cannot replace a zombie — the core only creates from `idle`.
  - `onEvent()` rebinds listeners when the underlying connection changes
  - proxied actor methods are recursively cached per consumer, stay referentially stable across reconnects, resolve the current socket at invocation, support nested Rivet paths, and return `undefined` for `then` to prevent Promise assimilation
  - when `actionDefaults` provided: also exposes `isMutating`, `pendingActions`, `lastActionError`, `lastAction`, `resetActionState()`

### Actor Warm-up (two tiers)

`warmUp` (cheap, HTTP resolve) and `preConnect` (heavy, opens a WS) are the
two warming tiers. Never call this "prefetch" — that name is reserved for
SvelteKit route preloading (`preloadCode`/`preloadData`), a different concern.

- `warmUp<ActorName>(opts: WarmUpActorOptions)`
  - warms actor resolution via raw Rivet client `getOrCreate(key, opts).resolve()` — no WebSocket connection
  - forwards `createWithInput` (including `null`) and `createInRegion` to `getOrCreate`; `createInRegion` is Rivet datacenter selection for newly created actors only and does not move existing actors
  - supports `noCreate: true` via raw client `get(key).resolve()` when callers only want to resolve an existing actor
  - analogous to SvelteKit's `data-sveltekit-preload-data` for routes
  - deduplicates concurrent resolves for the same actor (name + key + `noCreate` + `createInRegion` + optional create input), then clears the in-flight key after success or failure so a later hover can wake an actor that slept again; the key uses a length-prefixed string format so compound keys containing separators do not collide without paying `JSON.stringify()` on the common no-input path, with a non-throwing reference fallback for cyclic/BigInt input
  - fire-and-forget: errors are silently caught; failed attempts removed from dedup set for retry
  - SSR-safe: no-ops when `BROWSER` is false (via `esm-env`)
  - intended for hover-based warming to eliminate cold-start latency
  - `preloadActor` is a deprecated alias kept for back-compat

- `preConnect<ActorName>(opts: ActorOptions)` → `{ dispose(): Promise<void> }`
  - opens (and keeps open) a real WebSocket eagerly via `createReactiveActor(opts).mount()`
  - the caller OWNS the lifecycle and MUST call `dispose()` (unmount + dispose) or the socket leaks
  - SSR-safe: returns an inert `{ dispose }` when `BROWSER` is false
  - high-intent tier only — broad hover should prefer `warmUp`

```typescript
// Warm a document actor on hover (HTTP resolve, no WS)
rivet.warmUp({ name: "document", key: ["doc", docId] });

// High-intent: open the WS ahead of time, then hand off or dispose
const handle = rivet.preConnect({ name: "document", key: ["doc", docId] });
await handle.dispose();
```

### Error Utilities

- `isActorError(err)` — RivetKit structural guard for modern `RivetError`, legacy `ActorError`, and serialized cross-realm shapes
- `actorErrorCode(err)` — machine-readable `.code`, only present for `ActorError`
- `actorErrorMessage(err)` — human-readable message for any `Error`
- `getActionError(handle)` — `{ message, code, isActorError } | null` extracted from a handle's `lastActionError`, for use where `BaseActorViewModel.actorErrorMessage` isn't available (e.g. lazy/secondary actor handles)
- `ActionErrorInfo` type

These let ViewModels branch on a failed action's error code without importing `rivetkit/client` directly. See `.claude/rules/error-handling.md` for the full ViewModel-side pattern.

### Connection Inspector (opt-in)

- `createConnectionInspector()` — live registry of distinct sockets opened by
  `useActor` / `createReactiveActor` / `preConnect`. `warmUp` (HTTP only) and
  `createReactiveConnection` (raw path) are not listed.
- Enable with `SvelteRivetKitOptions.connectionInspector: true`. Off by default
  so applyState stays a no-op in production.
- Snapshot fields: `name`, `key`, `hash`, `connStatus`, `hasConnection`. Never
  records `params`, `getParams`, tokens, or payloads.
- Distinct sockets are keyed by framework hash. Multiple consumers of the same
  hash share one row; unregistering one owner cannot drop another owner's row.
- `revision` is reactive — overlays re-read `snapshot()` inside `$derived`.

### Other Exports

- `createConnectionHealth<K>(getSources)`
- `createConnectionInspector()`
- `ConnectionInspector` / `ConnectionInspectorSample` / `ConnectionInspectorReport` types
- `extract()`
- `Getter<T>` / `MaybeGetter<T>`
- `WarmUpActorOptions` type (+ deprecated `PreloadActorOptions` alias)
- `PreConnectHandle` type
- `ActionDefaults` type
- `SvelteActorOptions` type
- `SvelteRivetKitOptions` type
- `createClient` re-export from `rivetkit/client`
- `ActorConnStatus`, `ActorOptions`, `AnyActorRegistry` types

### Testing Subpath

`@rivetkit/svelte/testing` exports:

- `testWithEffect(name, fn)`
- `effectRootScope(fn)`

## Design Notes

### App-Owned Typed Context

The preferred provider-level API is `createRivetContext()`. Each app should own a local context instance rather than depending on a package-global default context.

### Shared Client Ownership Is Explicit

The package does not hide the raw `rivetkit/client` model. Apps that want a single transport should own that client locally and wrap it with `createSharedRivetKit()`.

### App-Owned Auth

Auth stays outside the package. `withActorParams()` exists to make token/org/session params ergonomic without baking Better Auth, Layerr token refresh, or framework-specific session logic into the adapter.

### Familiar Svelte Conventions

- `Getter` / `MaybeGetter` follow the same ergonomic direction teams will recognize from Runed and Bits UI
- provider/shared-client setup and stable action dispatch map to TanStack Query's stable mutation/current-observer pattern
- stable action functions plus subscription-owned snapshots map to XState Svelte's `send`/snapshot split
- the adapter keeps its framework-neutral core small and package-local, matching urql's thin binding posture
- composable primitives are preferred over monolithic app-framework wrappers

### Closure-Based Rune State

`createReactiveActor()` uses closure-based `$state` instead of class-field state so Proxy forwarding works correctly. Svelte class-field runes compile to private fields, and private fields do not cooperate with JS `Proxy`.

### Action Middleware Architecture

The action interceptor is built from `ActionDefaults` and used by the stable recursive action proxy. Every proxied method call flows through the interceptor, which:

1. Increments `pendingActions` / sets `isMutating`, including initial connection wait
2. Checks connection guard (fail-fast if disconnected)
3. Races the action against timeout (if configured)
4. On success: clears `lastActionError`, fires `onActionSuccess`
5. On failure: captures error to `lastActionError`, fires `onActionError`. With `throwOnError: false` (default), resolves to `undefined` instead of rejecting.
6. Decrements `pendingActions` / clears `isMutating` before observer callbacks; callback exceptions cannot strand state

The interceptor is a closure that captures `$state` variables directly — same pattern as the existing connection state tracking. No class fields, no double-proxy.

Hot-path action dispatch also maintains non-reactive mirrors for connection status and pending action count. Proxied method calls can therefore run inside Svelte effects without subscribing that effect to mutation tracking state, and without paying a per-call `untrack()` wrapper.
Connection hash and `hasEverConnected` control flow also use nonreactive mirrors,
so framework subscription pushes cannot become accidental `useActor()` effect
dependencies.

### SSR Safety

- `useActor()` is SSR-safe because `$effect` is the browser-only lifecycle boundary
- `createReactiveActor()` can be constructed during SSR because it does not subscribe or connect until `mount()`; `mount()` still belongs in a browser lifecycle
- `warmUp()` is SSR-safe — guarded by `BROWSER` from `esm-env`; no-ops during SSR to avoid wasteful HTTP calls. `preConnect()` is likewise guarded and returns an inert `{ dispose }` during SSR.
- prefer app-local typed context and browser-owned singletons over request-time mutable globals in SvelteKit code

## Integration With apps/web

The web app now uses `apps/web/src/lib/client/actor-client.ts` for shared raw client and shared wrapper ownership.

That composition point owns:

- `getRivetClient()` — shared raw client, cached per `page.data.rivetPublicEndpoint`
- `getRivet()` — shared wrapper via `createSharedRivetKit()`
- `actorIdentityHash()` — framework hash function that excludes volatile `authToken` params so token refreshes do not fragment a single actor instance into duplicate WebSockets
- `connectionInspector: import.meta.env.DEV` — local-dev overlay only; `AppTopBar` mounts `ActorConnectionIndicator` behind `{#if dev}`

`BaseActorViewModel` consumes `getRivet()` and `withActorParams(...)`, so primary reactive actors and any lazy actor handles continue sharing one endpoint-scoped transport while Layerr-specific token refresh remains in app code. The web VM base also guards in-flight async token resolution with a connect generation so fast mount/unmount cycles cannot leave unowned WebSockets.

`BaseActorViewModel._createAndMount()` passes `actionDefaults` with `timeout: 60 minutes`, `throwOnError: false`, `guardConnection: true`, and callback bridges that sync the package's action tracking state to the ViewModel's reactive `isMutating` and `error` fields. Subclass ViewModels still use `callAction()` when invoking actor methods from effects to preserve the app's explicit untracked action boundary.

## Benchmarks

The package includes `bun run --filter @rivetkit/svelte bench` for connection-management hot paths: proxy state reads, proxied method reads/calls, 32-actor fan-out reads, construct/mount/unmount, subscription pushes, preload hashing, concurrent actions, and `whenConnected()` fast path. Treat results as comparative microbenchmarks, not absolute production latency.

## Verification Expectations

When changing this package, verify at minimum:

- `bun run --filter @rivetkit/svelte check-types`
- `bun run --filter @rivetkit/svelte test`
- `bun run --filter @rivetkit/svelte build` when public exports or generated declarations changed
- consumer typecheck for `apps/web` and any other in-repo Svelte consumer if package surface or inferred types changed

The test command must include `rivetkit-real-runes.test.ts`; the fast rune shim
does not model automatic dependency tracking or teardown scheduling.

## Related

[README.md](./README.md) | [Root AGENTS.md](../../AGENTS.md) | [Web App](../../apps/web/AGENTS.md) | [Real-Time Architecture](../../docs/architecture/Real-Time%20Architecture.md)

## Euchre vendoring

Synced from `/Users/brittianwarner/goods/layerr/packages/rivetkit-svelte` at
Layerr revision `186866bc9055dadf7b4f89b5d0783a8f386da843` (clean source directory).
The source and tests are preserved. A local Vitest configuration isolates adapter
tests from the consuming app and retains upstream Vitest 4 benchmark support.
This standalone copy exports generated
`dist` files and declares the Euchre runtime peer `rivetkit ^2.3.23`; no Layerr
workspace packages or package-manager patches are needed. Run package checks
from this directory with `npm run check-types`, `npm test`, and `npm run build`.
