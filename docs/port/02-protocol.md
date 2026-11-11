# 02 - Wire Protocol Contract (euchreTable)

Lane: WIRE PROTOCOL. Source of truth is the TS app at `/Users/brittianwarner/goods/euchre` (read-only). All paths below are relative to `src/` there. Line numbers refer to the files as read on 2026-11-10.

This spec is the contract that BOTH the Rust backend (`apps/server`), the Rust desktop client (`apps/desktop`, rivetkit-client 3.1.0) and the existing JS web client must satisfy. Anything described as "TS behaviour" must be reproduced byte-for-byte at the JSON level unless explicitly marked "may change".

Conventions used in this document:
- JSON field names are exact. Rust structs use `#[serde(rename_all = "camelCase")]` unless a field is listed otherwise.
- `T | null` means the key is ALWAYS present and may be `null`. `T?` means the key may be ABSENT (TS `undefined`; JSON.stringify drops it). A Rust client must accept both absent and `null` for every `?` field; a Rust server must OMIT (not emit `null`) for `?` fields (`#[serde(skip_serializing_if = "Option::is_none")]`).
- `Seat` = integer 0..3. `Team` = integer 0|1. Tuples `[a, b]` are JSON arrays of exactly that length.
- `CardId` = 2-char string, rank then suit: ranks `9 T J Q K A`, suits `S H D C`. Example `"JS"`, `"TC"`, `"9D"`, `"AH"` (types.ts:25-31).

---------------------------------------------------------------------------

## 0. Topology and call surface (what a client touches)

| Thing | Value | Source |
|---|---|---|
| Actor name | `euchreTable` | registry.ts `use: { euchreTable }`; play page `name: 'euchreTable'` |
| Actor key | `["table", gameId]` (gameId = UUID v4 lowercase, regex `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`) | play `+page.svelte` (key), table-session `+server.ts:11` |
| Other actors (not client-facing) | `aiSeat` key `["table", gameId, "seat", "1"\|"2"\|"3"]`; `playerProfile` key `["user", userId]` | euchre-table/index.ts:766, :1010 |
| Events (server -> client) | `sync` (per-connection), `thinking`, `chat`, `chatDelta`, `presence` (broadcast) | protocol/index.ts:307-325; queues.ts:228-238 |
| Actions (client -> server) | `snapshot`, `submitMove`, `reviewTrick`, `getHandReview`, `lastTrick` | euchre-table/index.ts:1796-1912 |
| Schedule-target actions (server internal; must NOT be client-callable in Rust port) | `onTempoGate`, `releaseAiMove`, `onAiTimeout`, `onNudge`, `onAbandon`, `flushProfile`, `onReap` | index.ts:1914-1947 |
| Queues (server internal) | `move`, `review`, `aiDecision`, `aiSay`, `tick` | queues.ts:200-210 |
| `PROTOCOL_VERSION` | `1` | protocol/index.ts:121 |

Client defaults used by the web app (`client/rivet.ts:48-60`): `endpoint = ${origin}/api/rivet`, `gateway: { skipReadyWait: true }`, action `timeout: 12_000` ms, `throwOnError: false`, `guardConnection: true`; actor handle opened with `noCreate: true` (the client never creates a table), `actorId` pre-resolved by the session endpoint, `getParams` async function (called on every (re)connect so the token is fresh).

### 0.1 Connection params (UNTRUSTED, client -> server on connect)

`TableConnectParams` (protocol/index.ts:144-163):

| TS | JSON | Type | Notes |
|---|---|---|---|
| `token` | `token` | string | REQUIRED. Actor JWT (see 0.3). |
| `seat?` | `seat` | Seat | Advisory. Web client never sends it. If present and `!== 0` the server throws `seat_mismatch`. |
| `protocolVersion?` | `protocolVersion` | number | Web client never sends it. If present and `!== 1` -> `protocol_version_mismatch`. |
| `internalToken?` | `internalToken` | string | SERVER-ONLY credential (per-match UUID). If it passes the constant-time compare the connection is `role: "internal"` and receives NO `sync`. Rust desktop client must NEVER send it. |

Server-derived `TableConnState` (index.ts:1659-1700; protocol/index.ts:189-197):

```json
{ "userId": "guest-1f0c...", "seat": 0, "role": "player", "since": 1767996000123 }
```
`role`: `"player" | "internal"`. A player is ALWAYS seat 0 in v1 (HUMAN_SEAT). Seats 1-3 are AI actors and never have connections. There are no spectators.

Connection pipeline, in order (`onBeforeConnect` then `createConnState`):
1. `onBeforeConnect`: if `params.internalToken` matches -> allow. Else check `Origin` header vs `ALLOWED_ORIGINS` env (comma list). Empty list: allow when `RIVET_ENDPOINT` is unset (local dev), otherwise throw `UserError` code `origin_not_allowed`. Non-empty and origin not in list -> `origin_not_allowed`. **A native desktop client sends no Origin; the Rust server must therefore treat missing Origin as allowed-when-token-valid (or the allowlist must include the empty origin) or the desktop client cannot connect in production. DECISION NEEDED (see Open Questions).**
2. `createConnState`: internal token fast path -> `{userId:"internal", seat:0, role:"internal"}`. Else protocolVersion check, seat claim check, then verify `token` (0.3). Failure -> `invalid_token`. Ownership: if `ownerUserId === ""` the first authenticated visitor claims the table (persisted immediately); else `userId !== ownerUserId` -> `forbidden` ("Not your game").
3. `onConnect` (index.ts:1709-1715): for `role === "player"` immediately `conn.send("sync", { v, view: tableView(seat), steps: [] })` and `broadcast("presence", { seat, online: true })`. `onDisconnect`: broadcast `presence { seat, online:false }` and clear `turnOpened.connected`.

### 0.2 Session minting endpoint (HTTP, used by browser before connecting)

`POST /api/table-session/{gameId}` (routes/api/table-session/[gameId]/+server.ts):
- Requires `Origin` header equal to request origin else 403 "Origin not allowed"; gameId must match the UUID regex else 400.
- Requires cookie `euchre_guest` = HS256 JWT (iss `euchre-guest`, aud `euchre-session`, `sub` starting `guest-`, 400d expiry) else 401.
- Mints actor token `signGuest(userId, "euchre-actors")` (HS256, same iss, aud `euchre-actors`, exp `2h`). NOTE: protocol doc comments say ~15 min; code is 2h. Gateway token (when `RIVET_TOKEN` env set) is `issueToken({subject:userId, expiresIn:900})`.
- Verifies ownership by calling `snapshot()` server-side; error code `forbidden` -> 403; any other failure -> 503.
- Response 200, `Cache-Control: no-store`:

```json
{
  "actorId": "<resolved rivet actor id>",
  "token": "<HS256 actor jwt>",
  "gatewayToken": "<optional; present only when RIVET_TOKEN is configured>",
  "expiresAt": 1767996900000
}
```
`expiresAt` = gateway token expiry, or `Date.now()+900000` when no gateway token. Client treats a cached session as stale `10_000` ms before `expiresAt` (rivet.ts:15) and refetches with `forceRefresh`. Error copy the web client shows: 403 "This table belongs to another browser. Open Your games to find your table."; 401 "Please start a game in this browser first."; other "Your table is temporarily unavailable. Please reconnect."

The Rust backend must expose an equivalent endpoint (axum). It MAY change the token scheme for the desktop client (e.g. bearer token in connection params), but the `token` field in `TableConnectParams` and the response shape above must remain accepted for the web client.

### 0.3 Token verification rules (`actors/auth/verify.ts`, `auth/guest.ts`)
- Token `"dev"` or empty -> invalid.
- If JWT protected header `alg === "HS256"`: `verifyGuest(token, "euchre-actors")`: iss `euchre-guest`, aud `euchre-actors`, `payload.kind === "guest"` and `sub` starts with `guest-`. userId = `sub`.
- Else (RS/EdDSA etc.): verify against remote JWKS `${APP_URL}/api/auth/jwks`, issuer `APP_URL` (trailing slash stripped), aud `euchre-actors`, clockTolerance 5s; userId = claim `id` or `sub`; claim `email` REQUIRED (`jwt_missing_claims` otherwise).
- HS256 secret: `EUCHRE_ACTOR_JWT_SECRET` || `BETTER_AUTH_SECRET` || `AUTH_SECRET` (>=32 chars); dev fallback key string `euchre-local-only-guest-signing-key` only when `NODE_ENV !== production` and `RIVET_ENDPOINT` unset.

### 0.4 Table creation (server-side only; not a client concern but shapes the wire)
`TableCreateInput` (index.ts:188-201), JSON input to `euchreTable.create(["table", gameId], { input })`:
```json
{
  "ownerUserId": "guest-...",
  "profileUserId": "optional user id for playerProfile",
  "seed": "optional hex; fixtures only",
  "cfg": { "gameTo": 10, "stickTheDealer": false, "requireNaturalTrump": false },
  "firstDealer": 0,
  "personas": [ /* PersonaAssignment x3, see section 8 */ ]
}
```
`/play` (routes/play/+page.server.ts) creates with `cfg.gameTo=10`, `stickTheDealer = (query stick === "true")`, `requireNaturalTrump = (query natural === "true")`. NOTE the engine default for `stickTheDealer` is `true` but `/play` defaults it to `false`. `cfg` is merged over `DEFAULT_ENGINE_CONFIG` (types.ts:306-318). Redirect target `/play/{gameId}`.

---------------------------------------------------------------------------

## 1. Core wire types (engine vocabulary, re-exported through protocol)

All from `euchre/types.ts` (re-exported by `protocol/index.ts:70-100`).

### 1.1 Scalars / enums

| TS type | JSON representation | Values |
|---|---|---|
| `Suit` | string | `"S" "H" "D" "C"` |
| `Rank` | string | `"9" "T" "J" "Q" "K" "A"` |
| `CardId` | string | `${Rank}${Suit}` e.g. `"JS"` |
| `Seat` | number | 0 South (human), 1 West, 2 North (partner), 3 East. Clockwise. |
| `Team` | number | `seat & 1`. Team 0 = {0,2} = human team |
| `SeatRole` | number | 0 dealer, 1 eldest, 2 dealer's partner, 3 third/cutter |
| `GamePhase` | string | `lobby cutting deal bid_round_1 dealer_discard bid_round_2 trick_play trick_resolve hand_score game_over` |
| `GameStatus` | string | `active complete abandoned` |
| `HandResult` | string | `point march lone_point lone_march euchre throw_in` |
| `RuleCode` | string | `not_your_turn wrong_phase illegal_move must_follow_suit card_not_in_hand bidding_closed dealer_must_call sitting_out stale_turn` |
| `ActionKind` | string | `human ai auto` |
| `AIDecisionSource` | string | `llm fallback forced` |
| `AutoReason` | string | `ai_timeout ai_unreachable abandon` |

Engine-only phases (`turnSeat === null`, `legal === []` for everyone): `deal`, `trick_resolve`, `hand_score`, `game_over` (and `lobby`). Clients almost never observe `lobby`/`deal` because the table settles through them atomically (see 5.2).

### 1.2 Move / Bid / LegalMove (tagged unions, discriminator key `"t"`)

```ts
type Bid  = {t:'pass'} | {t:'orderUp', alone:boolean} | {t:'call', suit:Suit, alone:boolean};
type Move = Bid | {t:'cut', cut:boolean} | {t:'discard', card:CardId} | {t:'play', card:CardId};
```
JSON examples: `{"t":"pass"}`, `{"t":"orderUp","alone":false}`, `{"t":"call","suit":"H","alone":true}`, `{"t":"cut","cut":true}`, `{"t":"discard","card":"9C"}`, `{"t":"play","card":"JS"}`.

Rust: `#[serde(tag = "t")] enum Move { #[serde(rename="pass")] Pass, #[serde(rename="orderUp")] OrderUp{alone:bool}, #[serde(rename="call")] Call{suit:Suit, alone:bool}, #[serde(rename="cut")] Cut{cut:bool}, #[serde(rename="discard")] Discard{card:CardId}, #[serde(rename="play")] Play{card:CardId} }`.

`LegalMoveId` (string; the ONLY thing a client submits):

| Form | Meaning |
|---|---|
| `pass` | Pass |
| `orderUp` | Round-1 accept (not alone) |
| `orderUp+alone` | Round-1 accept, alone |
| `call:<S|H|D|C>` | Round-2 name suit |
| `call:<suit>+alone` | Round-2 name suit, alone |
| `cut:yes` | Bump (cut = true) |
| `cut:no` | Run 'em (cut = false) |
| `discard:<CardId>` | e.g. `discard:9C` |
| `play:<CardId>` | e.g. `play:JS` |

Move -> id function is `moveId()` in reduce.ts (id = exactly the strings above; `+alone` suffix when `alone` true).

`LegalMove` (types.ts:141-146): `{ "id": LegalMoveId, "move": Move, "label": string }`.

Label strings the UI shows verbatim (legal.ts:181-240, 53-68):

| Phase | id | label |
|---|---|---|
| cutting | `cut:yes` / `cut:no` | `Bump` / `Run 'em` (apostrophe is ASCII `'`) |
| bid_round_1 | `pass` | `Pass` |
| bid_round_1 | `orderUp` | `I take it` (seat is dealer) / `I assist` (seat is dealer's partner) / `Order it up` (otherwise); `orderUp+alone` = same + `, alone` |
| bid_round_2 | `pass` | `Pass` (omitted for the stuck dealer when `stickTheDealer`) |
| bid_round_2 | `call:S` etc. | `Spades` `Hearts` `Diamonds` `Clubs`; alone variant `Hearts, alone`. Turned-down suit omitted. If `requireNaturalTrump` and not stuck dealer, suits the seat holds no card of are omitted. |
| dealer_discard | `discard:<c>` | cardName e.g. `Jack of diamonds` (one entry per card in the dealer's 6-card hand, hand order) |
| trick_play | `play:<c>` | cardName; entries are the legal plays only, in hand order (see getLegalPlays) |

`cardName(c)` = `"<Rank name> of <suit name lowercased>"` with rank names `9 Nine, T Ten, J Jack, Q Queen, K King, A Ace`, suit names `Spades Hearts Diamonds Clubs`. `requireNaturalTrump` round-1: non-dealer seats holding no card of the up-card suit may only `pass` (legal = `[pass]`).

Order of `legal` is "engine order": cut `[yes,no]`; round-1 `[pass, orderUp, orderUp+alone]`; round-2 `[pass?, then for each suit in SUITS order (S,H,D,C): call:s, call:s+alone]`; discard/play in the seat's hand order (the order of `view.hand`). `view.legal` is `[]` unless `turnSeat === you`.

### 1.3 Trick, Play, BidRecord

```ts
Play      = { seat: Seat, card: CardId }
Trick     = { index: number /*0..4*/, ledSuit: Suit|null /* EFFECTIVE suit of 1st play */, plays: Play[], winnerSeat: Seat|null }
BidRecord = { seat: Seat, move: Bid, say: string /* verbatim table line */ }
```
`Trick.ledSuit` is the EFFECTIVE suit: leading the left bower leads trump. `winnerSeat` is `null` until the trick resolves.

### 1.4 Step (choreography channel) - types.ts:234-269

Discriminator `t`. All variants:

| `t` | Fields (JSON) | Emitted when |
|---|---|---|
| `cut` | `seat: Seat`, `cut: bool` | cutter answered |
| `dealt` | `packets: DealtPacket[]` where `DealtPacket = { seat: Seat, count: number, cards: CardId[]\|null }` | the deal (3-2/2-3 alternation); REDACTED per receiver |
| `upCardTurned` | `card: CardId` | up-card turned (public) |
| `bid` | `seat`, `id: LegalMoveId`, `say: string` | a pass/orderUp/call |
| `turnedDown` | `suit: Suit` | 4th round-1 pass (dealer turns it down) |
| `trumpSet` | `suit`, `makerSeat`, `aloneSeat: Seat\|null`, `sittingSeat: Seat\|null`, `viaOrderUp: bool` | trump determined |
| `dealerDiscarded` | `seat: Seat` (NO card field, ever) | dealer discards (or loner-sitting-dealer no-op) |
| `cardPlayed` | `seat`, `card` | a card laid |
| `trickWon` | `index: number`, `winnerSeat`, `plays: Play[]`, `sealsEuchre: bool` | trick complete (same transition as the 4th/3rd `cardPlayed`) |
| `handScored` | `result: HandResult`, `delta: [n,n]`, `score: [n,n]`, `makerSeat: Seat\|null`, `aloneSeat: Seat\|null`, `tricksWon: [n,n]` | hand scored |
| `throwIn` | (none) | round-2 exhaustion with stickTheDealer off |
| `gameWon` | `team: Team`, `score: [n,n]` | match over |

Rust: `#[serde(tag="t", rename_all="camelCase", rename_all_fields="camelCase")] enum Step { Cut{seat,cut}, Dealt{packets}, UpCardTurned{card}, Bid{seat,id,say}, TurnedDown{suit}, TrumpSet{suit,maker_seat,alone_seat,sitting_seat,via_order_up}, DealerDiscarded{seat}, CardPlayed{seat,card}, TrickWon{index,winner_seat,plays,seals_euchre}, HandScored{result,delta,score,maker_seat,alone_seat,tricks_won}, ThrowIn, GameWon{team,score} }`. Variant tag strings must be exactly those in the table (`upCardTurned`, `throwIn`, ...). Nullable fields (`aloneSeat` etc.) are `Option` serialized as explicit `null` (present key). Unit variant `throwIn` serializes `{"t":"throwIn"}`.

---------------------------------------------------------------------------

## 2. PublicGameView (the only state shape that leaves the server)

`types.ts:462-511` plus actor-added metadata (`tableView`, index.ts:453-473). Built by `project(state, seat)` then spread-merged with actor fields.

| TS field | JSON key | JSON type | Optional? | Meaning / redaction |
|---|---|---|---|---|
| `players?` | `players` | `TablePlayer[]` | `?` (actor adds; present on every actor-sent view) | public persona metadata, see 2.1 |
| `observations?` | `observations` | `TableObservation[]` | `?` (actor adds, may be `[]`) | public timing log for the CURRENT hand only; <=32 entries |
| `reviewTricks?` | `reviewTricks` | bool | `?` (actor adds, default false) | accessibility pref: "pause after each trick until Continue" |
| `rules?` | `rules` | `{stickTheDealer: bool, requireNaturalTrump: bool}` | `?` | from `cfg` |
| `awaitingTrickReview?` | `awaitingTrickReview` | bool | `?` | true only when `phase==="trick_resolve" && reviewReadyTurnId===turnId` |
| `v` | `v` | int | | state version; monotonic per match |
| `you` | `you` | Seat | | seat this view was projected for (always 0 for the human) |
| `status` | `status` | GameStatus | | |
| `phase` | `phase` | GamePhase | | |
| `handNo` | `handNo` | int | | 0-based hand counter; increments on every scored hand AND every throw-in |
| `dealerSeat` | `dealerSeat` | Seat | | |
| `turnSeat` | `turnSeat` | Seat\|null | | null in engine-only phases |
| `turnId` | `turnId` | string | | opaque turn nonce (base36 chars from two fnv1a hashes concatenated); echoed in `submitMove`/`reviewTrick continue`; regenerated on EVERY engine transition |
| `score` | `score` | `[int,int]` | | `[team0, team1]` |
| `trump` | `trump` | Suit\|null | | |
| `makerSeat` | `makerSeat` | Seat\|null | | |
| `aloneSeat` | `aloneSeat` | Seat\|null | | implies `== makerSeat` |
| `sittingSeat` | `sittingSeat` | Seat\|null | | loner's partner |
| `upCard` | `upCard` | CardId\|null | | NEVER nulled once dealt (V14); public for whole hand |
| `upCardTurnedDown` | `upCardTurnedDown` | bool | | distinguishes buried vs picked-up |
| `turnedDownSuit` | `turnedDownSuit` | Suit\|null | | suit unnameable in round 2 |
| `hand` | `hand` | CardId[] | | YOUR cards only. `[]` for the sitting seat |
| `handCounts` | `handCounts` | `[int,int,int,int]` | | indexed by seat; `handCounts[sittingSeat] === 0` |
| `kittyCount` | `kittyCount` | int | | `3 + ((upCardTurnedDown \|\| dealerDiscard !== null) ? 1 : 0)` i.e. 3 or 4 |
| `trick` | `trick` | Trick | | current trick (during `trick_resolve` it equals last entry of `trickLog`) |
| `trickLog` | `trickLog` | Trick[] | | completed tricks of this hand, oldest first, <=5 |
| `tricksWon` | `tricksWon` | `[int,int]` | | per team this hand |
| `passes` | `passes` | int | | passes within current round (reset to 0 entering round 2); NOT reset when trump is set |
| `bids` | `bids` | BidRecord[] | | auction so far, <=8 |
| `legal` | `legal` | LegalMove[] | | `[]` unless `turnSeat === you` (and never empty when it is) |
| `result` | `result` | HandResult\|null | | set at `hand_score` |
| `delta` | `delta` | `[int,int]`\|null | | post-clamp points of this hand |
| `winnerTeam` | `winnerTeam` | Team\|null | | set when a team reaches `cfg.gameTo` |

Field order in JSON is irrelevant. The pure `project()` output omits `players/observations/reviewTricks/rules/awaitingTrickReview`; the actor ALWAYS adds all five (so on the wire they are always present, but a client MUST tolerate absence, e.g. AI requests built from `tableView` also include them).

### 2.1 Nested actor metadata

`TablePlayer`: `{ "seat": Seat, "name": string, "blurb": string, "style": string }`. Built from the 3 persona assignments (seats 1,2,3 only; seat 0 is absent). `style` rule (index.ts:460-461): `persona.risk >= 0.7 ? "Bold bidder" : persona.risk <= 0.3 ? "Card counter" : "Steady player"`. Display name for UI: `seat === view.you ? "You" : players.find(seat).name || seatName(seat, you)`; `seatName` = `["You","Left opponent","Your partner","Right opponent"][(seat - you + 4) % 4]`.

`TableObservation`: `{ "handNo": int, "seat": Seat, "kind": "cut"|"bid1"|"bid2"|"discard"|"play", "action": string, "elapsedMs": int|null, "timing": "human-observed"|"computer-paced"|"unreliable" }`.
Server logic (observeMove, index.ts:551-580): appended on every accepted move of a deciding phase (`kind` from phase: cutting->`cut`, bid_round_1->`bid1`, dealer_discard->`discard`, bid_round_2->`bid2`, trick_play->`play`). `action` = the move's `label` for `play|pass|orderUp|call`; for discard the literal `"Dealer discarded a private card"`; for cut `"Cut decision"` (so the discard/cut specifics stay private). Human: `reliable = turnOpened.connected && elapsed in [0, 20000] ms and turnOpened.turnId === before.turnId`; `elapsedMs = reliable ? round(elapsed/250)*250 : null`; `timing = reliable ? "human-observed" : "unreliable"`. AI: `elapsedMs = null`, `timing = "computer-paced"`. List is filtered to the current `handNo` and truncated to the last 32 (`slice(-32)`), and the view re-filters by current handNo.

### 2.2 Redaction rules (what a viewer may and may not see)

The ONLY redaction boundary is `project(state, seat)` + `projectSteps(steps, seat)`; in the Rust port `PublicGameView` must have NO field capable of holding hidden cards.

| Hidden from viewer | Mechanism |
|---|---|
| Other seats' hands | `hand` is only `hands[seat]`; others expose only counts in `handCounts` |
| Kitty cards 2-4 (buried 3) | not representable; only `kittyCount` |
| Dealer's face-down discard | not representable; `dealerDiscarded` step has no card; `kittyCount` depends only on WHETHER a discard exists (V13) |
| Deck order / seed / internalToken / `pending` AI decision / hand journal | not in view |
| Sitting partner's cards (loner) | sitting seat gets `hand: []` and `handCounts[seat]=0` from the moment trump is called |
| AI rationale / chain of thought | never on the wire (it would leak the caller's holdings); only `chat` lines |
| Other seats' `legal` | `legal` computed for `you` only; `[]` when not your turn |

Always public: `upCard` (stays identified all hand, even when the dealer takes it or discards it), `turnedDownSuit`, all plays in `trick`/`trickLog`, `bids` with `say`, `score`.
`dealt` step: each packet's `cards` is `null` unless `packet.seat === receiverSeat` (then the cards array is copied); `count` always survives (`projectSteps`, euchre/index.ts:307-317). All other steps pass through unchanged.

`kittyCount` formula: 3 until the up-card is turned down or the dealer has discarded; then 4. Never reveals which.

---------------------------------------------------------------------------

## 3. Events (server -> client)

Exactly five. Names are literal strings. `sync` is the only per-connection event (and the only one that can carry hidden-card info); the rest are `c.broadcast`.

| Event | Delivery | Payload type | Subscribable by |
|---|---|---|---|
| `sync` | `conn.send("sync", payload)` per connection, `role === "player"` only | `SyncEvent` | players only (`canSubscribe: playersOnly`) |
| `thinking` | broadcast | `ThinkingEvent` | all |
| `chat` | broadcast | `ChatEvent` | all |
| `chatDelta` | broadcast | `ChatDeltaEvent` | all |
| `presence` | broadcast | `PresenceEvent` | all |

`PER_CONNECTION_EVENTS = ["sync"]`. In the Rust port any NEW event that could carry a CardId must go through `sync`.

### 3.1 `sync` - `SyncEvent`
```json
{ "v": 58, "view": { /* PublicGameView */ }, "steps": [ { "t": "cardPlayed", "seat": 0, "card": "JH" } ] }
```
- `v`: equals `view.v` (both are `game.v` at send time).
- `view`: `tableView(c, conn.state.seat)` (project + actor metadata).
- `steps`: `projectSteps(stepsSinceLastSync, seat)`. Empty array `[]` means "snap, do not animate": used on (re)connect (`onConnect`), on `reviewTrick` set/ready (`pushSync(c, [])`), and the review-gate sync (see 5.3).
- Missed `sync`s are NEVER replayed by the transport. Rivet may drop events across a network drop. Therefore the client resyncs via `snapshot()` on connect (5.4).
- One `sync` per accepted state change, batching all steps produced by that change (apply steps + settle steps). Multiple engine transitions in one actor message (e.g. cut + deal + upCardTurned) arrive as ONE sync with multiple steps and `v` bumped by the number of `commit()` calls (see 4).

### 3.2 `thinking` - `ThinkingEvent`
`{ "seat": Seat, "on": bool, "extended": bool }` (protocol/index.ts:239-243).
- `on:true, extended:false` is broadcast by `dispatchAi` right after the AI decision is parked and persisted (index.ts:821), seats 1-3 only.
- `on:false, extended:false` after an AI move is applied (`applyMove`, index.ts:929) and at an AI watchdog timeout (index.ts:1461).
- `extended:true` is declared in the contract ("still thinking...", at `TEMPO.aiExtendedAtMs` = 2200 ms prod / 800 ms fast) but NO code path emits it in this TS source (grep: only `extended:false` emitted). The Rust server MAY implement it; clients must tolerate it.
- The web client does NOT subscribe to `thinking`; the UI derives "who is thinking" from `view.turnSeat`. The desktop client may use it.

### 3.3 `chat` - `ChatEvent`
`{ "msgId": string, "seat": Seat, "kind": "call"|"banter"|"system", "text": string, "final": true }` (`final` is always the literal `true`).
- `kind: "call"`: engine-authored table script. `kind: "system"`: engine notice. `kind: "banter"`: LLM persona chatter, screened.
- All text passes `screenLine(text, BANTER_MAX_CHARS=140)`: every char with code < 0x20 or == 0x7f becomes a space, whitespace runs collapse to a single space, trim, hard `slice(0,140)` (UTF-16 code units in TS; port should clamp by characters but the cap must be 140). Empty result -> the event is NOT emitted. Rendered as text, never markup.
- Emission points (index.ts:506-527, :1289-1291, :1359-1361):

| Trigger | seat | kind | text | msgId |
|---|---|---|---|---|
| each `bid` step | step.seat | `call` | `step.say` | `${gameId}:${v}:${n}` where `v` = `game.v` AFTER the whole commit+settle and `n` = index of the step in the steps array |
| `throwIn` step | `game.hand.dealerSeat` (the NEW dealer, since state already rotated) | `system` | `Throw it in. Nobody wanted it.` | same pattern |
| `handScored` with `result==="euchre"` | `step.makerSeat` | `call` | `Euchre!` | same pattern |
| `handScored` with `result` `march` or `lone_march` | `step.makerSeat` | `call` | `March!` | same pattern |
| accepted `aiDecision` with `banter` string | acting seat | `banter` | screened banter | `${gameId}:${turnId}:${seat}` |
| `aiSay` message with `final:true` | body.seat | `banter` | body.text | body.msgId |

Chat for a move is emitted BEFORE the `sync` for that move (`emitScript` then `pushSync`). `say` strings produced by the reducer (`sayForBid`, reduce.ts:201-218): pass `Pass.` (the dealer's 4th round-1 pass: `Turn it down.`); orderUp `${orderLabel}.` e.g. `Order it up.` / `I assist.` / `I take it.` plus ` Alone.` when alone; round-2 call: stuck dealer `${Suit}, I guess.`; else if a suit was turned down: same colour as turned-down suit -> `Next.`, other colour -> `Crossing the creek.`; else `${Suit}.`; ` Alone.` appended when alone. (`lone` suffix is the literal string " Alone." with the leading space.)

### 3.4 `chatDelta` - `ChatDeltaEvent`
`{ "msgId": string, "seat": Seat, "delta": string }`. Streaming fragment of a banter line (delta also screened/clamped to 140). Emitted from `aiSay` messages with `final:false` and non-empty screened delta, only when the message's `handNo` equals the current hand (or, for legacy messages without `handNo`, `turnId` equals current). Batched upstream (~80 ms/~40 chars). A later `chat` with the same `msgId` replaces the accumulated buffer; a rejected line is retracted by msgId client-side. The web client does NOT subscribe to `chatDelta`.

### 3.5 `presence` - `PresenceEvent`
`{ "seat": Seat, "online": bool }`. On player connect/disconnect. The web client does not subscribe.

---------------------------------------------------------------------------

## 4. Versioning, seq and turnId

| Field | Owner | Rules |
|---|---|---|
| `GameState.v` / `view.v` / `SyncEvent.v` | ACTOR | Starts at 0 in `createGame`. `commit(c, state)` sets `game = {...state, v: game.v + 1}`: EVERY reducer output adopted (`apply`, each `advance` inside `settle`, tempo `advance`, review set/ready re-commit) increments it by exactly 1. The reducer never touches `v`. Strictly monotonic per match. First observable `v` after creation is 1 (`settle` runs `advance(lobby)->startHand`, one commit). A cut answered: `apply` commit -> v+1 (deal is atomic inside `applyCut`). `submitMove` of a `play` that completes a trick: 1 commit (trick_resolve is a resting phase). A tempo release from `trick_resolve`: 1 commit. `hand_score -> next hand`: 1 commit. `reviewTrick set/ready` also commits (`commit(c, game)`) with NO engine change so `v` bumps and `turnId` is unchanged. |
| `turnId` | ENGINE | `mintTurnId(state) = fnv1a(internalToken + "#" + prevTurnId + "#a").toString(36) + fnv1a(internalToken + "#" + prevTurnId + "#b").toString(36)`; initial `fnv1a(seed + "#turn0").toString(36)`. Changed by `next()` i.e. every `apply`/`advance`/`startHand`. NOT changed by the actor's review re-commit. Opaque to clients: compare for equality only. (A Rust server needs the same hash only for replay-determinism; clients never recompute it. fnv1a = 32-bit FNV-1a over the UTF-16 code units of the string (`s.charCodeAt(i)`, one XOR per code unit, not per UTF-8 byte): offset 0x811c9dc5, prime 0x01000193, in `euchre/deal.ts:27-34`.) <!-- critic: was "UTF-8 bytes"; deal.ts:30 uses charCodeAt. Identical for ASCII seeds, different for any non-ASCII seed or token --> |
| `appliedSeq` | ACTOR (internal) | Not on the wire. Set to `body.seq` on human moves; `seq = appliedSeq+1` at enqueue. In the shipped inline path `seq` is always `appliedSeq+1` so the `seq <= appliedSeq` branch is dead; dedupe actually works via `recentMoveIds`. |
| `recentMoveIds` | ACTOR (internal) | Ring of last 32 `clientMoveId`s (each truncated to 64 chars). Not in the view. |
| `clientMoveId` | CLIENT | `crypto.randomUUID()` per submit; idempotency key. |
| `StepsSeen` | - | There is no step sequence number. Steps are decoration with no ids; dedupe on the client is by hashing `${handNo}:${t}:${JSON.stringify(step)}` (GameFeedback, 80-entry window). |

Client drop rule: `applySync` drops an event when `payload.view.v < store.view.v` (STRICTLY less; equal `v` IS applied). The protocol docs say "drops v <= seen" but the code is `<`. The Rust desktop client should follow the code (apply on `>=`), because the actor may push a second view with the same `v`... in practice it does not, but the web store would accept it. `snapshot()` results are applied only when `!view || snap.v >= view.v`.

---------------------------------------------------------------------------

## 5. Actions (client -> server)

All five are normal rivetkit actions. Argument encoding is rivetkit's standard action args array: `args = [request]`. Responses are the action return value. With the web client's `throwOnError:false`, a thrown `UserError` resolves to `undefined` instead of throwing; a Rust client will see an `ActionError` with `code`/`message`/`metadata`. <!-- critic: rivetkit-client 3.1.0 has no structured error type (08 G16): the Rust client gets an anyhow string `RPC Error(user/<code>): <message>, null` on a connection or `action failed (user/<code>): <message>, metadata=None` on a stateless call. Parse `group/code` with 08 section 8 `parse_rivet_error`. Only `user/*`, `auth/forbidden` and the other public groups pass through; any other server error is masked to `core/internal_error`. --> Rust client must therefore treat BOTH `Ok(MoveAck{ok:false})` and an `Err(code)` as rejections.

### 5.1 `snapshot() -> PublicGameView`
Read-only, no args. Returns `tableView(c, c.conn.state.seat)`. Any authenticated connection (including `internal`; table-session endpoint calls it with only `params:{token}` to verify ownership and 403 on `forbidden`).

### 5.2 `submitMove(request) -> MoveAck`
Request (`SubmitMoveRequest`):
```json
{ "moveId": "play:JH", "turnId": "k2ab9x1c4d", "clientMoveId": "9f0f1d0e-5c0a-4c8e-9a53-0f9e2b3a1c11" }
```
NO `seat` field exists or is read. Acting seat = server constant HUMAN_SEAT (0), verified against `c.conn.state`.

Server algorithm (index.ts:1861-1912, then `onMove` 1182-1204):
1. `conn.role !== "player"` or `conn.seat !== 0` -> throw `UserError("Not a player connection", code "forbidden")`.
2. `game.status !== "active"` -> throw `UserError("This match is over", code "game_complete")`.
3. `legal = legalMoves(game, 0)`; `move = legal.find(id === request.moveId)`. Not found -> throw `UserError("That move is not legal right now", { code: "illegal_move", metadata: { legal } })`. (Includes "not your turn": legal is `[]` then.)
4. `clientMoveId` must be a non-empty string else throw `UserError("Missing clientMoveId", code "internal_error")`.
5. `onMove`: if `status !== active` -> `{ok:false, code:"game_complete", legal:[]}` (unreachable after step 2); if `clientMoveId` in `recentMoveIds` -> `{ ok:true, v: game.v }` (idempotent no-op; NOTE step 3 runs first, so a duplicate whose move is no longer legal gets `illegal_move` instead; the web client handles this by resyncing and checking `turnId` changed -> treat as success).
6. `apply(game, 0, move.move, { turnId: request.turnId })`. `RuleError` (rules violation incl. `stale_turn` when `request.turnId !== game.turnId`) is caught by the action wrapper and returned as `{ ok:false, code: err.code, legal: err.legal }` (legal = current legal set for seat 0).
7. On success, in order: `commit` (v+1) -> `appliedSeq = seq`, `pending = null` -> `observeMove` -> `rememberMoveId` -> journal -> `settle` (advance through `lobby`/`deal` only) -> **PERSIST** (`saveState immediate`) -> `emitScript` (chat `call`/`system` lines) -> `pushSync(steps)` -> `handBoundary` (archive review, record hand, notify AI) -> `armTurn` (schedule next turn) -> return `{ ok:true, v: game.v }`.
   => The ack is returned AFTER the `sync` has been sent to the connection, and `ack.v` equals the `v` of that sync (or a later one if an AI/timer already advanced further; because `armTurn` may synchronously dispatch AI whose reply arrives later, `ack.v` is the post-human-move v). Rust server must preserve "persist -> fan out sync -> ack".

`MoveAck` (protocol/index.ts:373-379):
```json
{ "ok": true, "v": 58 }
{ "ok": false, "code": "stale_turn", "legal": [ { "id": "play:JH", "move": {"t":"play","card":"JH"}, "label": "Jack of hearts" } ] }
```
`code` is `RuleCode | ProtocolErrorCode`. The mapping the client shows is generic copy; the web client does not branch on specific codes (see 6.3).

### 5.3 `reviewTrick(request) -> { queued: true }`
Request union:
```json
{ "action": "set", "enabled": true }
{ "action": "continue", "turnId": "k2ab9x1c4d" }
```
Validation (index.ts:1798-1820): non-player -> `forbidden`; invalid shape -> `UserError code "internal_error"`. Enqueues internal `review` message (`ReviewMsg`: `internalToken`, `gameId`, `action`, `enabled?`, `turnId`; for `set`, `turnId` = current turn) and returns `{ "queued": true }` immediately (non-blocking). Effects in the run loop (`onReview`, index.ts:1383-1405):
- `set`: `state.reviewTricks = enabled`; `commit` (v+1, same turnId); `pushSync(c, [])`; if disabling while `reviewReadyTurnId === turnId` it advances out of `trick_resolve` immediately.
- `continue`: only if `body.turnId === game.turnId && reviewReadyTurnId === game.turnId && phase === "trick_resolve"`: `advanceTempo` (clear `reviewReadyTurnId`, `advance`, commit, persist, emit script, pushSync(steps), handBoundary, armTurn). Otherwise silently ignored (still returns `{queued:true}` at the action level).

### 5.4 `getHandReview(handNo: number) -> CompletedHandReview | null`
Args `[handNo]`. Player only (else `forbidden`). Returns `null` if `handNo` is not an integer, `< 0`, `> current handNo`, or no archive exists. Otherwise the JSON parsed from kv key `review/${handNo}` (written by `archiveHand`, section 7). See section 7 for the type.

### 5.5 `lastTrick() -> Trick | null`
No args. Returns last entry of `hand.trickLog` or `null` if empty. Read-only, no role check. (The web client does not call it; `getHandReview` + `view.trickLog` supersede it. Keep for parity.)

---------------------------------------------------------------------------

## 6. Client-side store state machine (`game/table.svelte.ts`, play page)

### 6.1 `TableStore` state
| Field | Type | Initial | Notes |
|---|---|---|---|
| `view` | `PublicGameView \| null` (`$state.raw`) | `null` | latest accepted server view |
| `status` | `'idle'\|'connecting'\|'ready'\|'error'` | `'idle'` | `bind()` sets `'ready'` if `handle.isConnected` else `'connecting'`; `'error'` is declared but never assigned in this source |
| `error` | `string\|null` | null | user-facing copy |
| `submitting` | bool | false | true while a `submitMove` is in flight; blocks double-submit |
| `chat` | `{msgId, seat, kind, text}[]` | `[]` | spoken log; keeps the last 6 only |
| private `#handle` | actor handle | null | |
| private `#resyncPromise` | Promise\|null | null | coalesces concurrent resyncs |

### 6.2 Operations
- `applySync(payload)`: `if (view && payload.view.v < view.v) return; view = payload.view; status='ready'; error=null`. Note `payload.steps` is NOT applied to state; the PAGE stores `lastSteps = payload.steps` on EVERY sync event (even when `applySync` dropped it, and not updated by snapshot resyncs).
- `applyChat(payload)`: ignore if a message with the same `msgId` already exists (redelivery); else `chat = [...chat, payload].slice(-6)`.
- `resync()`: returns the in-flight promise if any; else `#fetchSnapshot()`: no-op unless `handle.isConnected`; `snap = await Promise.race([handle.snapshot(), reject after 4000ms "Snapshot timed out"])`; falsy -> error "Snapshot unavailable"; apply only if `!view || snap.v >= view.v` (sets `status='ready'`, `error=null`). Any failure: `error = "Waiting for the table to reconnect. Your game is saved."`. Never animates.
- `play(moveId)`: returns false immediately if no handle/view or `submitting`. Checks `view.legal.some(m=>m.id===moveId)` else `error='That move is not legal right now.'`, false. Then `submitting=true; error=null; clientMoveId = crypto.randomUUID()`; `ack = await handle.submitMove({moveId, turnId: view.turnId, clientMoveId})` where `view` is the view captured BEFORE the await.
  - `!ack || ack.ok === false` (rejection OR thrown-and-swallowed undefined): `await resync()`; if the refreshed `this.view.turnId !== view.turnId` the move actually landed (lost ack) -> return `true`; else `error='The table could not confirm your move. Your game is saved. Please try again.'`, return false.
  - `ack.ok`: `if (!this.view || this.view.v < ack.v) await resync()`; return true.
  - exception: `console.error`, `await resync()`, same turnId-changed -> true; else `error='Connection interrupted. Your game is saved. Reconnect and try again.'`, false.
  - `finally submitting=false`.

### 6.3 Page-level wiring (routes/play/[gameId]/+page.svelte)
1. On mount, `getActorId()` -> session POST (0.2); `useActor({name:'euchreTable', actorId, enabled: Boolean(actorId), noCreate:true, key:['table', gameId], getParams})`.
2. Event handlers registered at component init (NOT in effects): `chat` -> `store.applyChat(payload)` and, when `kind==="banter"` and `0<=seat<=3`, set `tableTalk = {seat, text, msgId, handNo: store.view?.handNo ?? null}` for 6500 ms (shown only while `tableTalk.handNo === view.handNo`); `sync` -> `store.applySync(payload); lastSteps = payload.steps`. `thinking`/`chatDelta`/`presence` are NOT subscribed.
3. `store.bind(table)`.
4. Resync triggers: (a) `$effect` whenever `table.isConnected` becomes true -> `store.resync()`; (b) every 10_000 ms `setInterval` if `table.isConnected && document.visibilityState==='visible'` -> `store.resync()`. (There is also the connect-time server push `sync{steps:[]}`; both may race, `v` ordering resolves it.)
5. Optimistic preview: `optimistic = {forV, view}`; `displayView = optimistic && optimistic.forV === live.v ? optimistic.view : live`, falling back to `lastGoodView` (a latch of the last non-null display view) when `store.view` is null so the 3D table never remounts. In `submitMove(moveId)`: guard `view && !store.submitting && table.isConnected`; find the `LegalMove` in `view.legal`; record `lastDecision`; for `play` moves capture the card element rect as the flight origin; set `optimistic = {forV: view.v, view: applyOptimistic(view, entry.move)}`; `ok = await store.play(moveId)`; `if (!ok) optimistic = null`. An optimistic view is shown ONLY while the live view's `v` still equals the `v` it was computed against; the next sync (higher v) automatically supersedes it.
6. Review: `table.reviewTrick(req)` guarded by `isConnected && !reviewBusy`; success iff `ack?.queued`; failure copy `'Could not update the table. Please try again.'` / `'The table did not confirm. Please try again.'`.
7. Hand review panel: `load={(handNo)=>table.getHandReview(handNo)}`; for the current hand it falls back to `view.trickLog`.
8. Where steps are consumed: `GameFeedback` (sounds + "going alone!"/"All five alone! Four points."/"Your team wins!" toasts; keyed dedupe), `GameAnnouncer` (aria-live sentences, one `dealt` line per sync). Trick animation in `TableStage` is driven by the VIEW (`trick.winnerSeat` set / `trick.plays`), not by steps, using `TABLE_MOTION` timers (section 9). A reconnect (steps `[]`) must therefore snap to the correct view without animation; the client animates nothing from `steps: []`.

### 6.4 `applyOptimistic(view, move)` contract (euchre/optimistic.ts) - REQUIRED for any client that previews
Exact on `OPTIMISTIC_FIELDS = ['hand','handCounts','trick','kittyCount','passes','trump','makerSeat','aloneSeat','sittingSeat','upCardTurnedDown','turnedDownSuit']`; always sets `legal: []`; other fields stay as before. Cases (move comes from `view.legal`):
- `play`: remove card from `hand` (first occurrence; return view unchanged if not held), `handCounts[you]-1` (floor 0), append `{seat:you,card}` to `trick.plays`; `ledSuit` set by `effectiveSuit(card, trump)` if first play; `winnerSeat` computed with `trickWinner` when `plays.length >= (sittingSeat===null ? 4 : 3)` and ledSuit & trump non-null; `legal:[]`.
- `discard`: remove card, `handCounts[you]-1`, `kittyCount = upCardTurnedDown ? kittyCount : 4`, `legal:[]`.
- `pass`: `passes+1`; if `passes >= 4 && phase==='bid_round_1' && upCard`: `passes:0, kittyCount:4, upCardTurnedDown:true, turnedDownSuit: upCard[1]`; `legal:[]`.
- `orderUp{alone}`: `trump = upCard suit`; `makerSeat=you`; `aloneSeat = alone?you:null`; `sittingSeat = alone?partnerOf(you):null`; dealer handCount +1 unless dealer is the sitting seat; sitting handCount =0; if you are the dealer and not sitting out, append upCard to `hand`; `kittyCount = dealerSitsOut ? 4 : kittyCount`.
- `call{suit,alone}`: `trump=suit`, maker/alone/sitting as above, sitting handCount 0.
- `cut`: returns view unchanged (deal is server-seeded; never previewed).

### 6.5 Reconnect / resync summary
1. Transport drops: `table.isConnected` false; the UI shows "Reconnecting... Your table is saved." and disables controls (`!table.isConnected`). The last good view stays on screen.
2. Transport re-opens: `getParams` re-runs (fresh token; session refetched if within 10 s of expiry); server `onConnect` pushes `sync{v, view, steps:[]}`; the `isConnected` effect also calls `snapshot()`. Both converge on the highest `v`.
3. Missed events are lost; correctness comes from `v` ordering + snapshot, never from event replay.
4. 10 s visible-tab poll heals a silently dead socket.
5. Human turns never time out (armHumanLadder sets `turnDeadlineAt=null`; nudge/abandon timers are legacy no-ops).
6. A reload re-fetches the session and re-opens the actor; state is fully server-held.

---------------------------------------------------------------------------

## 7. Hand review (`protocol/hand-review.ts`, archive in euchreTable)

```ts
interface CompletedHandReview {
  handNo: number;
  dealerSeat?: Seat;                       // absent in old archives
  tricksWon?: readonly [number, number];   // absent in old archives
  upCard?: CardId | null;                  // absent in old archives
  trump: Suit | null;
  makerSeat: Seat | null;
  aloneSeat: Seat | null;
  tricks: readonly Trick[];                // hand.trickLog
  buried: readonly CardId[];
  unplayed: readonly CardId[];
  result: HandResult | null;
  delta: readonly [number, number] | null;
}
```
Rust: `dealer_seat`, `tricks_won`, `up_card` are `Option` with `skip_serializing_if` (and `up_card`: when present may be null, so use `Option<Option<CardId>>` or a custom serde; simplest: `#[serde(default, skip_serializing_if="Option::is_none")] up_card: Option<CardId>` accepting null as None on read).

Archive rules (`archiveHand`, index.ts:1033-1052; written to `kv` key `review/${handNo}` as `JSON.stringify(review)`, strings):
- Written at hand boundary: at `hand_score` entry (guard `lastJournaledHandNo !== handNo`), or for a thrown-in hand from the PRE-throw state (`before.hand`).
- `buried = [...kitty.slice(1) /* 3 cards */, ...(dealerDiscard ? [dealerDiscard] : upCard ? [upCard] : [])]`. So: after order-up with discard -> 3 kitty + the dealer's discard (4); after turn-down -> 3 kitty + the turned-down up-card (4); loner with dealer sitting -> dealerDiscard == upCard.
- `unplayed = Object.values(hand.hands).flat()` i.e. concatenation of `hands[0], hands[1], hands[2], hands[3]` in seat order of whatever cards remain; at normal hand end this is only the sitting partner's 5 dead cards; for a throw-in it is all 20 dealt cards.
- `tricks = hand.trickLog` (5 for a normal hand).
- Delivery: only through `getHandReview` AFTER the hand is over (handNo must exist in kv; the live hand has no archive until scored), so it can reveal hidden cards post-hoc but never live. The Rust port must keep this gating: never serve an archive for the in-progress hand.
- UI: `HandReview` lets the player choose hands `0..view.handNo`; current hand uses `view.trickLog` locally; "Could not load this hand. Please try again." on failure.

Example:
```json
{
  "handNo": 3, "dealerSeat": 2, "tricksWon": [2, 3], "upCard": "KH", "trump": "H",
  "makerSeat": 3, "aloneSeat": null,
  "tricks": [ { "index": 0, "ledSuit": "H", "plays": [ {"seat":3,"card":"JH"}, {"seat":0,"card":"9H"}, {"seat":1,"card":"QH"}, {"seat":2,"card":"AH"} ], "winnerSeat": 3 } ],
  "buried": ["9C","TD","QS","KH"], "unplayed": [],
  "result": "euchre", "delta": [2, 0]
}
```
(Example shows one of five tricks for brevity; real payload has exactly 5 entries.)

---------------------------------------------------------------------------

## 8. Other wire shapes (server-internal and settings, for completeness)

These do not reach the desktop client during play but live in `protocol/index.ts` and must be ported with identical JSON because actors exchange them and the web client retargets later.

### 8.1 Actor-to-actor (euchreTable <-> aiSeat)
`InternalEnvelope`: `{ "internalToken": string, "gameId": string }` (constant-time compare before any state read).
`AIDecideRequest` (queue `decide`): envelope + `{ "seat": Seat, "turnId": string, "kind": "cut"|"bid1"|"discard"|"bid2"|"play", "view": PublicGameView, "legal": LegalMove[], "ranking"?: RankedMove[], "deadlineAt": number /*epoch ms*/ }`. `view` is `tableView(c, seat)` for the AI seat (same redaction as a human in that chair; AI never sees other hands).
`RankedMove`: `{ "id": LegalMoveId, "score": number, "why": string }`.
`AIDecision` (queue `aiDecision`): envelope + `{ "seat": Seat, "turnId": string, "moveId": LegalMoveId, "source": "llm"|"fallback"|"forced", "banter"?: string, "rationale"?: string, "latencyMs": number }`.
`AiSeatLifecycleMessage` (queues `resetHand` | `handEnd` | `gameEnd`): envelope + `{ "kind": "resetHand"|"handEnd"|"gameEnd", "handNo"?: number }` (handNo absent for gameEnd).
`AiSayMsg` (queue `aiSay`): envelope + `{ "seat": Seat, "msgId": string, "turnId": string, "handNo"?: number, "delta"?: string, "text"?: string, "final": bool }`.
`TickMsg` (queue `tick`): envelope + `{ "kind": "tempo"|"releaseAi"|"aiTimeout"|"nudge"|"abandon"|"flushProfile"|"reap", "turnId": string, "seat"?: Seat }`.
`MoveMsg` (queue `move`): `{ "seq": number, "turnId": string, "moveId": LegalMoveId, "clientMoveId": string, "userId": string }`. `ReviewMsg`: envelope + `{ "action": "set"|"continue", "enabled"?: bool, "turnId": string }`.

### 8.2 Personas (create input, `PersonaAssignment`)
```json
{ "seat": 1,
  "persona": { "id": "string", "version": 1, "name": "<=32", "blurb": "<=120", "prompt": "<=2000 UNTRUSTED", "housePrompt": "<=1000 UNTRUSTED",
               "difficulty": "rookie|casual|expert", "aggression": 0.5, "risk": 0.5, "chattiness": 0.5, "temperature": 0.5,
               "modelId": "claude-haiku-4-5|claude-opus-5", "bidModelId": "claude-haiku-4-5|claude-opus-5" },
  "dossier": "<=1600 chars" }
```
(Note: `modelId` is typed as an Anthropic union in protocol, but the project task states the AI uses OpenRouter "Jev"; the AI-lane spec owns model ids. The wire field names stay.) `PersonaView` returned by `aiSeat.getPersona()`: `{id, version, name, blurb, seat, difficulty, chattiness, degraded}`.

### 8.3 Error codes (`ProtocolErrorCode`, machine codes on `UserError.code` and `MoveAck.code`)
`origin_not_allowed`, `invalid_token`, `forbidden`, `seat_mismatch`, `internal_token_mismatch`, `game_not_found`, `game_complete`, `duplicate_move`, `rate_limited`, `protocol_version_mismatch`, `invalid_persona`, `ai_unavailable`, `internal_error`. `ErrorCode = RuleCode | ProtocolErrorCode`. `ConnectionErrorCode` (client-side guard): `ACTOR_NOT_YET_CONNECTED`, `ACTOR_DISCONNECTED`. `ProtocolError = { code, message, retryable?: bool }`. Clients map `code` to copy; never parse `message`. Codes actually raised in the TS table: `origin_not_allowed`, `invalid_token`, `forbidden`, `seat_mismatch`, `protocol_version_mismatch`, `game_complete`, `internal_error`, `illegal_move` (UserError code, not in `ProtocolErrorCode` but in `RuleCode`), and rule codes via `MoveAck`. `rate_limited`, `duplicate_move`, `game_not_found`, `ai_unavailable` are declared but unused here.

### 8.4 History records (playerProfile; not needed by the in-game client)
`MatchRecord`, `HandJournalEntry` (SERVER-ONLY: contain `seed` and `deckOrder`), `MatchSummary`, `MatchListPage`, `LifetimeStats`, `PlayerProfileState`, `EpisodeRecord`, `MatchStats`, `TokenUsage` are defined in protocol/index.ts:734-907; the playerProfile spec lane owns them. Key rule for this lane: `seed`/`deckOrder` NEVER leave the server.

---------------------------------------------------------------------------

## 9. Pacing and tempo

### 9.1 Presentation timing shared by server hold and client animation (`protocol/pacing.ts`)
```ts
TABLE_MOTION = { play: 700, read: 1500, collect: 780, stagger: 45 };   // ms
TRICK_PRESENTATION_MS = play + read + collect + 3*stagger + 185 = 700+1500+780+135+185 = 3300
```
Client use (TableStage): played-card flight `TABLE_MOTION.play` (700) with easing `cubic-bezier(.22,.7,.25,1)`; after a completed trick (non-review mode) wait `play + read` = 2200 ms then run `collect`: each card animates `TABLE_MOTION.collect` (780) with `delay = index * stagger` (45), easing `cubic-bezier(.4,0,.2,1)`; then trick-take sound. The server holds `trick_resolve` for `TRICK_PRESENTATION_MS` (3300) so the next card does not exist until the animation ends.

### 9.2 Server tempo policy (`euchre-table/tempo.ts`); `EUCHRE_FAST_TEMPO=1` (env, opt-in) selects the fast column

| Constant | Prod | Fast | Meaning |
|---|---|---|---|
| `trickResolveMs` | 3300 | 120 | hold in `trick_resolve` |
| `trickResolveSealsEuchreMs` | 3800 (`TRICK_PRESENTATION_MS+500`) | 180 | hold when `trickWon.sealsEuchre` |
| `handScoreMs` | 2500 | 250 | hold in `hand_score` |
| `thinkFloorMs.cut` | 700 | 80 | AI minimum think time |
| `thinkFloorMs.bid1` | 1900 | 120 | |
| `thinkFloorMs.bid2` | 1900 | 120 | |
| `thinkFloorMs.discard` | 1100 | 120 | |
| `thinkFloorMs.play` | 1000 | 100 | |
| `thinkJitterMs` | 400 | 40 | jitter width: `fnv1a(turnId + "#pace") % thinkJitterMs`, added to the floor (deterministic) |
| `aiHardCapMs` | 4000 | 1500 | watchdog; also `deadlineAt = now + aiHardCapMs` in `AIDecideRequest` |
| `aiExtendedAtMs` | 2200 | 800 | when `thinking.extended` should be raised (unused in TS) |
| `nudgeMs` | 90000 | 30000 | legacy, unused |
| `abandonMs` | 240000 | 90000 | legacy, unused |
| `profileRetryMs` | 30000 | same | |
| `reapMs` | 30 days (2_592_000_000) | same | self-reap after game over once recorded |

`thinkFloorMs(kind, turnId) = TEMPO.thinkFloorMs[kind] + (fnv1a(turnId + "#pace") % TEMPO.thinkJitterMs)`. `sealsEuchre` is true when the defending team (`1 - teamOf(makerSeat)`) reaches exactly 3 tricks. (Comment in reduce.ts says "1100 ms"; the live value is 3800 ms prod.)

AI turn timeline (`dispatchAi` / `onAiDecision` / `releasePending`): on AI's turn the table (1) computes `legal`; if exactly 1 legal -> parks it (`source:"forced"`, no model call); else parks the heuristic fallback (`source:"fallback"`, `answered:false`); (2) sets `pending = {seat, turnId, requestedAt, revealAt = now + thinkFloor, decision, source, answered}`, `turnDeadlineAt = now + aiHardCapMs`; schedules `onAiTimeout` at `aiHardCapMs` and `releaseAiMove` at `revealAt`; persists; broadcasts `thinking{seat,on:true,extended:false}`; (3) sends `decide` to the aiSeat (bounded by `deadlineAt`). The model answer is accepted once (first wins), upgrades the parked move, and the move is released no earlier than `revealAt` and no later than the watchdog (watchdog -> parked fallback released with `force`, or `legalMoves()[0]` autoPlay with `kind:"auto", reason:"ai_timeout"` if nothing parked). A parked placeholder is NOT played early while the persona is still thinking (the think floor is presentation only).

---------------------------------------------------------------------------

## 10. Phase machine, per-phase step emission and what resting states exist

Resting phases a client can observe: `cutting`, `bid_round_1`, `dealer_discard`, `bid_round_2`, `trick_play`, `trick_resolve` (server-held), `hand_score` (server-held), `game_over`. `lobby`/`deal` are transient: the actor's `settle()` advances through them inside the same actor message (creation: run loop `reconcile` -> `settle` -> `startHand`).

| Action | Steps emitted in the single resulting `sync` | New phase |
|---|---|---|
| cut (`cut:yes|no`) | `cut`, `dealt`, `upCardTurned` | `bid_round_1`, `turnSeat = eldest` (= dealer+1) |
| round-1 pass (1st-3rd) | `bid` | `bid_round_1`, next seat |
| round-1 pass (4th, dealer) | `bid` (say `Turn it down.`), `turnedDown` | `bid_round_2`, `turnSeat = eldest`, `passes=0`, `upCardTurnedDown=true`, `turnedDownSuit` |
| `orderUp[+alone]` | `bid`, `trumpSet{viaOrderUp:true}` ; if the dealer is the sitting partner: also `dealerDiscarded` and phase jumps straight to `trick_play` | `dealer_discard` (`turnSeat=dealer`; dealer hand becomes 6 cards, up-card appended) |
| `discard:<c>` | `dealerDiscarded{seat}` | `trick_play`, `turnSeat = openingLead` (eldest, or next active seat if eldest sits out) |
| round-2 pass | `bid` | next seat; 4th pass with stickTheDealer off: `bid`, `throwIn`, then fresh-hand steps (empty; new `cutting`) |
| `call:<suit>[+alone]` | `bid`, `trumpSet{viaOrderUp:false}` | `trick_play` |
| `play:<c>` (not last of trick) | `cardPlayed` | next active seat |
| `play:<c>` (last of trick: 4th card, or 3rd when a loner) | `cardPlayed`, `trickWon` | `trick_resolve`, `turnSeat=null`, `trick.winnerSeat` set, `trickLog` appended, `tricksWon[team]++` |
| tempo release from `trick_resolve` (trickLog<5) | (none: `steps: []`) | `trick_play`, `turnSeat = trick winner`, new empty trick `index+1` |
| tempo release from `trick_resolve` (5th trick) | `handScored` | `hand_score`, `result`/`delta` set, `score` updated, `winnerTeam` set if `score >= gameTo` |
| tempo release from `hand_score` | `gameWon` if `winnerTeam !== null` (status -> `complete`, phase `game_over`) else (none) and next hand opens (`cutting`, `dealerSeat+1`, `handNo+1`) | |

Important: tempo releases emit steps `[]` for `trick_resolve -> trick_play`; so the client MUST derive "trick collected / next trick begins" from view diffs (`trick.index`, `phase`), not from steps. Note that on the 5th trick the fifth completed trick stays in `hand.trick` during `hand_score`.

`stickTheDealer` sub-state of `bid_round_2`: `cfg.stickTheDealer && turnSeat===dealer && passes===3` -> `pass` removed from `legal`; a `pass` anyway is rejected `dealer_must_call`. The view exposes `rules.stickTheDealer` and `passes`.

Loner: when `aloneSeat` is set, `sittingSeat = partner(maker)`; all four-vs-three counts: tricks complete at 3 plays; `handCounts[sitting]=0`, `hand=[]` for the sitting viewer, `legal=[]` and `sitting_out` rule code for it; `turnSeat` skips the sitting seat; opening lead = `firstActiveFrom(eldest, sitting)`. Dealer-sitting special case: the dealer never picks up; `dealerDiscard` is set to the up-card immediately and a `dealerDiscarded` step follows `trumpSet`.

Scoring table is owned by the engine lane (`score.ts`): the wire carries only `result`, `delta` (post-clamp) and `score`.

---------------------------------------------------------------------------

## 11. A typical trick from the client's point of view (exact message sequence)

Scenario: trump decided; human (seat 0) is on lead, dealer = seat 3, maker = seat 1; tempo = production; no review mode. v values are illustrative.

Round trip legend: `C->S` action, `S->C` event.

```
t=0      S->C  sync   v=57 view{phase:"trick_play", turnSeat:0, turnId:"T57", legal:[5 play:* entries], trick:{index:0,ledSuit:null,plays:[],winnerSeat:null}}  steps:[]  (or steps from the previous transition)
t=u      (UI) user selects JH; optimistic = applyOptimistic(view, {t:"play",card:"JH"}) shown with forV=57
         C->S  submitMove({moveId:"play:JH", turnId:"T57", clientMoveId:"<uuid>"})
         [server] apply -> commit v=58 -> persist -> emit chat(none) -> sync -> ack
         S->C  sync   v=58 view{turnSeat:1, turnId:"T58", legal:[], trick.plays:[{seat:0,card:"JH"}], trick.ledSuit:"H", hand:[4 cards]} steps:[{t:"cardPlayed",seat:0,card:"JH"}]
         S->C  (action result) {ok:true, v:58}            // after the sync was already sent on the same ordered socket
         [server armTurn -> dispatchAi(seat 1)]
         S->C  thinking {seat:1, on:true, extended:false} // broadcast right after the parked decision persisted
t=+1000..1400ms (think floor + jitter, or until model answers; hard cap 4000ms)
         [optional] S->C chat {msgId:"<gameId>:<turnId>:1", seat:1, kind:"banter", text:"...", final:true}   // before the move's sync
         S->C  thinking {seat:1, on:false, extended:false}
         S->C  sync   v=59 steps:[{t:"cardPlayed",seat:1,card:"QH"}]  turnSeat:2
         S->C  thinking {seat:2,on:true,...}  ... sync v=60 (seat 2) ... thinking seat 3 ... sync v=61
         (4th card)
         S->C  sync   v=61 view{phase:"trick_resolve", turnSeat:null, legal:[], trick:{index:0,ledSuit:"H",plays:[4],winnerSeat:3}, trickLog:[that trick], tricksWon:[0,1]}  steps:[{t:"cardPlayed",seat:3,card:"JD"},{t:"trickWon",index:0,winnerSeat:3,plays:[4],sealsEuchre:false}]
t=+700ms client card flight ends; t=+2200ms client collects cards (780ms + 45ms stagger)
t=+3300ms (server tempo gate -> tick -> advance)
         S->C  sync   v=62 view{phase:"trick_play", turnSeat:3, turnId:"T62", trick:{index:1,ledSuit:null,plays:[],winnerSeat:null}, trickLog:[1 trick], tricksWon:[0,1]}  steps:[]
         (seat 3 is AI) S->C thinking {seat:3,on:true}  ...
```
Key facts for the client: (1) `submitMove` ack is delivered after the matching `sync`, so an `ack.v` is normally already <= `store.view.v`; the `resync()` after ack fires only when it is not; (2) turn changes are visible only via `view.turnSeat/turnId` (not via a dedicated event); (3) the human's next `legal` becomes non-empty again on the sync that sets `turnSeat===you`; (4) with `reviewTricks` on, after the trick lands the server does NOT advance at +3300 ms: it re-commits and sends `sync v=62 steps:[]` with `awaitingTrickReview:true`, waits for `reviewTrick({action:"continue", turnId})`, answers `{queued:true}`, then advances and sends `sync v=63 steps:[]`.

### Other canonical sequences

Deal/cut (human is cutter when dealer = 1; cutter = dealer+3 mod 4; with dealer 3 the cutter is seat 2 (AI)):
```
S->C sync steps:[]   phase:"cutting", turnSeat: cutter, legal: [cut:yes, cut:no] (only if cutter==0)
C->S submitMove({moveId:"cut:no", ...})  ->  S->C sync steps:[cut, dealt(8 packets, only own packets have cards) <!-- critic: was 4; dealPlan (deal.ts:171-181) returns two passes of 4 = 8 packets -->, upCardTurned]  phase:"bid_round_1", turnSeat: dealer+1
```
Order-up where the human is dealer:
```
(AI) sync steps:[bid{say:"I assist."}, trumpSet{viaOrderUp:true}] phase:"dealer_discard", turnSeat:0, hand: 6 cards incl. upCard, legal: 6 x discard:*
C->S submitMove("discard:9C") -> sync steps:[dealerDiscarded{seat:0}] phase:"trick_play", hand: 5 cards, kittyCount:4
```
Chat sidecar for every bid: `chat{kind:"call", text: step.say}` arrives BEFORE the sync that carries the `bid` step.

---------------------------------------------------------------------------

## 12. JSON examples of every payload

### 12.1 Full `PublicGameView` (human on lead in trick 1; dealer 3 picked up the up-card)
```json
{
  "players": [
    { "seat": 1, "name": "Dot", "blurb": "Plays tight.", "style": "Card counter" },
    { "seat": 2, "name": "Ray", "blurb": "Steady hands.", "style": "Steady player" },
    { "seat": 3, "name": "Moe", "blurb": "Loves a gamble.", "style": "Bold bidder" }
  ],
  "observations": [
    { "handNo": 2, "seat": 0, "kind": "bid1", "action": "Pass", "elapsedMs": 1750, "timing": "human-observed" },
    { "handNo": 2, "seat": 1, "kind": "bid1", "action": "I assist", "elapsedMs": null, "timing": "computer-paced" },
    { "handNo": 2, "seat": 3, "kind": "discard", "action": "Dealer discarded a private card", "elapsedMs": null, "timing": "computer-paced" }
  ],
  "reviewTricks": false,
  "rules": { "stickTheDealer": false, "requireNaturalTrump": false },
  "awaitingTrickReview": false,
  "v": 57,
  "you": 0,
  "status": "active",
  "phase": "trick_play",
  "handNo": 2,
  "dealerSeat": 3,
  "turnSeat": 0,
  "turnId": "k2ab9x1c4d0q8z",
  "score": [3, 5],
  "trump": "H",
  "makerSeat": 1,
  "aloneSeat": null,
  "sittingSeat": null,
  "upCard": "KH",
  "upCardTurnedDown": false,
  "turnedDownSuit": null,
  "hand": ["9S", "TC", "JH", "QD", "AH"],
  "handCounts": [5, 5, 5, 5],
  "kittyCount": 4,
  "trick": { "index": 0, "ledSuit": null, "plays": [], "winnerSeat": null },
  "trickLog": [],
  "tricksWon": [0, 0],
  "passes": 1,
  "bids": [
    { "seat": 0, "move": { "t": "pass" }, "say": "Pass." },
    { "seat": 1, "move": { "t": "orderUp", "alone": false }, "say": "I assist." }
  ],
  "legal": [
    { "id": "play:9S", "move": { "t": "play", "card": "9S" }, "label": "Nine of spades" },
    { "id": "play:TC", "move": { "t": "play", "card": "TC" }, "label": "Ten of clubs" },
    { "id": "play:JH", "move": { "t": "play", "card": "JH" }, "label": "Jack of hearts" },
    { "id": "play:QD", "move": { "t": "play", "card": "QD" }, "label": "Queen of diamonds" },
    { "id": "play:AH", "move": { "t": "play", "card": "AH" }, "label": "Ace of hearts" }
  ],
  "result": null,
  "delta": null,
  "winnerTeam": null
}
```

### 12.2 `SyncEvent` (after the human plays JH)
```json
{
  "v": 58,
  "view": { "...": "same shape as 12.1 with v=58, turnSeat=1, turnId='n7p0c3...', hand=['9S','TC','QD','AH'], handCounts=[4,5,5,5], trick={index:0,ledSuit:'H',plays:[{seat:0,card:'JH'}],winnerSeat:null}, legal=[]" },
  "steps": [ { "t": "cardPlayed", "seat": 0, "card": "JH" } ]
}
```
(The `view` placeholder is illustrative; the wire carries a full PublicGameView.)

### 12.3 Deal `sync.steps` as received by seat 0 (cut + deal; dealer 3; packets 3-2/2-3 pattern as produced by `dealtPackets`)
```json
[
  { "t": "cut", "seat": 2, "cut": false },
  { "t": "dealt", "packets": [
      { "seat": 0, "count": 3, "cards": ["9S", "TC", "JH"] },
      { "seat": 1, "count": 2, "cards": null },
      { "seat": 2, "count": 3, "cards": null },
      { "seat": 3, "count": 2, "cards": null },
      { "seat": 0, "count": 2, "cards": ["QD", "AS"] },
      { "seat": 1, "count": 3, "cards": null },
      { "seat": 2, "count": 2, "cards": null },
      { "seat": 3, "count": 3, "cards": null }
  ] },
  { "t": "upCardTurned", "card": "KH" }
]
```
(Packet order/counts are determined by `dealPlan` in the engine lane; only the receiver's own packets carry `cards`.) <!-- critic: example had only the first 4 packets; dealPlan always emits 8 (eldest first, then the complement pass). With dealer 3, eldest is seat 0; threeFirst = (hashSeed(seed,handNo) & 1) === 1 picks 3-2-3-2 then 2-3-2-3. Card ids in the second pass are illustrative. -->

### 12.4 Other steps
```json
{ "t": "bid", "seat": 1, "id": "orderUp", "say": "I assist." }
{ "t": "turnedDown", "suit": "H" }
{ "t": "trumpSet", "suit": "H", "makerSeat": 1, "aloneSeat": null, "sittingSeat": null, "viaOrderUp": true }
{ "t": "dealerDiscarded", "seat": 3 }
{ "t": "trickWon", "index": 0, "winnerSeat": 3, "plays": [ {"seat":0,"card":"JH"}, {"seat":1,"card":"QH"}, {"seat":2,"card":"9H"}, {"seat":3,"card":"AH"} ], "sealsEuchre": false }
{ "t": "handScored", "result": "march", "delta": [0, 2], "score": [3, 7], "makerSeat": 1, "aloneSeat": null, "tricksWon": [0, 5] }
{ "t": "throwIn" }
{ "t": "gameWon", "team": 1, "score": [8, 10] }
```

### 12.5 Events
```json
{ "seat": 1, "on": true,  "extended": false }                                                 // thinking
{ "msgId": "g-uuid:58:0", "seat": 1, "kind": "call", "text": "I assist.", "final": true }     // chat (script)
{ "msgId": "g-uuid:n7p0c3:2", "seat": 2, "kind": "banter", "text": "Nice lead.", "final": true } // chat (banter)
{ "msgId": "g-uuid:n7p0c3:2", "seat": 2, "delta": "Nice " }                                   // chatDelta
{ "seat": 0, "online": true }                                                                 // presence
```
(`g-uuid` stands for the real gameId UUID. `msgId` for scripts is `${gameId}:${v}:${stepIndex}`.)

### 12.5b Actions
```json
// submitMove request / acks
{ "moveId": "orderUp+alone", "turnId": "k2ab9x1c4d0q8z", "clientMoveId": "2d9a3c40-..." }
{ "ok": true, "v": 58 }
{ "ok": false, "code": "must_follow_suit", "legal": [ { "id": "play:9H", "move": {"t":"play","card":"9H"}, "label": "Nine of hearts" } ] }
// reviewTrick
{ "action": "set", "enabled": true }      -> { "queued": true }
{ "action": "continue", "turnId": "..." } -> { "queued": true }
// thrown UserError (illegal move id), as seen by a Rust client
{ "code": "illegal_move", "message": "That move is not legal right now", "metadata": { "legal": [ /* LegalMove[] */ ] } }
```

### 12.6 Connection params and snapshot
```json
{ "token": "<HS256 JWT>", "protocolVersion": 1 }
```
`snapshot()` -> a full `PublicGameView` (12.1).

---------------------------------------------------------------------------

## 13. Rust implementation notes (crate `protocol`)

1. Use one `serde` model per TS type above with `camelCase`. Provide `#[serde(default)]` on every `T?`. For `Option<T>` that is "always present, may be null" do NOT skip-serialize.
2. `Seat` as `#[derive(Serialize_repr, Deserialize_repr)] #[repr(u8)] enum` or `u8` newtype with `TryFrom` validation 0..=3 (reject out-of-range on deserialize). `Team` same with 0|1.
3. `LegalMoveId`: store as `String`; provide parse/format helpers (`pass`, `orderUp[+alone]`, `call:S[+alone]`, `cut:yes|no`, `discard:XX`, `play:XX`). Compare on exact string; never normalize.
4. `CardId`: newtype validated `[9TJQKA][SHDC]`.
5. Tuples: `[i32;2]`, `[u8;4]`.
6. `Step` and `Move`/`Bid` are internally tagged on `"t"` (see 1.2, 1.4). `MoveAck` is untagged-by-`ok`: model as an enum with a custom `Deserialize` that branches on `"ok"` (serde `tag = "ok"` only supports string tags; implement manually or use a struct with `ok: bool`, `v: Option<u64>`, `code: Option<String>`, `legal: Option<Vec<LegalMove>>`).
7. `SyncEvent.steps` default `[]`. Treat `view.players/observations/reviewTricks/rules/awaitingTrickReview` as optional.
8. Redaction: implement `project(state, seat) -> PublicGameView` and `project_steps(steps, seat)` in the same crate/module as the engine so the server cannot send engine `GameState`; do not derive `Serialize` on `GameState`.
9. `v` as `u64`; `turnId` as `String`; timestamps (`since`, `deadlineAt`, `expiresAt`) as epoch-millis `u64`/`i64`.
10. Action arguments are positional JSON args; `submitMove` = 1 arg object; `getHandReview` = 1 arg number; `reviewTrick` = 1 arg object; `snapshot`/`lastTrick` = 0 args.
11. Desktop client store must replicate section 6: drop `view.v < seen.v`; keep last good view; snapshot on connect + 10 s poll while foregrounded (use the same `recovery-coordinator` idea if one exists in the Rust client); `submitting` guard; after a failed ack, resync and compare `turnId`; optimistic preview keyed by `forV`; chat dedupe by `msgId`, last 6.
12. Message ordering guarantees the client relies on: within one connection the transport is ordered; the server sends chat -> sync -> action-ack for a human move; AI move: banter chat (optional) -> `thinking off` -> call chat (for bids) -> sync.

---------------------------------------------------------------------------

## 14. Parity gaps and quirks worth knowing before porting

- `thinking.extended:true`, `chatDelta` and `presence` are contract-declared but only `thinking(false extended)`, `chat` and `presence` are emitted by the TS table; `chatDelta` comes through `aiSay` (aiSeat lane). The web UI consumes only `sync` and `chat`.
- `applySync` uses `<` (equal v accepted); docs say `<=`.
- `ConnRole 'internal'` connections receive no `sync` (`pushSync` and `onConnect` skip them).
- The inline `submitMove` path bypasses the durable `move` queue ("M2/serverless workaround", index.ts:1898-1911). The Rust actor should serialize `submitMove` with the run loop (single writer) but must keep the persist -> sync -> ack ordering.
- `players[].style` and `observations` are computed on every view (cheap); `awaitingTrickReview` depends on `reviewReadyTurnId`.
- The only state change that bumps `v` without changing `turnId` is review `set` and the review-ready gate.
- Token lifetime comment mismatch: ~15 min documented, 2 h minted (`signGuest` for `euchre-actors`); gateway token 900 s.
- `AIDecideRequest.view` includes the actor-added metadata fields.
- Table creation default for stick-the-dealer differs between engine (true) and `/play` (false unless `?stick=true`).
- `/games`, `/settings`, `/new` routes and `playerProfile` are outside this lane.
