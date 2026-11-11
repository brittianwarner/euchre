# 00 - Completeness critic

This review covers the eight port specs (01-08) and ARCHITECTURE.md, checked against the TS source at `/Users/brittianwarner/goods/euchre`. That source is read-only and was not modified.

How it was done:

- Every spec was read in full.
- All 192 files under `src/` and `server/` were listed. Each was mapped to the spec that owns its directory, and each mapping was confirmed by a path or stem mention in that spec.
- Facts the port depends on were spot-checked against the source with file:line citations. These cover engine determinism, wire shapes, tempo, action names, keys, defaults and copy.

Small factual errors were fixed in place. Each fix carries a `<!-- critic: ... -->` marker, and there are 33 markers across 01-08. ARCHITECTURE.md was not edited, because it outranks the specs. Its contradictions are listed below.

Precedence rule used throughout: code > ARCHITECTURE.md > specs.

---

## 1. Coverage table (file -> spec)

Legend:
- A spec number alone means the file is named by path in that spec.
- `*` means the spec covers the file by stem or by behaviour, not by path.
- `none` means no spec mentions the file.

### 1.1 `server/`

| Files | Spec | Notes |
|---|---|---|
| `server/index.ts`, `server/README.md` | 05 | Bun HTTP host. Becomes `apps/server` (axum). |
| `server/bootstrap-rivet-namespace.ts` + `.test.ts` | 05 | Namespace bootstrap at boot. |
| `server/public-origin.ts` + `.test.ts` | 05 | Origin policy. |
| `server/rivet-gateway-proxy.ts` + `.test.ts` | 05 | `/rivet` gateway proxy. |

### 1.2 `src/` root and routes

| Files | Spec | Notes |
|---|---|---|
| `src/env.ts` | 05* | Covered by the env table. |
| `src/app.css` | 06, 07 §10 | Tokens and fonts (DM Sans, Source Serif 4). |
| `src/app.d.ts`, `src/app.html` | none | SvelteKit boilerplate. `app.d.ts` is empty. The web app keeps both as is. |
| `src/demo.spec.ts`, `src/routes/page.svelte.spec.ts` | none | Template sample tests with no product behaviour. |
| `src/routes/+layout.svelte`, `+page.svelte`, `new/+page.svelte`, `games/+page.svelte`, `games/[matchId]/+page.svelte`, `play/+layout.svelte`, `play/[gameId]/+page.svelte`, `settings/+page.svelte`, `cardtest/+page.svelte` | 06 | |
| `src/routes/+layout.server.ts`, `+layout.ts`, `play/+layout.ts`, `play/[gameId]/+page.ts`, `cardtest/+page.ts`, `cardtest/README.md`, `play/README.md` | 05 | |
| `src/routes/games/+page.server.ts`, `games/[matchId]/+page.server.ts`, `play/+page.server.ts`, `play/[gameId]/+page.server.ts`, `settings/+page.server.ts`, `settings/presets.ts` | 06, 05 | |
| `src/routes/games/_lib/session.server.ts` | 05, 06* | Secret fallback uses `\|\|` (see risk 11). |
| `src/routes/api/rivet/[...all]/+server.ts`, `api/table-session/[gameId]/+server.ts` | 05 | |

### 1.3 `src/lib/` (engine, protocol, store)

| Files | Spec | Notes |
|---|---|---|
| `src/lib/euchre/{cards,deal,index,invariants,legal,optimistic,reduce,score,seats,trick,types}.ts` (11) | 01 | |
| `src/lib/euchre/__tests__/*.test.ts` (10) | 01 | Golden and parity sources. |
| `src/lib/protocol/{index,hand-review,pacing}.ts` | 02 | |
| `src/lib/index.ts` | 02, 01 | Barrel file. |
| `src/lib/game/table.svelte.ts` | 02, 06* | TableStore (client reducer). |
| `src/lib/game/table.test.ts` | 02*, 06* | |
| `src/lib/game/README.md` | none | Prose only. |

### 1.4 `src/lib/actors/`, `src/lib/ai/`, `src/lib/client/`, `src/lib/server/`

| Files | Spec | Notes |
|---|---|---|
| `actors/euchre-table/{index,queues,tempo,send-bounded}.ts` + 4 tests (`decision-delivery`, `review`, `send-bounded`, `state-proxy`) | 03 | |
| `actors/registry.ts` | 03, 05 | |
| `actors/ai-seat/{decide,difficulty,index,llm-bridge,memory,screen,types}.ts` | 04 | |
| `ai/*.ts` (27 files: banter, bid-rollouts, config, decide, decision-context, heuristic, index, jev, notation, partnership, play-rollouts, prompt, schema, screen, table-context, testing, types + 9 tests) | 04 | |
| `ai/model.ts` | 04* | Matched by stem only. |
| `actors/auth/{guest,verify}.ts`, `guest.test.ts` | 05, 03 | |
| `actors/player-profile/{auth,guards,index,repository,schema,settings,types}.ts` | 05 | |
| `actors/player-profile/protocol-constants.ts` | 05* | |
| `client/rivet.ts`, `client/README.md` | 05, 02, 06* | |
| `server/{guest,personas,recent-tables,rivet}.ts` | 05 | |
| `server/auth/{auth,crypto,email,env,gateway,internal-token,schema}.ts`, `server/db/{index,schema}.ts` | 05 §9 | Dead code (Better Auth, authStore, drizzle). Do not port. |
| `server/auth/auth-store.actor.ts`, `server/auth/migrations.ts` | none | Same dead tree. 05 line 18 and §9 cover all of `src/lib/server/auth/**` as "do not port". |

### 1.5 `src/lib/ui/`, `src/lib/three/`, assets

| Files | Spec | Notes |
|---|---|---|
| `ui/{BidPanel,CardRack,GameFeedback,GlobalNav,HandReview,Hint,RulesPanel,ScoreBoard,ScoreCards}.svelte`, `ui/onboarding/{GameAnnouncer,Walkthrough}.svelte`, `ui/table-presentation.ts` + test | 06 | |
| `ui/{TableStage,IdentityGate,PlayingAs}.svelte`, `ui/sound/{SoundToggle.svelte,sound.svelte.ts}` | 06* | |
| `ui/{TableStatusBar,TrickView,HandA11y,DiscardPanel}.svelte` | 06* (§20 item 1) | Not imported by the live page. Do not port. |
| `ui/card-art.ts` | none by path | Behaviour (`warmCardArt()`) is in 06, and 07 references the art cache. |
| `ui/sound/index.ts`, `ui/README.md` | none | A re-export file and prose. |
| `three/TableFelt3D.svelte`, `three/scene/{FeltSurface.svelte,feltTexture.ts}` | 06 | The live felt. |
| `three/{EuchreTable3D,TableScene}.svelte`, `three/cards/{Card.svelte,faces,suits}`, `three/layout/{Hand.svelte,layout,reducedMotion,tempo}`, `three/scene/{Felt,Table}.svelte`, `three/layout/__tests__/layout.test.ts` | 06* (§16) | Unmounted 3D scene. Do not port. |
| `three/cards/{cardTexture,shadowTexture}.ts`, `three/layout/{Kitty,OpponentHand,TrickPile}.svelte`, `three/layout/{cardMotion,index}.ts`, `three/layout/__tests__/{cardMotion,dealLifecycle}.test.ts`, `three/scene/{CameraRig,Lights,SeatAnchors}.svelte`, `three/scene/{breakpoints,index,seatLayout}.ts` | none by path | Same unmounted scene, covered by 06 §16 "do not port". |
| `src/lib/assets/favicon.svg` | 06 | |

### 1.6 Outside `src/` and `server/` (checked for completeness)

| Path | Spec |
|---|---|
| `patches/rivetkit@2.3.23.patch`, `vite-plugin-rivet-dev.ts`, `drizzle.config.ts` | 05 |
| `rivetkit-svelte/`, `Dockerfile` | 05, ARCHITECTURE |
| `scripts/*` | 01, 04, 05, 07, ARCHITECTURE |
| `static/art/**` | 06, 07, ARCHITECTURE |
| `assets/*.blend` | none (3D source files, not ported) |

**Coverage verdict:** every live file has an owner spec. Of the 192 files, 25 have no path mention. All 25 fall into one of these groups:
- dead code (`server/auth/**`)
- the unmounted 3D scene
- boilerplate or template tests
- README prose
- one re-export
- `card-art.ts`, whose behaviour is specified

---

## 2. Contradictions

Status: **fixed** (edited in the spec), **annotated** (a critic note names the conflict and the winner), or **open** (needs a decision).

| # | Conflict | Winner / status |
|---|---|---|
| 1 | Profile actor storage. 05 §10.3 said one KV key `b"state"` with a `parking_lot::Mutex`. 08 says `ctx.kv()` is deprecated, capped at 131072 bytes, and that SQLite should be used with a tokio Mutex lane. | 08. **Fixed** in 05. |
| 2 | ARCHITECTURE "Storage" (line 98): "`aiSeat` state is one SQL row or KV blob". 08 says not to use KV. | **Open.** Use one SQLite row. ARCHITECTURE should drop "or KV blob". |
| 3 | 03 §16.3 kept a `run` handler waiting on `abort_signal().cancelled()`. 08 §6 rule 6 and G35 say an active `run` blocks idle sleep. | 08. **Fixed**: no `run`, reconcile in `on_start`. |
| 4 | 03 §16.6 used `is_hibernatable()` to tell WebSocket conns from HTTP conns. 08 §12 says `can_hibernate_websocket` defaults to false. | 08. **Annotated.** There is still no verified discriminator. |
| 5 | Per-connection state has three designs: 03 uses `conn.inner().set_state` (from source, untested), 05 uses a `DashMap<conn_id,bool>`, and 08 uses a `HashMap<conn_id, ConnState>` or `set_state`. Conn ids change after wake. | **Open.** Pick one. The recommendation is to re-derive role from `ctx.conn().params()` on every action and keep only an in-memory presence map. |
| 6 | 06 §14.1 named `POST /tables/{id}/session`. 05 defines `POST /api/table-session/{gameId}`. | 05. **Fixed.** |
| 7 | 06 §6.1 quoted AI think floors of 250-450 / 900-1400 / 900-1800 ms, attributed to "04 §10". The real values in tempo.ts:54-94 are cut 700, bids 1900, discard 1100, play 1000, plus `fnv1a(turnId#pace) % 400`. | Code. **Fixed.** |
| 8 | Minimum desktop window: 06 §20.1 says 800x600; 07 verified a 720x520 `min_size`. | **Annotated, open.** 720x520 is the verified floor. |
| 9 | Rust client encoding: 05 (lines 808, 876) says `Cbor`; 07 §8.4 says `Json`. 08 verified all three encodings for the Rust client. | **Open.** Either works for Rust. JS clients must use `json` or strip `undefined` (G34). Recommend Json everywhere for one encoding and easier debugging. |
| 10 | ARCHITECTURE D14 says sound is "off by default, with the initial value from `TablePrefs.sound`". But `TablePrefs.sound` defaults to `true` (settings.ts:142-146), so the decision contradicts itself. | **Open.** |
| 11 | 07 §9 palette (felt `#285641`, radius 18) vs the shipped table: TableStage.svelte:1336-1342 makes the felt transparent, so what shows is `#17452f` with the canvas gradient `#245a49 -> #10382c` (06). | 06 / code. **Fixed** (annotated in 07). |
| 12 | 07 §10 and §16 bundle DM Sans only, but app.css:7 also loads Source Serif 4. | Code. **Fixed.** |
| 13 | 04 AI wire types vs 02 and code: `RankedMove.why` was optional, `AIDecision` carried `confidence`, `AiSayMsg` lacked `gameId`, and `DecideOutcome.source` used invented values. | 02 / code. **Fixed.** |
| 14 | 03 §3.2: status `'lobby'\|'active'\|'game_over'`, a non-null pending decision, and source `'model'`. Code (types.ts:72-85, 152, 379-388) has status `active\|complete\|abandoned`, a nullable decision, and source `llm\|fallback\|forced\|null`. | Code. **Fixed.** |
| 15 | 05 said presets fill blurb, prompt and dials. `applyPreset` also sets difficulty (settings/+page.svelte:75-85). | Code. **Fixed.** |
| 16 | 02 §5 implied the Rust client gets a structured `{group, code}` error. 08 G16 says the Rust client returns string errors only (use `parse_rivet_error`), and errors not marked public are masked to `core/internal_error`. | 08. **Fixed.** |
| 17 | 06 §8 and §20.3 left out `final: true` (chat) and `v` (sync), wrote MoveAck `code` as optional, and typed reviewTrick's return as `{queued: boolean}`. | 02 / code. **Fixed.** |
| 18 | 03 §16.4 names `SubmitMove{...}` as a named-field struct; 08 §14 uses the newtype `SubmitMove(SubmitMoveRequest)`. | 08. **Open, minor.** Use 08's newtype shape, which is in the shape matrix. |
| 19 | 07 §8.4 placeholder bridge calls `get_or_create("table")`. The real actor is `euchreTable` with key `["table", gameId]`, and must not be auto-created from the client. | 05 §11.4 item 7. **Annotated.** |
| 20 | 07 §16 copies card PNGs into `apps/desktop/assets/cards`. ARCHITECTURE embeds them with `include_bytes!` from `apps/web/static/art/cards`. | ARCHITECTURE. **Annotated.** |
| 21 | ARCHITECTURE §4 writes `schedule().after(dur, "name", &cbor_args)`; 08 uses `rivetkit::action::encode_positional(&args)`. | Same intent; 08's API name is the real one. No edit. |

---

## 3. Corrections made (33 markers)

| Spec | Location | Change | Source |
|---|---|---|---|
| 01 | §11.9 applyPassRound2 (line ~498) | "Four `next()` mints" changed to **three** (thrownIn, rotate, startHand). | reduce.ts:687-689 |
| 02 | turnId / fnv1a row (line ~327) | FNV-1a runs over UTF-16 code units (`charCodeAt`), not UTF-8 bytes. | deal.ts:27-34 |
| 02 | §5 errors (line ~339) | The Rust client has string errors only (`parse_rivet_error`, G16), and non-public errors mask to `core/internal_error`. | 08 G16 |
| 02 | §11 flow (line ~610) | `dealt` carries **8** packets, and only the receiver's own packets have cards. | deal.ts:171-181, 225-235 |
| 02 | §12.3 example (line ~706) | Example expanded to all 8 packets (counts 3,2,3,2,2,3,2,3), plus a note on `dealPlan` / `threeFirst`. | deal.ts:171-181 |
| 03 | §3.2 status (line ~109) | Changed to `active\|complete\|abandoned`. | types.ts:85 |
| 03 | §3.2 pending (line ~114) | `decision: LegalMoveId\|null`; `source: 'llm'\|'fallback'\|'forced'\|null`. | types.ts:152, 379-388 |
| 03 | §3.2 phases (line ~121) | Named the terminal `game_over` phase. | types.ts:72-82 |
| 03 | §16.3 (line ~676) | Replaced the `run` + abort-signal design with "no `run`, reconcile in `on_start`". | 08 §6 rule 6, G35 |
| 03 | §16.6 (line ~723) | `is_hibernatable()` is not a WebSocket test; discriminate by params. | 08 §12 |
| 03 | §18 (line ~761) | Marked which of Q1-Q10 are answered by 08 or ARCHITECTURE. Q6 and the WS discriminator stay open. | 08, ARCHITECTURE |
| 04 | `RankedMove` (line 49) | `why` is required. | protocol/index.ts:424-430 |
| 04 | `AIDecision` (line 51) | Fields are `{internalToken, gameId, seat, turnId, moveId, source, banter?, rationale?, latencyMs}`, with no `confidence`. | protocol/index.ts:397-537 |
| 04 | `AiSayMsg` (line 52) | Added `gameId` (from the envelope). | queues.ts:179-190 |
| 04 | `DecideOutcome` (line 58) | `source` uses `AIDecisionSource` (`llm\|fallback\|forced`). | types.ts:152 |
| 05 | presets (line 309) | Presets also set difficulty. | settings/+page.svelte:75-85 |
| 05 | §10.3 `c.state` (line 777) | Use an SQLite row plus a tokio Mutex lane, not a KV blob with `parking_lot`. | 08 §2, kv deprecation |
| 05 | §10.3 `c.db` (line 778) | Bind-parameter API marked verified. | 08 §2 |
| 05 | §10.3 `createConnState` (line 779) | `ctx.conn()` is Some for stateless HTTP calls and None only for scheduled dispatch. | 08 §1, G5, G24 |
| 05 | §10.3 `UserError` (line 781) | Verified: `rivet_error::RivetError` with `RivetErrorKind::Dynamic`, group `user`. | 08 §8 |
| 05 | §10.3 arg decoding (line 783) | Use the `(Value,)` tuple shape; a bare Value-first signature mis-binds. | 08 G32 |
| 06 | §6.1 (line ~396) | Corrected the AI think floors to the tempo.ts values. | tempo.ts:54-94 |
| 06 | §8 chat (line ~472) | Payload includes `final: true`; `chatDelta` noted. | protocol/index.ts:256-270 |
| 06 | §14.1 (line ~574) | `POST /api/table-session/{gameId}`. | 05 §4.2, §11.2 |
| 06 | §20.1 (line ~714) | Noted that 800x600 conflicts with the verified 720x520. | 07 §11 |
| 06 | §20.3 (line ~731) | Sync includes `v`; MoveAck failure is `{ok:false, code, legal}`; reviewTrick returns `{queued:true}`. | protocol/index.ts:221-296, 373-379; euchre-table/index.ts:1798 |
| 07 | §8.4 (line ~617) | Use `get("euchreTable", ["table", gameId], {params:{token}})` or `get_for_id`, never `get_or_create`. Never drop a `Client` clone (G26). Time out every call (G29). | 05 §11.4 item 7, 08 |
| 07 | §9 palette (line ~651) | The base `.felt` rules are overridden on the live table. | TableStage.svelte:500-511, 1336-1342 |
| 07 | §10 fonts (line ~679) | Bundle Source Serif 4 as well as DM Sans. | app.css:6-7 |
| 07 | §16 fonts (line ~995) | Fonts list updated. | app.css:7 |
| 07 | §16 cards (line ~994) | Embed from `apps/web/static/art/cards` (per ARCHITECTURE). | ARCHITECTURE "Assets" |
| 08 | §5 example (line ~406) | The aiSeat key has 4 segments: `["table", gameId, "seat", "1"\|"2"\|"3"]`. | ai-seat/index.ts:112, euchre-table/index.ts:766 |
| 08 | §14 action table (line ~910) | Added the `reviewTrick` row: an enum tagged by `action`, returning `{queued:true}`, with a JS interop test owed. | euchre-table/index.ts:1798-1827 |

Facts checked and found correct in the specs:
- **Engine:** the cut formula (`4 + fnv1a(seed#handNo#cut) % 16`), the `seed#handNo` shuffle seed, mulberry32, `cutterOf = (dealer+3)&3`, and `projectSteps` blanking other seats' cards.
- **Protocol:** `PROTOCOL_VERSION = 1`, `BANTER_MAX_CHARS 140`, `RATIONALE_MAX_CHARS 120`, and `ChatKind = 'call'|'banter'|'system'`.
- **Tempo:** trick resolve 3300 ms (3800 when it seals a euchre), hand score 2500, AI hard cap 4000, extended thinking at 2200, nudge 90 s, abandon 240 s, profile retry 30 s, reap 30 days, and the `EUCHRE_FAST_TEMPO` scaling.
- **Personas and prefs:** all three default personas are `expert`; `TablePrefs` defaults are pace normal, banter, rationale and sound on, reduce motion off.
- **Actor surface:** the hook and action line numbers in 03 and 08.
- **Client timings:** banter 6500, resync 10000, illegal-move flash 3200, deal 580/75, token refresh at `expiresAt - 10000`, action timeout 12000, snapshot timeout 4000, and dedupe caps 80/40/30.
- **Spoken lines:** the engine call strings ("Crossing the creek.", "Throw it in. Nobody wanted it.").

---

## 4. Remaining gaps

1. **WS vs HTTP connection discriminator** (03 Q1). It is unverified. Presence and `sync`-on-connect must not fire for the ephemeral connection of each stateless HTTP call, or every HTTP action causes a presence flap and a full sync push.
2. **Schedule retry on `Err`** (03 Q6). 08 shows schedules are persisted and at-most-once, but does not test what happens when the target action returns `Err`. The due-time ledger plus `on_start` reconcile must cover a lost fire.
3. **`conn.inner().set_state` persistence** across hibernation is from source only and untested. Avoid depending on it (see contradiction 5).
4. **`thinking {extended: true}`.** tempo.ts:81 defines `aiExtendedAtMs` 2200. 03 and 06 should state whether the Rust table emits the escalation or only the `on`/`off` edges. The UI escalation copy depends on it.
5. **Heuristic `why` strings and the banter system prompt.** 04 points to the TS source instead of transcribing them. Golden tests must assert them byte for byte, so the port needs a fixture extraction step (for example, dump from the TS tests).
6. **Font files are missing from both repos.** DM Sans and Source Serif 4 TTFs, plus a mono face, must be fetched (OFL) and licensed before the desktop build. They cannot be copied.
7. **Test parity inventory.** The 10 engine tests, the 4 table tests, the 9 AI tests and `table.test.ts` are named but not mapped one to one to Rust test names. The builder should port each `describe` as a Rust module so coverage stays auditable.
8. **The JS interop test for the `reviewTrick` tagged enum** and for `updateSettings` (a `Value` argument) is owed (08 §14, G32).
9. **ARCHITECTURE line 98 and D14** still carry contradictions 2 and 10. They need an owner edit, because the critic does not rewrite ARCHITECTURE.
10. **`snapshot` has no role check** in TS (euchre-table/index.ts:1829). D6 decides the hardening; 03 should state the Rust behaviour explicitly so it is not left to chance.
11. **The `RIVET_EXPOSE_ERRORS` policy is undecided.** Without it, clients see `core/internal_error` for every non-public error, so all protocol codes must use public `RivetError`s.

---

## 5. Top 15 porting risks (ranked)

1. **Concurrency lane and lost updates.**
   - rivetkit Rust 3.1.0 runs actions concurrently. Every state mutation needs one `tokio::sync::Mutex` lane, held across the read-modify-persist-emit sequence.
   - The lane must never be held across an actor-to-actor call or an LLM await. Doing so deadlocks until the 60 s action timeout.
2. **No queues and no `run` loop.**
   - Durability for the tempo gate, AI release, timeouts, nudge, abandon, flushProfile and reap must come from the persisted due-time ledger plus reconcile in `on_start`.
   - Schedules are at-most-once, and retry on `Err` is untested, so a lost fire can stall a table forever unless reconcile re-arms it.
3. **JSON wire parity.**
   - Several things must match the TS output exactly:
     - null vs absent (`Option` with `skip_serializing_if` vs `null`)
     - serde renames such as `bid_round_1`
     - the `t` step tags
     - number formatting, including the f64 `fenceNonce`
     - key order (`preserve_order`)
   - The TS web client and golden fixtures compare these shapes literally.
4. **JS-to-Rust argument binding.**
   - JS bare encoding sends `undefined` as `["$Undefined",0]`, which breaks `Option` and is masked as `core/internal_error` (G34).
   - Value-first action signatures mis-bind (G32).
   - Affected: `reviewTrick` (tagged enum), `updateSettings` (Value) and `submitMove` (optional `clientMoveId`).
5. **Connection identity.**
   - Stateless HTTP calls run `on_before_connect`, `on_connect` and `on_disconnect` on every call.
   - `ctx.emit` from `on_connect` does not reach the joiner.
   - `conn.send` in a stateless call is dropped silently.
   - Conn ids change after wake.
   - Presence, initial sync and role checks all depend on getting this right, and no WS discriminator is verified yet.
6. **Engine determinism.**
   - Several pieces must reproduce the TS results bit for bit:
     - FNV-1a over UTF-16 code units
     - mulberry32 with `u32` wrapping (`Math.imul` semantics)
     - the `toString(36)` turnId chain, three mints on a throw-in
     - cut index and dealPlan packet order
   - One mismatch breaks every golden replay and lets turnIds collide.
7. **UTF-16 string limits.**
   - `screenLine` and the banter and rationale caps (140/120) slice by UTF-16 code units in TS.
   - Rust `chars()` or byte slicing gives different lengths for emoji and astral characters, and can panic on a non-char boundary.
8. **Error masking and string-only client errors.**
   - Every protocol code (`forbidden`, `stale_turn`, `invalid_persona`, and so on) must be a public `RivetError` (`RivetErrorKind::Dynamic`, group `user`). Otherwise clients see `core/internal_error`.
   - The Rust desktop client gets only strings and must parse them with `parse_rivet_error`.
9. **rivetkit-client lifecycle.**
   - Dropping any `Client` clone kills reconnection (G26), and `ctx.client()` clones (G27).
   - Inline callbacks must not block (G28).
   - There is no per-call timeout, and RPCs hang across actor sleep (G29). Wrap every call in `tokio::time::timeout`.
   - `connect()` panics outside a tokio runtime, and dropping a `SubscriptionHandle` does not unsubscribe.
10. **Size limits.**
    - Arguments are capped at 64 KiB, and over-size WS messages are dropped silently.
    - Events and outputs are capped at 1 MiB. `sync` with many steps, `getHandReview` and the hand-review history grow with game length.
    - The 131072-byte KV cap is a further reason to stay on SQLite.
11. **Gateway and auth.**
    - There are two JWT families: the profile/table actor token and the gateway token.
    - The secret fallback diverges: `??` in player-profile/auth.ts:61-70 vs `||` in session.server.ts:52-59. An empty env var yields different secrets.
    - Native clients have no `Origin` header, so the origin policy must not reject them.
    - Gateway token placement is Option A or B (05 §12.2).
12. **AI latency budget.**
    - The actor ladder uses 2200/1400/400/300 ms (ai-seat/types.ts:100-109).
    - ai/config.ts records that a 2200 ms first attempt "aborted every real call", so the ladder fell to the heuristic every time. Together with the 4000 ms hard cap, the port may ship an LLM path that never wins.
    - Rollouts must run in `spawn_blocking` within budget, and detached dispatch must keep decision and banter ordering.
13. **Rune animation fidelity.**
    - `El::matched` flights carry radius, fill and opacity but no rotation.
    - There is no arc path and no rotateY flip.
    - Reduce motion is automatic.
    - The 2.2 s read hold and deal staggering must live in state logic, not CSS-like timing, or the table feels different from the web.
14. **Sleep and wake.**
    - In-memory state (conn map, lane contents, timers) is lost on sleep, and `create` reruns.
    - On macOS the dev engine can orphan actors.
    - The first request after boot may return 503 and needs retry.
    - Every wake must rebuild from SQLite and reconcile.
15. **Web retarget.**
    - The SvelteKit client moves from rivetkit JS 2.3.23 (patched) plus `rivetkit-svelte` to rivetkit JS 3.1.0 against Rust actors, plus adapter-static.
    - Cookies, the `/rivet` proxy, `throwOnError:false` semantics and every behaviour must stay the same, while the encoding must change to `json` (risk 4).

---

## 6. Open questions

1. Q6: does a scheduled action that returns `Err` retry, or is the fire lost? Does the design accept "lost, re-armed by reconcile"?
2. What is the verified way to tell a WebSocket conn from a stateless HTTP conn in 3.1.0? Params marker, transport field, or something else?
3. Is `conn.inner().set_state` persisted across hibernation, or should per-conn state be in-memory only?
4. Sound default: is it off (D14 text) or on (`TablePrefs.sound = true`)? Which one does the desktop read first?
5. Minimum desktop window: 720x520 (verified) or 800x600 (06)?
6. One encoding for every client: Json (recommended) or Cbor for Rust and Json for JS?
7. Does the Rust table emit `thinking {extended: true}` at 2200 ms, as TS defines it?
8. AI first-attempt budget: keep the actor's 2200 ms (and accept near-total heuristic fallback), or adopt ai/config.ts's 6500 ms, which needs the 4000 ms hard cap and think floors revisited?
9. aiSeat persistence: confirm one SQLite row, and strike "or KV blob" from ARCHITECTURE.
10. `snapshot` role hardening (D6): spell out the exact Rust check in 03.
11. Enable `RIVET_EXPOSE_ERRORS` in dev only, or never (relying on public errors)?
12. Persona seat names (D16): confirm the unified naming is reflected in 04 and 05 fixtures.
