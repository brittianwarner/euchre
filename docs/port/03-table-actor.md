# 03 - Table actor (`euchreTable`) porting spec

Source of truth: `/Users/brittianwarner/goods/euchre/src/lib/actors/euchre-table/` (READ-ONLY).
`index.ts` (1954 lines), `queues.ts` (238), `tempo.ts` (112), `send-bounded.ts` (30), plus tests
`decision-delivery.test.ts`, `review.test.ts`, `send-bounded.test.ts`, `state-proxy.test.ts`.
All `index.ts:N` refs below are to that file unless stated. Where docs (`01-ARCHITECTURE.md`,
`06-REVISED-ARCHITECTURE.md`) disagree with code, **the code wins** (see section 15).

Rust target: `apps/server` on rivetkit Rust SDK 3.1.0. The Rust actor MUST keep: actor name
`euchreTable`, key `["table", gameId]`, action names, event names, and JSON payload shapes
(camelCase, `null` vs absent preserved).

---

## 1. Overview and security properties

The table is the single authority for one match. Human is seat 0 (`HUMAN_SEAT`), AI opponents are
seats 1-3, each backed by an `aiSeat` actor keyed `["table", gameId, "seat", "1"|"2"|"3"]`. The
durable per-user profile is the `playerProfile` actor keyed `["user", userId]`.

Properties that MUST survive the port:

1. **Single writer.** In TS every mutation is serialized by the `run` loop draining queues
   (`queues.ts:10-18`), except `submitMove`, which calls `onMove` inline (`index.ts:1861`). Rust has
   no queue handlers and runs actions concurrently, so a lane mutex replaces the loop (section 16).
2. **Redaction.** Only `sync` carries card identity; it is sent per-connection via `conn.send`
   after `project(game, seat)`. The other four events are broadcasts and structurally cannot hold a
   card (`queues.ts:222-231`).
3. **Internal token is the actor-to-actor boundary.** `tokenOk(game.internalToken, body.internalToken)`
   runs first, before any state read, on every internal message (`queues.ts:96-115`). `canPublish`
   is defence in depth only.
4. **No seat in client payloads.** The acting human seat is the constant `HUMAN_SEAT`; the verified
   conn seat is only checked (`queues.ts:121-129`).
5. **Server-held presentation pauses** (`trick_resolve`, `hand_score`) are a security property: the
   next card is not computed until the tempo gate fires (`tempo.ts:21-26`).
6. **Save before ack/fan-out.** State is persisted with `saveState({immediate:true})` before
   `pushSync`, before the move ack returns, before AI dispatch.
7. **Determinism.** No RNG outside the seeded engine; think-floor jitter is `fnv1a(turnId+'#pace')`.
8. **Stale-nonce rejection.** `turnId` is minted by the reducer and re-checked in `apply()`; every
   timer carries the `turnId` it was armed against.

---

## 2. Identity, create input, registry

| Item | Value | Ref |
|---|---|---|
| Actor name | `euchreTable` | registry.ts:66 |
| Key | `["table", gameId]`; `gameId = c.key[1] ?? "g-" + randomHex(8)` | index.ts:1585 |
| Create | server: `client.euchreTable.create(['table', gameId], {input})` (`routes/play/+page.server.ts`) | |
| Connect | browser: `get(['table', gameId])` with `params: {token}` | |

`TableCreateInput` (camelCase, all optional):

```
ownerUserId?: string      // '' until claimed
profileUserId?: string    // profile key owner if different from owner
seed?: string             // default randomHex(16)
cfg?: Partial<EngineConfig>
firstDealer?: Seat
personas?: PersonaAssignment[]   // 3 entries, seats 1..3
```

`/play` passes `cfg {gameTo:10, stickTheDealer, requireNaturalTrump}`.

`createState` (index.ts:1585-1617) builds:
`createGame({gameId, seed: input.seed ?? randomHex(16), internalToken: crypto.randomUUID(), cfg, firstDealer})`
which yields a `lobby` GameState, then wraps it in `TableState` (section 3).

`createVars` (index.ts:1618): `{ jwks: makeJwks(appUrl()) }`, rebuilt on every wake.

---

## 3. State shapes (`c.state`)

Persisted whole; there is NO `c.db` usage and the only `c.kv` key family is `review/{handNo}`.

### 3.1 TableState

```
schema: 1
matchId: string                 // = gameId
ownerUserId: string             // '' until first authenticated conn claims it
profileUserId?: string
startedAt: number               // ms
game: GameState
personas: PersonaAssignment[]   // 3
journal: PlayerAction[]         // <= 40, current hand only; reset at hand start
stats: MatchStats
handsPlayed: number
lastJournaledHandNo: number     // -1 initially
lastResetHandNo: number         // -1 initially
recordedAt: number | null
abandonStrikes: number          // legacy, unused
tempoId: string | null          // schedule ids
abandonId: string | null
profileRetryId: string | null
reapId: string | null
reviewTricks?: boolean          // per-match "pause after each trick" toggle
reviewReadyTurnId?: string | null
observations?: Observation[]    // <= 32, current hand only
turnOpened?: { turnId: string, at: number, connected: boolean }
```

`MatchStats = {euchresFor, euchresAgainst, lonersAttempted, lonersMade, marches, throwIns, tricks:[n,n]}`.

### 3.2 GameState (engine, `euchre/types.ts`)

`schema, v, gameId, internalToken, seed, cfg, firstDealer, status ('active'|'complete'|'abandoned'), <!-- critic: was 'lobby'|'active'|'game_over'; GameStatus is euchre/types.ts:85. 'lobby' and 'game_over' are GamePhase values, not statuses -->
score, handNo, misdealStreak, winnerTeam, hand, turnId, turnDeadlineAt, timerId, watchdogId,
revealId, pending, recentMoveIds (cap 32), appliedSeq`.

`pending: PendingDecision | null`:
`{seat, turnId, requestedAt, revealAt, decision: LegalMoveId | null, source: 'llm'|'fallback'|'forced' | null, answered?: boolean}`. <!-- critic: was decision non-null and source 'model'|...; euchre/types.ts:379-388 and AIDecisionSource at :152 -->

`HandState`: `handNo, phase, dealerSeat, turnSeat, deckOrder, cut, hands, kitty, upCard,
upCardTurnedDown, turnedDownSuit, trump, makerSeat, aloneSeat, sittingSeat, dealerDiscard, passes,
bids, trick, trickLog, tricksWon, result, delta`.

Phases used by the table: `lobby, deal, cutting, bid_round_1, dealer_discard, bid_round_2,
trick_play, trick_resolve, hand_score` (plus terminal `game_over`). <!-- critic: named the terminal phase; GamePhase is euchre/types.ts:72-82 -->

Engine API consumed (from the rules-engine crate): `apply(state, seat, move, {turnId})`,
`advance(state)`, `legalMoves(state, seat)`, `project(state, seat)`, `projectSteps(steps, seat)`,
`isRuleError`, `RuleError{code, legal}`, `createGame`, `fnv1a`. The reducer mints `turnId` as
`fnv1a(internalToken#turnId#a|b).toString(36)`-style; the initial one is
`fnv1a(seed#turn0).toString(36)` (`reduce.ts`). `v` increments only in `commit`.

### 3.3 c.vars

`{ jwks }` only (cached remote JWKS for `{APP_URL}/api/auth/jwks`). Dies on every migration; must be
rebuilt lazily. Never store durable data there.

### 3.4 c.kv / c.db

- `c.kv.put('review/{handNo}', JSON(CompletedHandReview))` at hand end (`archiveHand`, index.ts:1033).
  `getHandReview` reads and parses it.
- `c.db`: **not used** in code. Docs mention `journal`, `match_meta`, `steps:`, `talk:`; none exist.

### 3.5 CompletedHandReview

```
handNo, dealerSeat, tricksWon, upCard, trump, makerSeat, aloneSeat,
tricks: hand.trickLog,
buried: [...kitty.slice(1), dealerDiscard ?? (upCard if any)],   // 4 entries
unplayed: Object.values(hand.hands).flat(),
result, delta
```

---

## 4. Wire types

```
MoveAck = {ok:true, v:number} | {ok:false, code: RuleCode|ProtocolErrorCode, legal: LegalMove[]}
RuleCode = not_your_turn | wrong_phase | illegal_move | must_follow_suit | card_not_in_hand
         | bidding_closed | dealer_must_call | sitting_out | stale_turn
ProtocolErrorCode = origin_not_allowed | invalid_token | forbidden | seat_mismatch
         | internal_token_mismatch | game_not_found | game_complete | duplicate_move
         | rate_limited | protocol_version_mismatch | invalid_persona | ai_unavailable | internal_error
TableConnectParams = { token?: string, internalToken?: string, protocolVersion?: number, seat?: number }
TableConnState = { userId: string, seat: Seat, role: 'player'|'internal', since: number }
PROTOCOL_VERSION = 1
```

`UserError(code, {metadata})` is thrown for connection/action faults (client sees `group:"user"`).

### tableView (index.ts:453) = `project(game, seat)` plus

```
players: [{seat, name, blurb, style}]   // style: risk>=0.7 'Bold bidder' | risk<=0.3 'Card counter' | else 'Steady player'
observations: filtered to current handNo
reviewTricks: reviewTricks ?? false
rules: {stickTheDealer, requireNaturalTrump ?? false}
awaitingTrickReview: phase==='trick_resolve' && reviewReadyTurnId===turnId
```

---

## 5. Connection and auth

### 5.1 onBeforeConnect (index.ts:1628)

1. If `params.internalToken` and `tokenOk(game.internalToken, ...)` -> allow (internal callers).
   (Also accepts the profile/dev token path where applicable; see `profileInternalToken`.)
2. Else check Origin allowlist: env `ALLOWED_ORIGINS` (comma list). Empty list: allowed only when
   `RIVET_ENDPOINT` is unset (local dev); otherwise `UserError origin_not_allowed`. Origin not in
   list -> `origin_not_allowed`.

### 5.2 createConnState (index.ts:1659)

1. Valid `internalToken` -> `{userId:'internal', seat:0, role:'internal', since}`.
2. `params.protocolVersion` present and != 1 -> `protocol_version_mismatch`.
3. `params.seat` present and != 0 -> log + `seat_mismatch`.
4. `verifyPlayer(jwks, token, appUrl)`; failure -> `invalid_token`.
5. If `ownerUserId === ''`: first authenticated user claims it (`saveState({immediate:true})`);
   else if `userId !== ownerUserId` -> `forbidden`.
6. Return `{userId, seat:0, role:'player', since: Date.now()}`.

### 5.3 verifyPlayer (`actors/auth/verify.ts`)

- Reject empty token and the literal `'dev'`.
- Header alg HS256 -> guest token: `verifyGuest(token,'euchre-actors')`; issuer `euchre-guest`, aud
  `euchre-actors`, `kind==='guest'`, `sub` starts `guest-`. Secret from `EUCHRE_ACTOR_JWT_SECRET` /
  `BETTER_AUTH_SECRET` / `AUTH_SECRET` (>= 32 chars) with a local dev fallback.
- Otherwise `jwtVerify` against JWKS `{APP_URL}/api/auth/jwks`, issuer = APP_URL without trailing
  slash, audience `euchre-actors`, `clockTolerance` 5 s; claims `id || sub` and `email` required.
- `APP_URL = APP_URL ?? PUBLIC_APP_URL ?? 'http://localhost:5173'`.

### 5.4 tokenOk (queues.ts:107)

```ts
export function tokenOk(expected: string, got: unknown): boolean {
	if (!expected || typeof got !== 'string') return false;
	if (got.length !== expected.length) return false;
	let diff = 0;
	for (let i = 0; i < expected.length; i++) {
		diff |= expected.charCodeAt(i) ^ got.charCodeAt(i);
	}
	return diff === 0;
}
```

`profileInternalToken()` (index.ts:176): env `EUCHRE_INTERNAL_TOKEN` (>= 16 chars); dev fallback
`'euchre-dev-insecure-secret'` when `RIVET_ENDPOINT` unset; else `null`.

### 5.5 onConnect / onDisconnect (index.ts:1709/1717)

- onConnect (role `player` only): `conn.send('sync', {v, view: tableView(seat), steps: []})`,
  `broadcast('presence', {seat, online:true})`.
- onDisconnect: if `turnOpened` set `connected=false`; `broadcast('presence', {seat, online:false})`.
- Event `sync` has `canSubscribe: playersOnly`.

---

## 6. Events (exactly five)

| Event | Delivery | Payload |
|---|---|---|
| `sync` | per-conn `conn.send` | `{v, view: PublicGameView(+tableView extras), steps: projectSteps(steps, seat)}` |
| `thinking` | broadcast | `{seat, on, extended}`; `extended` always `false` in code |
| `chat` | broadcast | `{msgId, seat, kind:'call'|'banter'|'system', text, final:true}` |
| `chatDelta` | broadcast | `{msgId, seat, delta}` |
| `presence` | broadcast | `{seat, online}` |

`pushSync(c, steps)` (index.ts:475): for each player conn, `conn.send('sync', ...)`.
`emitChat` (index.ts:490): `screenLine(text,140)`, drop empty, broadcast `chat`.
`screenLine` (index.ts:424): non-string -> `''`; iterate by code point; code < 0x20 or == 0x7f -> space;
collapse `\s+` to one space; trim; `.slice(0, max)` counting **UTF-16 code units** (Rust must
emulate: truncate by UTF-16 units, not chars; take care not to split a surrogate pair inconsistently
with JS - JS would leave a lone surrogate; decide and document, see risks).

`emitScript` (index.ts:506): per step, `msgId = "{gameId}:{v}:{n}"` where `n` counts every step from 0.
- `bid` step -> `emitChat(seat,'call', step.say)`.
- `throwIn` -> `emitChat(dealerSeat,'system','Throw it in. Nobody wanted it.')` (uses post-transition dealerSeat).
- `handScored` with `makerSeat !== null`: `'Euchre!'` for euchre, `'March!'` for march/lone_march; kind `call`.

---

## 7. Actions

All TS actions run concurrently (not serialized by the framework). `c.conn.state` supplies identity.

| Action | Args | Returns | Auth | Mutates |
|---|---|---|---|---|
| `reviewTrick` (1798) | `{action:'set', enabled:boolean}` or `{action:'continue', turnId:string}` | `{queued:true}` | player only (`forbidden`); invalid -> `UserError internal_error` | yes (via `review` queue) |
| `snapshot` (1829) | none | `tableView(conn.seat)` | NONE (hole: internal conn gets seat-0 view) | no |
| `getHandReview` (1837) | `handNo:number` | `CompletedHandReview \| null` | player only | no |
| `lastTrick` (1844) | none | last `trickLog` entry or `null` | none | no |
| `submitMove` (1861) | `{moveId, turnId, clientMoveId}` | `MoveAck` | player, seat 0 | yes (inline `onMove`) |
| `onTempoGate` (1916) | `turnId` | void | none (!) | sends `tick` |
| `releaseAiMove` (1921) | `turnId` | void | none | sends `tick` |
| `onAiTimeout` (1926) | `turnId, seat` | void | none | sends `tick` |
| `onNudge` / `onAbandon` (1931/1936) | `turnId` | void | none | legacy no-ops (tick) |
| `flushProfile` (1941) | none | void | none | sends `tick` |
| `onReap` (1945) | none | void | none | sends `tick` |

`reviewTrick`: enqueues `review` `{internalToken, gameId, action, enabled? (set only), turnId}` where
turnId is `request.turnId` for continue, else current. Returns `{queued:true}`.

`getHandReview`: returns `null` if `handNo` not an integer, `< 0`, or `> game.handNo`; else reads kv
`review/{handNo}` and parses, or `null`.

### submitMove check order (index.ts:1861-1915) - port verbatim

1. role must be `player` and seat 0, else `UserError forbidden`.
2. `game.status !== 'active'` -> `UserError game_complete`.
3. `legalMoves(game, 0)` lookup of `moveId`; missing -> `UserError 'illegal_move'` with metadata `{legal}`.
4. `clientMoveId` must be a non-empty string else `internal_error`.
5. Build `MoveMsg {seq: appliedSeq+1, turnId: request.turnId, moveId, clientMoveId: slice(0,64), userId}`.
6. Call `onMove` inline; a `RuleError` becomes `{ok:false, code, legal}`.

HOLE: `turnId` is not type-checked, so omitting it bypasses the stale check. Rust MUST require a
string `turnId` (open question Q9).

Because the schedule-target actions are public, any client can invoke `onTempoGate` etc. and skip
the server-held pause. Rust closes this (section 16.5).

---

## 8. Queues (TS) and handlers

`tableQueues` (queues.ts:203):

| Queue | Message | Completes with | canPublish |
|---|---|---|---|
| `move` | `MoveMsg {seq, turnId, moveId, clientMoveId, userId}` | `MoveAck` | humanPlayerOnly |
| `review` | `ReviewMsg {internalToken, gameId, action:'set'\|'continue', enabled?, turnId}` | undefined | externalDenied |
| `aiDecision` | `AIDecision` | undefined | externalDenied |
| `aiSay` | `AiSayMsg {internalToken, gameId, seat, msgId, turnId, handNo?, delta?, text?, final}` | undefined | externalDenied |
| `tick` | `TickMsg {internalToken, gameId, kind, turnId, seat?}` | undefined | externalDenied |

`TickKind = tempo | releaseAi | aiTimeout | nudge | abandon | flushProfile | reap`.
`tick(game, kind, turnId)` (index.ts:1952) builds `{ internalToken: game.internalToken, gameId: game.gameId, kind, turnId }`.

Run loop (index.ts:1735-1795): `await reconcile(c)` once, then
`for await (const message of c.queue.iter({completable:true}))` with a switch on `message.name`:
- `move`: `onMove` -> `message.complete(ack)`; a `RuleError` completes `{ok:false, code, legal}`.
- `review`/`aiDecision`/`aiSay`/`tick`: run handler, `complete()`; a `RuleError` is logged and completed.
- any other error: rethrown **unacked** (message redelivered).

In practice `submitMove` bypasses the move queue (documented serverless deadlock workaround).

### 8.1 onMove (index.ts:1182)

1. `status !== 'active'` -> `{ok:false, code:'game_complete', legal:[]}`.
2. `seq <= appliedSeq` or `clientMoveId` in `recentMoveIds` -> `{ok:true, v}` (idempotent; checked
   BEFORE legality/turn checks).
3. Look up move in `legalMoves(game, 0)`; missing -> `{ok:false, code:'illegal_move', legal}`.
4. `applyMoveAsHuman` (1207): `apply(before, 0, move.move, {turnId: body.turnId})`; `commit` (v+1);
   `appliedSeq = seq`; `pending = null`; `observeMove`; `rememberMoveId`; `journal {seat,id,move,kind:'human'}`;
   `settle`; `saveState immediate`; `emitScript`; `pushSync`; `handBoundary`.
5. `armTurn`; return `{ok:true, v}`.

### 8.2 Core helpers

- `commit(c, state)` (541): `game = {...state, v: v+1}`. Only place `v` increments.
- `settle` (602): loop `advance` only while phase is `lobby` or `deal`. `trick_resolve` and
  `hand_score` are held by timers.
- `journal` (546): push, keep last 40.
- `rememberMoveId` (583): ring of 32.
- `cancelTurnSchedules` (614): cancel `timerId, watchdogId, revealId, tempoId, abandonId`, null them.
- `armTurn` (646):
  1. cancel turn schedules.
  2. status not active -> `recordMatchIfNeeded`; return.
  3. phase `trick_resolve` && `reviewTricks` && `reviewReadyTurnId === turnId` -> return, no timer.
  4. `trick_resolve` or `hand_score` -> `schedule.after(handScoreMs | trickResolveMs(seals), 'onTempoGate', turnId)`;
     `seals` = a `trickWon` step with `sealsEuchre`; store `tempoId`; save immediate.
  5. else `turnSeat`; null -> return.
  6. set `turnOpened {turnId, at: now, connected: any player conn}` if a new turn.
  7. human seat -> `armHumanLadder` (686): `turnDeadlineAt=null, timerId=null, abandonId=null`, save. No timer; waits forever.
  8. AI seat -> `dispatchAi`.
- `applyMove` (912, AI/auto path): `apply(before, seat, move.move, {turnId: before.turnId})`, commit,
  `pending=null`, `observeMove`, journal `{seat,id,move,...attribution}`, settle, save, thinking-off
  broadcast for AI seats, `emitScript`, `pushSync`, `handBoundary`.
- `autoPlay(seat, reason)` (945): plays `legalMoves[0]` with `{kind:'auto', reason}`, then `armTurn`.
- `observeMove` (551): `kind = decisionKindFor(phase)`; `elapsed = now - turnOpened.at` if `turnOpened.turnId===before.turnId`;
  reliable iff human && `turnOpened.connected` && `0<=elapsed<=20000`; action label: move label for
  play/pass/orderUp/call, `'Dealer discarded a private card'` for discard, `'Cut decision'` for cut;
  `elapsedMs = reliable ? round(elapsed/250)*250 : null`; timing `'human-observed'|'unreliable'` for human,
  `'computer-paced'` for AI; observations filtered to current hand, last 32.
- `decisionKindFor` (362): `cutting->cut, bid_round_1->bid1, dealer_discard->discard, bid_round_2->bid2, trick_play->play`.

### 8.3 handBoundary (index.ts:976)

- phase `hand_score` and `lastJournaledHandNo !== handNo`: `archiveHand`, set `lastJournaledHandNo`,
  `handsPlayed++`, `stats = accumulateStats`, `recordHand`, `notifyAiSeats('handEnd', handNo)`.
- else if a `throwIn` step occurred and not yet journaled for `before.hand.handNo`: archive `before.hand`,
  `stats.throwIns++`, `recordHand` (handsPlayed NOT incremented).
- phase `cutting` and `lastResetHandNo !== handNo`: set `lastResetHandNo`, `journal=[]`,
  `notifyAiSeats('resetHand', handNo)`.
- then save immediate.

`accumulateStats` (398; human team = team 0): `euchresFor` +1 if euchre and makerTeam 1;
`euchresAgainst` +1 if euchre and makerTeam 0; `lonersAttempted` +1 if `aloneSeat !== null`;
`lonersMade` +1 if lone and result lone_point/lone_march; `marches` +1 for march/lone_march;
`throwIns` +1 for throw_in; `tricks` add `tricksWon`.

### 8.4 onAiDecision (index.ts:1248) - validation order

1. `tokenOk` fail -> drop, log `internal_token_mismatch`.
2. `gameId` mismatch -> drop.
3. status not active -> drop.
4. `body.turnId !== game.turnId` -> drop (stale).
5. `seat = hand.turnSeat` must be an AI seat.
6. `body.seat === seat` else log `seat_mismatch`, drop.
7. `pending` exists, `pending.turnId` matches, not `answered`/`forced`.
8. `now >= (turnDeadlineAt ?? requestedAt + aiHardCapMs)` -> `releasePending(true)`; return.
9. Re-derive `legalMoves`; chosen = legal id match else fallback `legal[0]` (logged).
10. `source = chosen ? body.source : 'fallback'`.
11. If `banter` is a string: `emitChat(seat,'banter',banter,"{gameId}:{turnId}:{seat}")`.
12. `pending.decision/source/answered = true`; log `'AI decision accepted'`.
13. If `revealAt - now > 0`: cancel watchdog, `schedule.after(wait,'releaseAiMove',turnId)`, save, return; else `releasePending()`.

### 8.5 releasePending(force) (index.ts:1312)

No-op if no pending, turnId differs, or `decision` is null. Unless forced, keep waiting while the model
is outstanding (not answered, source not forced, persona exists, `now < deadline`). If `turnSeat !==
pending.seat`: clear pending, save. Else play `legal.find(id===decision) ?? legal[0]` with
`{kind:'ai', source: pending.source ?? 'fallback'}`, then `armTurn`.

### 8.6 onAiSay (index.ts:1348)

`tokenOk`, `gameId`; seat must be AI. If `handNo` undefined require `turnId===game.turnId`, else require
`handNo === hand.handNo`. `final` -> `emitChat(seat,'banter',text,msgId)`. Delta -> `screenLine(delta,140)`,
non-empty -> broadcast `chatDelta {msgId, seat, delta}`.

### 8.7 onReview (index.ts:1383) / advanceTempo (1369)

`onReview` needs tokenOk, gameId, status active.
- `set` with boolean `enabled`: set `reviewTricks`; `commit(game)` (v+1); save; `pushSync([])`; if disabled
  and `reviewReadyTurnId===turnId` -> `advanceTempo`.
- `continue`: requires `body.turnId===turnId`, `reviewReadyTurnId===turnId`, phase `trick_resolve`; then `advanceTempo`.

`advanceTempo`: `advance(before)`; unchanged -> return; `reviewReadyTurnId=null`; commit; settle; save;
`emitScript`; `pushSync`; `handBoundary`; `armTurn`.

### 8.8 onTick (index.ts:1408) order

1. `tokenOk` (drop+log) and `gameId` check.
2. `flushProfile`: clear `profileRetryId`, `recordMatchIfNeeded`.
3. `reap`: clear `reapId`; if `recordedAt` null -> re-schedule `onReap` and save; else `c.destroy()`.
   (Both are match-scoped; exempt from the turnId check.)
4. Drop stale `turnId` or non-active status.
5. Switch:
   - `tempo`: clear `tempoId`; if `trick_resolve` && `reviewTricks`: `reviewReadyTurnId=turnId`, commit, save,
     `pushSync([])`, return; else `advanceTempo`.
   - `releaseAi`: clear `revealId`; `releasePending()`.
   - `aiTimeout`: clear `watchdogId`; ignore if seat non-AI or != turnSeat; broadcast thinking off;
     if pending has a decision for this turn -> `releasePending(true)` else `autoPlay(seat,'ai_timeout')`.
   - `nudge`/`abandon`: legacy; clear `timerId`/`abandonId`, save.

### 8.9 reconcile (index.ts:1496), runs at start of `run` on every wake

- status not active -> `recordMatchIfNeeded`; return.
- pending for this turn with a decision:
  - not answered && not forced && persona exists: `remaining = deadline - now`; `<=0` -> `releasePending(true)`;
    else re-arm watchdog (`onAiTimeout`) and save; return.
  - else `wait = revealAt - now`; `<=0` -> `releasePending()`; else re-arm `releaseAiMove` and save; return.
- else: `settle([])`, save, `emitScript`+`pushSync` if steps, `handBoundary`, `armTurn`.

---

## 9. Tempo constants (`tempo.ts`)

`TRICK_PRESENTATION_MS = 700 + 1500 + 780 + 3*45 + 185 = 3300` (`protocol/pacing.ts`).

| Constant | Normal | Fast (`EUCHRE_FAST_TEMPO=1`) |
|---|---|---|
| trickResolveMs | 3300 | 120 |
| trickResolveSealsEuchreMs | 3800 | 180 |
| handScoreMs | 2500 | 250 |
| thinkFloorMs cut/bid1/bid2/discard/play | 700/1900/1900/1100/1000 | 80/120/120/120/100 |
| thinkJitterMs | 400 | 40 |
| aiHardCapMs (safety cap) | 4000 | 1500 |
| aiExtendedAtMs (unused) | 2200 | 800 |
| nudgeMs (legacy) | 90000 | 30000 |
| abandonMs (legacy) | 240000 | 90000 |
| profileRetryMs | 30000 | 30000 |
| reapMs | 30 days | 30 days |

`thinkFloorMs(kind, turnId) = floor[kind] + fnv1a(turnId + '#pace') % thinkJitterMs`.
`fnv1a`: offset `0x811c9dc5`, per UTF-16 unit `charCodeAt`: `h ^= c; h = Math.imul(h, 0x01000193) >>> 0`.
Rust must hash UTF-16 code units of the turnId (turnIds are base36 ASCII so bytes are equivalent, but
keep the UTF-16 iteration for any non-ASCII input).

---

## 10. AI dispatch protocol (table to aiSeat to table)

### 10.1 dispatchAi (index.ts:789)

1. `kind` from phase (`decisionKindFor`); `legal = legalMoves(game, seat)`; empty -> log error, return.
2. `revealAt = now + thinkFloorMs(kind, turnId)`; `view = tableView(seat)`; `forced = legal.length === 1`.
3. `fallbackId = forced ? legal[0].id : (topMove(view, view.hand, legal) ?? legal[0].id)`.
4. Set:
```
game.pending = { seat, turnId, requestedAt: now, revealAt, decision: fallbackId,
                 source: forced ? 'forced' : 'fallback', answered: forced }
```
5. `deadlineAt = now + aiHardCapMs`; `turnDeadlineAt = deadlineAt`.
6. `watchdogId = schedule.after(aiHardCapMs, 'onAiTimeout', turnId, seat)`;
   `revealId = schedule.after(max(0, revealAt - now), 'releaseAiMove', turnId)`.
7. Save immediate; broadcast `thinking {seat, on:true, extended:false}`.
8. If forced: return. If no persona handle: log warning, keep heuristic fallback.
9. Else `sendBounded(handle, 'decide', req, c.abortSignal, max(1, deadlineAt - now))`, catching errors.

Request (`AIDecideRequest`): `{internalToken, gameId, seat, turnId, kind, view, legal, deadlineAt}`.
`ranking` is never sent. The `view` is the redacted `tableView(seat)` for that AI seat only.

### 10.2 aiSeat handle and create input

`aiSeatHandle` (763): `client.aiSeat.getOrCreate(['table', gameId, 'seat', String(seat)], {createWithInput})`;
null if no persona for the seat. `AiSeatCreateInput` JSON:

```
{ gameId, seat, internalToken, fenceNonce, persona, dossier, matchId,
  profileKey?: ['user', profileUserId ?? ownerUserId] }   // profileKey only when ownerUserId !== ''
```

`fenceNonce(matchId)` (index.ts:706):
`h = fnv1a32(matchId); h.toString(16).padStart(8,'0') + ((h * 2654435761) >>> 0).toString(16).padStart(8,'0')`.
The product `h * 2654435761` exceeds 2^53 in JS (float), so exact parity needs f64 emulation:
`((h as f64) * 2654435761.0) % 4294967296.0` cast to u32 (JS `>>>0` is mod 2^32 of the truncated
double). Result is 16 hex chars even though `PERSONA_FENCE_NONCE_CHARS` is 12 - copy the
function, do not "fix" it.

### 10.3 Reply

The aiSeat, after deciding, sends `aiDecision` (a queue message in TS) back to the table:
`AIDecision {internalToken, gameId, seat, turnId, moveId, source, banter?}`; and `aiSay`
(`AiSayMsg`) for streamed chatter. Validation: section 8.4 and 8.6.

### 10.4 Lifecycle notifications

`notifyAiSeats(kind, handNo?)` (865): for seats 1-3 with a persona,
`sendBounded(handle, kind, {internalToken, gameId, kind, handNo?})`, with `kind` in
`resetHand | handEnd | gameEnd`. Failure is logged and skipped.

### 10.5 sendBounded (send-bounded.ts) - verbatim

```ts
export async function sendBounded(handle, name, body, lifecycle: AbortSignal|undefined, budgetMs = 1000) {
	const controller = new AbortController();
	let timer; let onAbort = () => {};
	const stopped = new Promise<never>((_, reject) => {
		onAbort = () => { controller.abort(); reject(new Error('Actor queue delivery interrupted')); };
		if (lifecycle?.aborted) onAbort();
		else lifecycle?.addEventListener('abort', onAbort, { once: true });
		timer = setTimeout(onAbort, budgetMs);
	});
	try {
		if (lifecycle?.aborted) await stopped;
		await Promise.race([handle.send(name, body, { signal: controller.signal }), stopped]);
	} finally { clearTimeout(timer); lifecycle?.removeEventListener('abort', onAbort); }
}
```

Semantics: race delivery vs timeout vs actor-lifecycle abort; on timeout/abort abort the controller and
reject with `Actor queue delivery interrupted`; always clear timer and listener; send nothing if already
aborted. Reason: queue delivery can park behind a cold actor; the watchdog must never block on it.
Rust: `tokio::time::timeout` + `CancellationToken` (`ctx.abort_signal()`) via `tokio::select!`.

---

## 11. Profile recording

- `recordHand` (1054): `RecordHandsMessage {internalToken: profileToken, matchId, hands:[HandJournalEntry]}`
  to `playerProfile.recordHands`. Skipped if no owner or no token. `HandJournalEntry {matchId, handNo, seed,
  dealerSeat, deckOrder, moves: journal, trump, makerSeat, aloneSeat, tricksWon, result, delta, endedAt}`.
  Best effort: failure logged, entry dropped.
- `outcomeOf` (1092): `winnerTeam 0 -> 'won'`, `1 -> 'lost'`, else `'abandoned'`.
- `recordMatchIfNeeded` (1107): once, guarded by `recordedAt`; skipped if `ownerUserId === ''`.
  Builds `MatchRecord {schema:1, matchId, userId: profileUserId ?? owner, seed, cfg, firstDealer, startedAt,
  endedAt, status, outcome, score, winnerTeam, handsPlayed, personas, stats, tokens: zeros}`.
  - No profile handle or token: `recordedAt = now`, `notifyAiSeats('gameEnd')`, save, return.
  - Else `sendBounded(profile,'recordGame',{internalToken, record})`; success -> `recordedAt = now`,
    `notifyAiSeats('gameEnd')`.
  - Failure: log, if `profileRetryId` null `schedule.after(profileRetryMs,'flushProfile')`, save, return.
  - Then, if `reapId` null: `reapId = schedule.after(reapMs,'onReap')`; save.

---

## 12. Match lifecycle walk-through

1. **Create** (`lobby`): server creates actor with input; browser connects (`get` + `params.token`); first
   authenticated conn claims `ownerUserId`; `onConnect` sends first `sync`.
2. **Start / first reconcile**: `run` -> `reconcile` -> `settle([])` advances `lobby -> deal -> cutting`; `handBoundary`
   resets journal (`lastResetHandNo`), notifies seats `resetHand`; `armTurn`.
3. **Cut / bid1 / discard / bid2 / play**: AI seats via `dispatchAi`; human via `submitMove`.
4. **Trick complete -> `trick_resolve`**: tempo gate `onTempoGate` after 3300/3800 ms; if `reviewTricks`, the tick
   sets `reviewReadyTurnId` and waits for `reviewTrick{continue}`.
5. **Hand complete -> `hand_score`**: `handBoundary` archives review, journals, stats, `handEnd`; tempo gate 2500 ms;
   `advance` deals the next hand (back to `cutting`) or ends the game (`game_over`).
6. **Throw-in** (all pass twice, no stick-the-dealer): hand archived via the throwIn branch.
7. **Game over**: `armTurn` -> `recordMatchIfNeeded`; schedules `onReap` (30 d) once recorded; `onReap` calls `c.destroy()`.

---

## 13. Durability ordering rules (must keep)

- `commit -> settle -> saveState(immediate) -> emitScript -> pushSync -> handBoundary -> armTurn -> ack`.
- Timers are armed AFTER state is saved, and their ids are stored in state, so reconcile can re-arm.
- Dispatch to the aiSeat occurs AFTER `pending` is saved, so a crash re-derives from `pending` in reconcile.
- Queue message completes only after the handler (and its save) returned; error => redelivery (TS).
- Idempotency: `recentMoveIds` (32) + `appliedSeq`; a retried move returns `{ok:true, v}`.

---

## 14. Tests to port as acceptance tests

- `decision-delivery.test.ts` (7 cases): AI decision acceptance, stale turn drop, seat mismatch drop, token drop,
  reveal-wait park, deadline fallback, forced decision.
- `review.test.ts` (11 cases): set/continue toggling, stale turn, wrong phase, v bump, tempo with review on/off,
  forged internal token ignored, status guards.
- `send-bounded.test.ts` (3 cases): timeout abort, lifecycle abort, success clears timer/listener.
- `state-proxy.test.ts`: rivetkit TS-only proxy behaviour; drop.
- Add: forged-publish test per `01-ARCHITECTURE.md` 3.2 (removing `tokenOk` must make a forged call succeed),
  plus the Rust-specific tests in section 16.8.

---

## 15. Doc vs code discrepancies (code wins)

`01-ARCHITECTURE.md` describes things the code does not do: a `startHand` queue; a `meter` action;
`onTurnOpen`; `c.db` journal + `match_meta`; kv `steps:` / `talk:`; 10-minute reap (code: 30 days); the 90 s / 240 s
human nudge/abandon ladder (code: humans wait forever, `armHumanLadder` arms nothing); abandonment after two
strikes (`abandonStrikes` is legacy); "kv not used" (retracted by doc 06; code uses `review/{n}`); node
`timingSafeEqual` (code: hand-rolled XOR). Doc 06 additions that ARE real: serverless via `registry.handler()`,
`c.vars` dying on migration, personas stored in state (not `match_meta`), `c.kv` allowed.

---

## 16. Mapping to the rivetkit Rust SDK (3.1.0)

Grounded in `rivetkit-3.1.0/src/{actor,action,context,event,start,registry,typed_client,sqlite}.rs` and
`rivetkit-core-3.1.0/src/actor/{schedule,connection,config}.rs`.

### 16.1 SDK facts that drive the design

- `trait Actor` has associated types `Input` (Default), `Actions` (tuple ActionSet, <= 128), `Events` (tuple
  EventSet, <= 16), `ConnParams` (Default), `Action`; hooks `create(&Ctx)`, `run`, `on_create`, `on_start`,
  `on_before_connect(ctx,&params)`, `on_connect(ctx, ConnCtx)`, `on_disconnect`, `on_subscribe(ctx, conn, event)`,
  `on_fetch`, `on_sleep`, `on_destroy`. `create` runs on every start; `on_create` only when `is_new`.
- There is **no `createConnState`**. `ConnCtx` exposes `id()`, `params()`, `send(name,&event)`,
  `is_hibernatable()`; `conn.inner().state()/set_state(Vec<u8>)` carry opaque per-conn bytes (persisted as CBOR for
  hibernatable conns).
- Actions: `trait Action {type Output; const NAME}` + `Handler<A>::handle(self: Arc<Self>, ctx, action)`. Args are
  positional CBOR: newtype struct decodes from `[obj]`, tuple struct from `[a,b]`, unit from `[]`. Output goes
  through the JSON value model; NaN rejected. **Actions are `tokio::spawn`ed concurrently.**
- **No queue handlers** (`QueueSend` rejected: "queue handler not found"); **no workflows**; **no framework-managed
  state**. Durable storage is `ctx.sql()` (`exec/query/run/execute/execute_batch`, `BindParam`,
  `SqliteDbExt::transaction`). `ctx.kv()` is deprecated.
- `ctx.schedule()`: `after(Duration, action_name, &[u8] cbor_args) -> id`, `at(ts_ms,...)`, `cancel(id)`, `get`, `list`.
  One-shots are **claimed before dispatch (at-most-once)** and fire as actions with `conn: None`; a crash
  mid-handler loses the timer. Defaults: `max_schedules` 1000, `action_timeout` 60 s, `sleep_timeout` 30 s.
- `ctx.emit(E)` / `ctx.broadcast(name,&payload)`; single-struct payload = one positional arg (same as TS).
- `ctx.client()` -> `rivetkit_client::Client`: `get_or_create(name,key,GetOrCreateOptions{params, create_with_input,..})`,
  `create(..,CreateOptions{input})`, `handle.action(name, Vec<JsonValue>)`, `connect()`.
- `ctx.keep_awake(fut)`, `register_task(fut)` (drained at shutdown), `wait_until`; `ctx.destroy()`, `ctx.sleep()`,
  `ctx.input()`, `ctx.key()` (`Vec<ActorKeySegment::String|Number>`), `ctx.abort_signal()`, `ctx.conns()`,
  `ctx.conn()`, `ctx.disconnect_conn`.
- `run` is a spawned task cancelled on stop; `report_run_error` stops the actor with an error.
- `ActorConfig`: `no_sleep`, `can_hibernate_websocket`, `sleep_timeout`, `action_timeout`.
- User errors: construct `RivetError{kind: Dynamic{group:"user", code, default_message}, meta, ..}` inside
  `anyhow::Error` (verify; public status for group `user` is 400).

### 16.2 Struct and storage

```rust
pub struct EuchreTable {
    lane: tokio::sync::Mutex<TableState>,   // the single-writer lane
    jwks: JwksCache,                         // lazily rebuilt per wake
}
```

SQL (actor SQLite):

```sql
CREATE TABLE IF NOT EXISTS table_state (
  id INTEGER PRIMARY KEY CHECK (id = 1), schema INTEGER NOT NULL,
  state_json TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS hand_review (
  hand_no INTEGER PRIMARY KEY, review_json TEXT NOT NULL);
```

`TableState` serde with `#[serde(rename_all="camelCase")]`, `Option<T>` => absent-or-null exactly as TS. `save()` =
`UPDATE/INSERT table_state` inside `transaction`. `archiveHand` writes `hand_review` AND the updated state blob in ONE
transaction so archive and `lastJournaledHandNo` are atomic (TS had a small window between kv.put and saveState).

`Actor::create`: load row, else build from `ctx.input()` (`TableCreateInput`, Default) plus `ctx.key()[1]` for gameId,
generate `internalToken` (uuid v4) and persist.

### 16.3 Run loop replacement

<!-- critic: the original line said "`run`: take lane, reconcile, release, then `ctx.abort_signal().cancelled().await`". 08 section 6 rule 6 and G35 verified that an active `run` handler blocks idle sleep (`CanSleep::ActiveRunHandler`), so a table that parks in `run` never sleeps. Do the reconcile in `create`/`on_start` (both run on every wake, 08 section 1) and write no `run` handler. -->
No `run` handler: in `on_start` (every wake), take the lane, `reconcile` (section 8.9), release. Reconcile MUST NOT
block startup on AI sends (they are detached). All mutating paths take the lane:

| TS path | Rust path |
|---|---|
| `move` queue / `submitMove` inline | `SubmitMove` action -> lock lane -> `onMove` |
| `review` queue | `ReviewTrick` action -> `onReview` inline (player-only; no internalToken needed since auth is conn state) |
| `aiDecision` queue | public `AiDecision` action (tokenOk first, constant-time via `subtle`, length first) |
| `aiSay` queue | public `AiSay` action (same) |
| `tick` queue | schedule-target actions run tick handlers directly (16.5) |

Persist -> fan-out -> return keeps save-before-ack. Redelivery on crash is replaced by client retry through
`recentMoveIds`/`appliedSeq` idempotency and by reconcile re-deriving timers on wake.

### 16.4 Actions (Rust names)

`SubmitMove{moveId,turnId,clientMoveId}`, `ReviewTrick` (enum tagged by `action`), `Snapshot`, `GetHandReview(u32)`,
`LastTrick`, `AiDecision(AIDecision)`, `AiSay(AiSayMsg)`, `OnTempoGate(String)`, `ReleaseAiMove(String)`,
`OnAiTimeout(String,u8)`, `FlushProfile`, `OnReap`. `NAME` constants are the TS action names verbatim
(`submitMove`, `reviewTrick`, `snapshot`, `getHandReview`, `lastTrick`, `aiDecision`, `aiSay`, `onTempoGate`,
`releaseAiMove`, `onAiTimeout`, `flushProfile`, `onReap`). `onNudge`/`onAbandon` are legacy no-ops: omit, or keep as no-ops
for wire parity (open question Q7). `AiDecision`/`AiSay` are new action names (TS used queue names `aiDecision`/`aiSay`);
the Rust aiSeat actor calls them as actions.

### 16.5 Timers (schedule) and the public-scheduled-action hole

Use `ctx.schedule().after(...)` with the same action names. Hardening (closes the TS hole where any client can call
`onTempoGate`):

1. Scheduled fires have `ctx.conn().is_none()`; HTTP/WS client calls always have a conn. Schedule-target handlers
   `forbid` when a conn is present. (Verify `ctx.conn()` semantics, Q1.)
2. Persist due times in state (`tempoDueAt`, `revealAt` already, `turnDeadlineAt` already, `reapDueAt`,
   `profileRetryDueAt`) and refuse an early fire.
3. Stale-turnId check retained.
4. At-most-once compensation: reconcile on every `create`/start re-derives and re-arms anything due; a handler error
   re-schedules the same timer with short backoff; optionally a guard timer per turn. Timer ids remain stored in state
   and are cancelled in `cancelTurnSchedules`.
5. Do not store durable meaning only in the schedule - the due-time ledger in state is the truth.

### 16.6 Connection identity and events

- `on_before_connect`: internal-token pre-check + Origin policy. Native clients: if an Origin header is present it must
  be allowlisted; if absent the JWT is the auth (Q4).
- `on_connect`: verify JWT (or internal token), claim owner, then store `TableConnState` CBOR via
  `conn.inner().set_state`. Actions read it back; missing conn/state => `forbidden`.
- `on_subscribe`: deny `sync` for non-players.
- Presence only for real WebSocket conns, since ephemeral HTTP action conns also run connect/disconnect (Q1). <!-- critic: `is_hibernatable()` is not a WebSocket test: 08 section 12 says `can_hibernate_websocket` defaults to false, so WS conns also report false. 08 G5/G6: stateless calls run on_before_connect + on_connect + on_disconnect per call and `conn.send` on them is silently dropped. Discriminate on params instead (internal callers carry `internalToken`; the HTTP API's own calls should carry an explicit marker), and verify with a probe before relying on it. -->
- Per-connection `sync`: iterate `ctx.conns()`, read each conn's seat from its state, `conn.send("sync",&SyncEvent)`.
- `thinking/chat/chatDelta/presence`: `ctx.broadcast`.
- Require player role for `Snapshot` (deliberate hardening vs TS).

### 16.7 aiSeat and playerProfile calls

- `ctx.client()?.get_or_create("aiSeat", key, GetOrCreateOptions{create_with_input: AiSeatCreateInput, params:{internalToken}})`
  then `.action("decide", [req])`.
- Dispatch is **detached**: `register_task`/`keep_awake` wraps a `tokio::time::timeout(deadline)` + abort-signal race
  (section 10.5). Do not await inside the lane. The aiSeat `decide` action must ack immediately and later call back into
  the table's `aiDecision` action.
- Lifecycle messages `resetHand/handEnd/gameEnd` -> aiSeat actions of the same names; detached, best-effort.
- `playerProfile` actions `recordHands`/`recordGame` with the profile token. `recordMatchIfNeeded`: snapshot under the lane,
  call outside it, re-take the lane to set `recordedAt` and arm the reap (avoids holding the lane across network).
- Because the dispatch is detached, ordering differs from TS (TS awaited up to the hard cap in-loop). The `pending` ledger
  and watchdog make this safe.

### 16.8 Rust acceptance tests to add

- concurrent `SubmitMove` x N on the lane never double-applies (seq/clientMoveId).
- scheduled-target action called by a client with a conn is rejected; early fire before due time is ignored.
- crash/restart between save and timer arm: reconcile re-arms; no stuck turn.
- `fenceNonce` golden vectors vs TS output.
- `screenLine`, `fnv1a`, `thinkFloorMs` golden vectors vs TS.
- JSON golden: `sync`, `MoveAck`, `TableState` round trip against TS-emitted fixtures (null vs absent).

---

## 17. Risks (see structured output)

At-most-once schedules; concurrency lane; conn identity/origin for native clients; `snapshot` leak; public timer
actions; f64 `fenceNonce`; UTF-16 `screenLine`; `fnv1a` parity; user-error construction; JSON null/absent parity;
detached AI dispatch ordering; reap 30 d vs doc 10 min; unbounded single-blob state growth; `ctx.conns()` race with
disconnects during fan-out.

## 18. Open questions

Q1 `ctx.conn()` and `on_connect` behavior for HTTP ephemeral conns and distinguishing WS conns. <!-- critic: partly answered by 08: `ctx.conn()` is Some for WS and stateless HTTP calls and None only for scheduled/internal dispatch (08 section 1, G24); stateless calls run the connect hooks per call (G5). Still open: a reliable WS-vs-HTTP discriminator. Q3 answered by 08 section 8 (`user_error` with `RivetErrorKind::Dynamic`, group `user`). Q4, Q7-Q10 decided in ARCHITECTURE D1, D7-D10. Q5: 08 section 2/12 verified writes survive sleep. Q6 open: 08 says schedules are persisted but does not test retry on Err. --> Q2 does `conn.inner().set_state`
persist across hibernation. Q3 exact way to throw `UserError`-equivalent with metadata. Q4 allow no-Origin native clients.
Q5 is SQL `execute` durable at return. Q6 does a scheduled handler returning `Err` get retried. Q7 keep legacy
`onNudge/onAbandon`. Q8 is the `topMove` heuristic dependency acceptable in the table crate. Q9 require `turnId` in
`submitMove`. Q10 migrate existing TS tables.
