# 01 - Rules Engine Porting Spec (`euchre-engine` Rust crate)

Source of truth: `/Users/brittianwarner/goods/euchre/src/lib/euchre/*.ts` (READ-ONLY). Where
`docs/02-GAME-RULES-ENGINE.md` and the TS code disagree, **the code wins** (the doc is partly stale; known
divergences are listed in section 17). All `file:line` references are to `/Users/brittianwarner/goods/euchre/src/lib/euchre/`
unless prefixed otherwise.

Golden vectors in this document were produced by running the real TS engine under `bun` (scripts kept outside the repo).
They are authoritative test fixtures for the Rust port (section 15).

## 0. Scope, purity, and what touches time/randomness/IO

* The engine is **pure and deterministic**. There is **no** `Date.now`, `Math.random`, network, filesystem, or env access
  anywhere in `src/lib/euchre/*` (`deal.ts:1-13`, `reduce.ts:14-23`). Randomness is a pure function of
  `(seed: string, handNo: number)` via FNV-1a + mulberry32. Time never enters: `turnDeadlineAt` is only a field the actor writes.
* Even the turn nonce (`turnId`) is a deterministic hash chain (`reduce.ts:233`), not random.
* The only non-determinism a caller may inject: `GameInit.internalToken` (production passes a cryptographically random value;
  default is derived from the seed) and `GameInit.firstDealer` override.
* Purity contract (V21): no argument is ever mutated; every returned value is freshly allocated. In Rust: take `&GameState`, return a new `GameState`.
* The reducer never reads or writes: `v`, `timerId`, `watchdogId`, `revealId`, `pending`, `recentMoveIds`, `appliedSeq`, `turnDeadlineAt`
  (actor-owned; `types.ts` GameState doc). `createGame` initialises them to `0 / null / null / null / null / [] / 0 / null`.
* TS-only artifacts that need no port: `Object.freeze`, `Object.setPrototypeOf`, `structuredClone` (tests only), `readonly` modifiers.

## 1. Recommended crate layout

```
crates/euchre-engine/src/
  lib.rs        // re-exports (section 14)
  types.rs      // Suit, Rank, Card, Seat, Team, SeatRole, GamePhase, GameStatus, HandResult, RuleCode, Bid, Move, LegalMoveId,
                //   LegalMove, ActionKind, AIDecisionSource, AutoReason, PlayerAction, Play, Trick, BidRecord, DealPacket,
                //   DealtPacket, Step, EngineConfig, HandState, PendingDecision, GameState, PublicGameView & friends
  cards.rs      // SUITS, RANKS, DECK, effective_suit, trump_rank, plain_rank, bowers_of, is_left_bower, same_color
  seats.rs
  deal.rs       // fnv1a, hash_seed, mulberry32, shuffle, cut_for_hand, deal_plan, deal_from, deal_hand ...
  legal.rs
  trick.rs
  score.rs
  reduce.rs     // create_game, start_hand, apply, advance, move_id, say_for_bid, RuleError
  invariants.rs
  optimistic.rs
  project.rs    // project(), project_steps()  (lives in TS index.ts:247-321) - see Open Questions re protocol crate
```

Hard rule from the TS source: `suitOf()` (printed suit) may only be compared inside `cards.rs`. Everything else must go through `effective_suit`.

## 2. Wire / serde conventions (for JSON parity with TS)

* All JSON field names are **camelCase** exactly as in the TS interfaces (listed per type below).
* Enum-like string unions serialize as the exact TS string: suits `"S"|"H"|"D"|"C"`, ranks `"9"|"T"|"J"|"Q"|"K"|"A"`.
* `CardId` = 2-char string `"<Rank><Suit>"`, e.g. `"JS"`, `"TC"`, `"9D"`, `"AH"`. Recommended Rust: `struct Card { rank: Rank, suit: Suit }`
  with custom `Serialize/Deserialize/Display/FromStr` as that 2-char string. `Ord`/hash derive freely, but **do not** sort cards with a derived
  order where TS sorts strings (tests `sort()` CardIds lexicographically only to compare multisets).
* `Seat` = integer 0..3 (serialize as JSON number). `Team` = 0|1. `SeatRole` = 0..3. Use `#[repr(u8)]` newtypes with validating Deserialize.
* Tuples `[number, number]` / `[number; 4]` / the 4-card kitty serialize as JSON arrays (`[u8;2]`, `[u8;4]`, `[Card;4]`).
* `null` fields: use `Option<T>` serialized **as `null`** (not omitted) for every field typed `T | null` in TS.
  Fields typed with TS `?` (truly optional) must be `#[serde(skip_serializing_if = "Option::is_none")]`: `EngineConfig.requireNaturalTrump`,
  `PlayerAction.source`, `PlayerAction.reason`, `PendingDecision.answered`, and in `PublicGameView`: `players`, `observations`, `reviewTricks`, `rules`, `awaitingTrickReview`.
* **Tagged unions** use discriminator field `"t"`: `#[serde(tag = "t")]` with per-variant `rename` (variant names below are the exact tag strings; camelCase
  of the Rust variant name matches for all of them: `pass`, `orderUp`, `call`, `cut`, `discard`, `play`; steps `cut`, `dealt`, `upCardTurned`, `bid`,
  `turnedDown`, `trumpSet`, `dealerDiscarded`, `cardPlayed`, `trickWon`, `handScored`, `throwIn`, `gameWon`).
* **GOTCHA - GamePhase strings**: `serde(rename_all="snake_case")` would turn `BidRound1` into `bid_round1`. Required strings are exactly
  `"lobby" "cutting" "deal" "bid_round_1" "dealer_discard" "bid_round_2" "trick_play" "trick_resolve" "hand_score" "game_over"` - use explicit `#[serde(rename = "...")]` on every variant.
* `HandResult`: `"point" "march" "lone_point" "lone_march" "euchre" "throw_in"` (snake_case rename works). `GameStatus`: `"active" "complete" "abandoned"`.
  `RuleCode`: `"not_your_turn" "wrong_phase" "illegal_move" "must_follow_suit" "card_not_in_hand" "bidding_closed" "dealer_must_call" "sitting_out" "stale_turn"`.
  `ActionKind`: `"human" "ai" "auto"`. `AIDecisionSource`: `"llm" "fallback" "forced"`. `AutoReason`: `"ai_timeout" "ai_unreachable" "abandon"`.
* `HandState.hands: Record<Seat, CardId[]>` serializes in TS as a JSON **object with string keys `"0".."3"`**, not an array (the persisted state; the tests in
  `house-rules.test.ts` abuse a JS array there, which a Rust port should not replicate). Internal Rust type: `[Vec<Card>; 4]`. If GameState is ever persisted/compared against TS,
  give it a custom serde to `{"0":[...],"1":[...],"2":[...],"3":[...]}`. `GameState` itself is server-internal (it carries `internalToken` and `seed`) and must never be on a client wire.
* Integer types: scores/counts fit `u8`; `handNo`, `v`, `appliedSeq`, `requestedAt`, `revealAt`, `turnDeadlineAt` use `u32`/`u64`/`i64` (timestamps ms since epoch).

## 3. Cards (`cards.ts`)

### 3.1 Types

```ts
type Suit = 'S'|'H'|'D'|'C';   // S,C black; H,D red
type Rank = '9'|'T'|'J'|'Q'|'K'|'A';
type CardId = `${Rank}${Suit}`;
```

### 3.2 Constants (frozen; order is load-bearing for seeded shuffles - `cards.ts:12-54`)

* `SUITS = ['S','H','D','C']`
* `RANKS = ['9','T','J','Q','K','A']`
* `DECK` (24, suit-major, rank ascending), exactly:
  `9S TS JS QS KS AS 9H TH JH QH KH AH 9D TD JD QD KD AD 9C TC JC QC KC AC`
* `CARD_IDS` is an alias of `DECK` (same array).
* `COLOR_MATE`: `S<->C`, `H<->D` (private).

### 3.3 Functions

| TS (`cards.ts`) | Rust | Contract |
|---|---|---|
| `rankOf(c)` :62 | `Card::rank()` | first char |
| `suitOf(c)` :69 | `Card::printed_suit()` | **printed** suit; only comparable inside cards.rs |
| `sameColor(a,b)` :85 | `same_color(a,b)` | `a==b || COLOR_MATE[a]==b` (reflexive) |
| `effectiveSuit(card, trump: Suit\|null)` :98 | `effective_suit(card, Option<Suit>) -> Suit` | if `trump` is Some and rank==J and printed suit == COLOR_MATE[trump] -> trump, else printed suit. `None` trump => printed suit always |
| `trumpRank(card, trump)` :113 | `trump_rank(card, Suit) -> u8` | 0 if `effective_suit != trump`. Jack: printed==trump -> 8 (right), else 7 (left). Else `A6 K5 Q4 T3 9 2` |
| `plainRank(card)` :129 | `plain_rank(card) -> u8` | `A6 K5 Q4 J3 T2 9 1` (rank only; jack below queen) |
| `bowersOf(trump)` :138 | `bowers_of(Suit) -> [Card;2]` | `[J<trump>, J<COLOR_MATE[trump]>]` = [right, left] |
| `isLeftBower(card, trump\|null)` :146 | `is_left_bower(card, Option<Suit>)` | false for None; else rank==J && printed==COLOR_MATE[trump] |

Derived facts (tested): with any trump, exactly 7 cards are trump (right J, left J, A K Q T 9 of trump); the same-colour suit has 5 remaining cards
(its jack is the left bower), each off-colour suit has 6. `trump_rank(c,t) > 0` iff `effective_suit(c,t)==t`. `effective_suit(c, None)==printed` for all 24.
Exactly one card per trump differs between printed and effective suit (the left bower).

## 4. Seats (`seats.ts`)

`Seat` 0 South (human), 1 West, 2 North (human's partner), 3 East; clockwise. Team 0 = {0,2}, team 1 = {1,3}. `SeatRole` relative to dealer: 0 dealer, 1 eldest, 2 dealer's partner, 3 third seat (cutter).

| TS | Rust | Formula |
|---|---|---|
| `HUMAN_SEAT` | `HUMAN_SEAT: Seat = 0` | |
| `SEATS` | `SEATS = [0,1,2,3]` | |
| `nextSeat(s)` | `next_seat` | `(s+1)&3` |
| `partnerOf(s)` | `partner_of` | `(s+2)&3` |
| `teamOf(s)` | `team_of` | `s&1` |
| `opponentTeam(t)` | `opponent_team` | `1-t` |
| `seatRole(s, dealer)` | `seat_role` | `(s - dealer) & 3` - **in Rust use `(s + 4 - dealer) & 3`** (JS `&` on negatives wraps; Rust unsigned underflow would panic) |
| `eldestOf(d)` | `eldest_of` | `nextSeat(d)` |
| `cutterOf(d)` | `cutter_of` | `(d+3)&3` |
| `biddingOrder(d)` | `bidding_order -> [Seat;4]` | eldest, +1, +2, +3 (dealer last) |
| `firstActiveFrom(s, sitting)` | `first_active_from` | `s == sitting ? nextSeat(s) : s` |
| `nextActiveSeat(s, sitting)` | `next_active_seat` | `n = nextSeat(s); n == sitting ? nextSeat(n) : n` (one skip suffices; total even when `s == sitting`) |
| `activeSeatCount(sitting)` | `active_seat_count` | `sitting==None ? 4 : 3` |
| `isActiveSeat(s, sitting)` | `is_active_seat` | `s != sitting` |

## 5. Deal, RNG, determinism (`deal.ts`) - EXACT algorithms

**The engine IS seeded and deterministic.** PRNG = **mulberry32** seeded by **FNV-1a(32-bit)** of a derived string. Seeds are arbitrary strings.
Rust must reproduce identical shuffles bit-for-bit.

### 5.1 fnv1a (`deal.ts:27`)

```ts
let h = 0x811c9dc5;
for (i in 0..s.length) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; }
return h >>> 0;
```
Rust: `h: u32 = 0x811c9dc5; for u in s.encode_utf16() { h ^= u as u32; h = h.wrapping_mul(0x01000193); }`.
**Important:** TS `charCodeAt` iterates **UTF-16 code units**, so non-ASCII seeds/gameIds must hash over `encode_utf16()`, NOT UTF-8 bytes. (Verified: `fnv1a("héllo")=4058363231`, `fnv1a("😀")=3409036472`.)

### 5.2 hashSeed (`deal.ts:41`)
`hash_seed(seed, hand_no) = fnv1a(format!("{seed}#{hand_no}"))` (hand_no formatted as decimal integer).

### 5.3 mulberry32 (`deal.ts:50`)

```ts
let t = a >>> 0;
return () => {
  t = (t + 0x6d2b79f5) >>> 0;
  let x = t;
  x = Math.imul(x ^ (x >>> 15), x | 1);
  x ^= x + Math.imul(x ^ (x >>> 7), x | 61);
  return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
};
```
Rust (all wrapping u32; the JS `x + imul(...)` is a double add then ToInt32, equal to wrapping u32 add):
```rust
t = t.wrapping_add(0x6d2b79f5);
let mut x = t;
x = (x ^ (x >> 15)).wrapping_mul(x | 1);
x ^= x.wrapping_add((x ^ (x >> 7)).wrapping_mul(x | 61));
((x ^ (x >> 14)) as f64) / 4294967296.0
```
Returns f64 in [0,1). Keep it f64 (not f32) so `floor(rnd*(i+1))` matches exactly.

### 5.4 shuffle (Fisher-Yates, `deal.ts:73`)
```ts
a = items.slice();
for (i = a.length-1; i > 0; i--) { j = Math.floor(rnd() * (i+1)); swap(a[i], a[j]); }
```
Consumes exactly `len-1` draws; descending i; swap a[i]<->a[j].

### 5.5 Hand-level derivations

| Fn | Definition |
|---|---|
| `handRng(seed, handNo)` :65 | `mulberry32(hashSeed(seed, handNo))` |
| `shuffleForHand(seed, handNo)` :131 | `shuffle(DECK, handRng(seed, handNo))` (consumes 23 draws; nothing else consumes from this stream) |
| `startsWithThree(seed, handNo)` :160 | `(hashSeed(seed, handNo) & 1) == 1` |
| `cutForHand(deck, seed, handNo)` :102 | `span = deck.len()-8` (=16 for 24 cards); `i = 4 + (fnv1a(format!("{seed}#{handNo}#cut")) % span)`; return `deck[i..] ++ deck[..i]`. **Not stream-positional** (restart-safe) |
| `cutAt(deck, rnd)` :89 | `i = 4 + floor(rnd()*(len-8))`, rotate. **Unused by the reducer**; port only if exported |
| `pickFirstDealer(rnd)` :113 | `floor(rnd()*4)` |
| first dealer in `createGame` | `pickFirstDealer(mulberry32(fnv1a(format!("{seed}#firstDealer"))))` (a dedicated derived stream; does not disturb per-hand shuffle) |
| `newDeck()` | mutable copy of DECK |
| `assertDeckOrder(d)` :140 | panics/Err if len != 24 (`deckOrder must hold 24 cards, got N`), duplicates (`deckOrder contains duplicate cards`), or missing card (`deckOrder is missing XX`) |

### 5.6 Deal plan (`deal.ts:171`)
```
order = [eldest, +1, +2, +3]           // eldest = nextSeat(dealer)
first  = order.map((seat,i) => { seat, count: ((i%2==0) == threeFirst) ? 3 : 2 })
second = first.map(p => { seat: p.seat, count: 5 - p.count })
plan   = first ++ second               // 8 packets, 20 cards
```
So `threeFirst=true` => counts `[3,2,3,2,2,3,2,3]`; `false` => `[2,3,2,3,3,2,3,2]`. Packet seat order is always `[e, e+1, e+2, e+3, e, e+1, e+2, e+3]`.

### 5.7 dealFrom / dealtPackets / dealHand
* `dealFrom(deckOrder, plan)` (:194): `assertDeckOrder`; error `deal plan must account for 20 cards, got N` if sum != 20. Walk packets in order, taking cards from the front of `deckOrder` into `hands[seat]` (hands are in **deal order, not sorted**). `kitty = [deck[20],deck[21],deck[22],deck[23]]`. `kitty[0]` is the up-card; `kitty[1..3]` are buried forever.
* `dealtPackets(deckOrder, plan)` (:225): same walk, producing `DealtPacket { seat, count, cards: Some(slice) }` (unredacted).
* `dealHand(deckOrder, dealerSeat, seed, handNo)` (:256): `plan = dealPlan(dealer, startsWithThree(seed, handNo))`; returns `DealResult { deckOrder, plan, hands, kitty, upCard: kitty[0] }`. Does not shuffle or cut.

## 6. Types (`types.ts`)

### 6.1 Small unions
Phases `GamePhase` (10), `GameStatus`, `HandResult`, `RuleCode`, `ActionKind`, `AIDecisionSource`, `AutoReason` as in section 2.
Engine-only phases (`turnSeat == null`, `legalMoves == []`): `lobby`, `deal`, `trick_resolve`, `hand_score`, `game_over`. Phases with an acting seat: `cutting`, `bid_round_1`, `dealer_discard`, `bid_round_2`, `trick_play`.
Stick-the-dealer is NOT a phase; it is a sub-state of `bid_round_2` (see `is_stuck_dealer`).

### 6.2 Moves

```ts
type Bid  = {t:'pass'} | {t:'orderUp', alone:boolean} | {t:'call', suit:Suit, alone:boolean};
type Move = Bid | {t:'cut', cut:boolean} | {t:'discard', card:CardId} | {t:'play', card:CardId};
```
`Move` has **no seat field** by construction.

`LegalMoveId` (string): `pass`, `orderUp`, `orderUp+alone`, `call:<S|H|D|C>`, `call:<suit>+alone`, `cut:yes`, `cut:no`, `discard:<CardId>`, `play:<CardId>`.

```ts
interface LegalMove { id: LegalMoveId; move: Move; label: string }
interface PlayerAction { seat: Seat; id: LegalMoveId; move: Move; kind: ActionKind; source?: AIDecisionSource /*only kind=ai*/; reason?: AutoReason /*only kind=auto*/ }
```

### 6.3 Tricks, bids, deal

| TS | JSON | Notes |
|---|---|---|
| `Play {seat, card}` | `{"seat":n,"card":"JS"}` | |
| `Trick {index, ledSuit: Suit\|null, plays: Play[], winnerSeat: Seat\|null}` | camelCase | `index` 0..4. `ledSuit` is the **effective** suit of the first play |
| `BidRecord {seat, move: Bid, say: string}` | | `say` is the verbatim table line |
| `DealPacket {seat, count}` | | |
| `DealtPacket extends DealPacket {cards: CardId[]\|null}` | | `null` when not addressed to receiver (projectSteps) |

### 6.4 Step (choreography channel; decoration, never truth)

| `t` | Fields |
|---|---|
| `cut` | `seat`, `cut: bool` |
| `dealt` | `packets: DealtPacket[]` |
| `upCardTurned` | `card` |
| `bid` | `seat`, `id: LegalMoveId`, `say` |
| `turnedDown` | `suit` |
| `trumpSet` | `suit`, `makerSeat`, `aloneSeat: Seat\|null`, `sittingSeat: Seat\|null`, `viaOrderUp: bool` |
| `dealerDiscarded` | `seat` (**deliberately no card field**) |
| `cardPlayed` | `seat`, `card` |
| `trickWon` | `index`, `winnerSeat`, `plays: Play[]`, `sealsEuchre: bool` |
| `handScored` | `result`, `delta: [n,n]`, `score: [n,n]`, `makerSeat: Seat\|null`, `aloneSeat: Seat\|null`, `tricksWon: [n,n]` |
| `throwIn` | (none) |
| `gameWon` | `team`, `score: [n,n]` |

### 6.5 EngineConfig (`types.ts:~140`; JSON camelCase)

| Field | Type | Default (`DEFAULT_ENGINE_CONFIG`) | Meaning |
|---|---|---|---|
| `deckVariant` | `"24"` | `"24"` | single code path |
| `gameTo` | number | **10** | points to win; 5/7/10/11 tested |
| `stickTheDealer` | bool | **true** | dealer may not pass in round 2 once 3 have passed |
| `requireNaturalTrump?` | bool | false | left bower alone does not qualify to order up / name trump (stuck dealer & round-1 dealer exempt) |
| `loner` | `"makerOnly"` | | only maker goes alone, only on the successful call |
| `defendAlone` | `false` | | no code path |
| `superEuchre` | `false` | | no code path (no 4-point row) |
| `farmersHand` | `"off"` | | no code path |
| `railroading` | `false` | | no code path |
| `dealerMustHoldTrump` | `false` | | no code path |
| `misdealLimit` | number | **3** | cap on `misdealStreak` |

Rust: model the constant-valued fields as bool/`String`/enum with a single variant so JSON round-trips; `Partial<EngineConfig>` overrides => a `EngineConfigOverrides` struct of `Option`s (only `Some` overrides; unlike JS spread, an absent key never overwrites).
Config knobs that actually branch: `gameTo`, `stickTheDealer`, `requireNaturalTrump`, `misdealLimit`. There is **no defend-alone and no per-team loner variation**; there is no config for farmers hand etc.

### 6.6 HandState

| Field | Type | Notes |
|---|---|---|
| `handNo` | number | 0-based |
| `phase` | GamePhase | |
| `dealerSeat` | Seat | |
| `turnSeat` | Seat\|null | null in engine-only phases |
| `deckOrder` | CardId[24] | persisted permutation (post-cut after a bump) |
| `cut` | bool\|null | null until cutter answers; true=bump |
| `hands` | Record<Seat, CardId[]> | all four hands (server-only) |
| `kitty` | [CardId x4] | `deckOrder[20..23]`; kitty[0] = up-card |
| `upCard` | CardId\|null | = kitty[0] once dealt; **never nulled** (V14) |
| `upCardTurnedDown` | bool | true after round-1 four-pass |
| `turnedDownSuit` | Suit\|null | printed suit of the turned-down up-card |
| `trump` | Suit\|null | |
| `makerSeat` | Seat\|null | |
| `aloneSeat` | Seat\|null | non-null => `== makerSeat` |
| `sittingSeat` | Seat\|null | `== partnerOf(aloneSeat)` |
| `dealerDiscard` | CardId\|null | never revealed |
| `passes` | number | within current round; reset to 0 on entering round 2; <=4 |
| `bids` | BidRecord[] | <=8 |
| `trick` | Trick | current (or just-completed during `trick_resolve`/`hand_score`) |
| `trickLog` | Trick[] | completed, oldest first; <=5 |
| `tricksWon` | [n,n] | by team |
| `result` | HandResult\|null | set at hand_score |
| `delta` | [n,n]\|null | post-clamp |

### 6.7 GameState

`schema: 1`, `v: number`, `gameId: string`, `internalToken: string`, `seed: string`, `cfg: EngineConfig`, `firstDealer: Seat`, `status: GameStatus`, `score: [n,n]`,
`handNo: number` (increments on every scored hand AND every throw-in, **but not after the final scored hand**), `misdealStreak: number` (consecutive throw-ins, capped at `misdealLimit`),
`winnerTeam: Team|null`, `hand: HandState`, `turnId: string`, `turnDeadlineAt: number|null`, `timerId: string|null`, `watchdogId: string|null`, `revealId: string|null`,
`pending: PendingDecision|null`, `recentMoveIds: string[]` (ring, cap 32 - actor-owned), `appliedSeq: number`.

`PendingDecision { seat, turnId: string, requestedAt: number, revealAt: number, decision: LegalMoveId|null, source: AIDecisionSource|null, answered?: boolean }` (actor-owned; port the type, no engine logic).

### 6.9 PublicGameView and friends (also exported from the engine)

```ts
TableObservation { handNo, seat, kind: 'cut'|'bid1'|'bid2'|'discard'|'play', action: string, elapsedMs: number|null, timing: 'human-observed'|'computer-paced'|'unreliable' }
TablePlayer { seat, name, blurb, style }
PublicGameView {
  players?: TablePlayer[]; observations?: TableObservation[]; reviewTricks?: boolean;
  rules?: { stickTheDealer: bool, requireNaturalTrump: bool }; awaitingTrickReview?: boolean;
  v, you: Seat, status, phase, handNo, dealerSeat, turnSeat: Seat|null, turnId: string, score: [n,n],
  trump, makerSeat, aloneSeat, sittingSeat, upCard, upCardTurnedDown, turnedDownSuit,
  hand: CardId[], handCounts: [n,n,n,n], kittyCount: number,
  trick: Trick, trickLog: Trick[], tricksWon: [n,n], passes: number, bids: BidRecord[],
  legal: LegalMove[], result: HandResult|null, delta: [n,n]|null, winnerTeam: Team|null }
```
The pure `project()` leaves `players/observations/reviewTricks/rules/awaitingTrickReview` **absent** (actor adds them). **Never add a field capable of carrying a foreign CardId** (security boundary).

## 7. Naming & labels (`legal.ts:22-74`) - exact strings

| Const | Values |
|---|---|
| `SUIT_NAME` | S "Spades", H "Hearts", D "Diamonds", C "Clubs" |
| `SUIT_NOUN` | S "spade", H "heart", D "diamond", C "club" |
| `RANK_NAME` | 9 "Nine", T "Ten", J "Jack", Q "Queen", K "King", A "Ace" |
| `cardName(c)` | `"{RANK_NAME} of {SUIT_NAME lowercased}"` e.g. `"Jack of diamonds"` (printed card, never effective) |
| `cardNameLower(c)` | `"jack of diamonds"` |
| `orderLabel(seat, dealer)` | seat==dealer -> `"I take it"`; seat==partnerOf(dealer) -> `"I assist"`; else `"Order it up"` |
| `joinList(items)` | `[]`->`""`, `[a]`->`a`, `[a,b]`->`a and b`, `[a,b,c]`->`a, b and c` |

## 8. Legal moves (`legal.ts`)

### 8.1 getLegalPlays(hand, ledSuit: Option<Suit>, trump: Option<Suit>) -> Vec<Card> (:93)
`ledSuit == None` -> whole hand (copy). Else `following = hand.filter(effective_suit(c,trump) == ledSuit)`; non-empty -> `following` (hand order preserved) else the whole hand. No obligation to trump or overtrump. Never empty for a non-empty hand.

### 8.2 whyIllegal(hand, card, ledSuit, trump) -> Option<String> (:114)
1. `!hand.contains(card)` -> `"That card isn't in your hand."`
2. `ledSuit == None` -> None
3. `effective_suit(card,trump) == ledSuit` -> None
4. `following` = cards in hand whose effective suit == ledSuit; empty -> None.
5. `opener = "{SUIT_NAME[ledSuit]} were led."`
6. If some `following` card is the left bower: `"{opener} Your {cardName(bower)} is a {SUIT_NOUN[ledSuit]} right now — you have to follow."` (em dash U+2014; first left bower found in hand order)
7. Else `"{opener} You still hold {joinList(following.map(c => "the " + cardNameLower(c)))} — you have to follow suit."`
Example (verbatim, doc test): `whyIllegal(["JD","9C"],"9C","H","H") == "Hearts were led. Your Jack of diamonds is a heart right now — you have to follow."`

### 8.3 isStuckDealer(state, seat) (:148)
`cfg.stickTheDealer && hand.phase=='bid_round_2' && seat==hand.dealerSeat && hand.passes==3`.

### 8.4 legalMoves(state, seat) -> Vec<LegalMove> (:172) - the single source of truth
Returns `[]` if `state.status != active`, or `seat == hand.sittingSeat`, or `hand.turnSeat != Some(seat)`. Otherwise by phase (order of entries is part of the contract):

* **cutting**: `[ {id:"cut:yes", move:{t:cut,cut:true}, label:"Bump"}, {id:"cut:no", move:{t:cut,cut:false}, label:"Run 'em"} ]` (apostrophe is ASCII `'`).
* **bid_round_1**: `order = orderLabel(seat, dealer)`.
  * **requireNaturalTrump gate**: if `cfg.requireNaturalTrump && upCard.is_some() && seat != dealer && !hands[seat].any(|c| c.printed_suit == upCard.printed_suit)` -> `[ {pass, "Pass"} ]` only. (Compares **printed** suit via `c[1]`; the left bower does not count. Dealer exempt.)
  * Else `[ {"pass", {t:pass}, "Pass"}, {"orderUp", {t:orderUp,alone:false}, order}, {"orderUp+alone", {t:orderUp,alone:true}, "{order}, alone"} ]`.
* **dealer_discard**: one entry per card in `hands[seat]` **in hand order**: `{id:"discard:<card>", move:{t:discard,card}, label:cardName(card)}` (six cards incl. the up-card).
* **bid_round_2**: start with `[ {pass,"Pass"} ]` unless `isStuckDealer(state, seat)` (then empty). For each suit in `SUITS` order `S,H,D,C`: skip `turnedDownSuit`; skip if `requireNaturalTrump && !isStuckDealer && !hands[seat].any(|c| c.printed_suit == s)`; push `{id:"call:<s>", move:{t:call,suit:s,alone:false}, label:SUIT_NAME[s]}` then `{id:"call:<s>+alone", move:{t:call,suit:s,alone:true}, label:"{SUIT_NAME[s]}, alone"}`. Normal count: 7 (pass + 3x2), stuck dealer: 6.
* **trick_play**: `getLegalPlays(hands[seat], trick.ledSuit, trump)` mapped to `{id:"play:<card>", move:{t:play,card}, label:cardName(card)}` (hand order).
* any other phase: `[]`.

V19: for any phase with a non-null `turnSeat` (and active seat) the set is never empty. Note requireNaturalTrump keeps `pass` in round 1 always, and in round 2 `pass` is present unless stuck-dealer (who is exempt from the natural-trump filter, so still has 6 calls).

### 8.5 helpers
`legalMoveIds(state, seat) -> Vec<String>` (same order); `findLegalMove(state, seat, id) -> Option<LegalMove>` (linear find by id string). Used by reducer backstop.

## 9. Trick logic (`trick.ts`)

* `cardValue(card, led: Suit, trump: Option<Suit>) -> u8` (:29): `eff = effective_suit(card, trump)`; if trump is Some and `eff == trump` -> `100 + trump_rank(card, trump)` (102..108); else if `eff == led` -> `50 + plain_rank(card)` (51..56); else `0`.
  A card that is neither trump nor led cannot win (so `9C` beats `AH` when clubs led and diamonds trump). Ties among positive values are impossible over 24 distinct cards.
* `trickWinner(plays, led, trump: Suit) -> Seat` (:44): panics (plain Error) on empty plays. `best = plays[0]`; iterate `i>=1`; replace only if `v > bestValue` (**strictly greater** - zero-value ties keep the earlier play; "the first play holds").
* `ledSuitOf(plays, trump: Option<Suit>) -> Option<Suit>` (:64): `None` if empty else `effective_suit(plays[0].card, trump)`.
* `playsPerTrick(sitting)` = `activeSeatCount` (4 or 3). `isTrickComplete(plays, sitting)` = `plays.len() >= playsPerTrick`.
* `currentlyWinning(plays, trump: Option<Suit>) -> Option<Play>` (:88): `led = ledSuitOf`; `None` if none; else the same strictly-greater scan as `trickWinner` (never disagrees with it on any prefix).

## 10. Scoring (`score.ts`)

Table (`scoreHand`, :41). `makers = teamOf(makerSeat)`, `defenders = 1-makers`, `made = tricksWon[makers]`, `alone = aloneSeat != null` (only null-vs-non-null matters):

| made | alone | HandResult | raw points to |
|---|---|---|---|
| 0,1,2 | either | `euchre` | **2 to defenders** |
| 3,4 | false / true | `point` / `lone_point` | 1 to makers |
| 5 | false / true | `march` / `lone_march` | **2 / 4** to makers |

No super-euchre row, no defend-alone row. Then clamp: `next[i] = min(gameTo, score[i] + raw[i])`; return `ScoreOutcome { result, delta: [next[0]-score[0], next[1]-score[1]] }` (**delta is post-clamp**: a march at 9 gives delta 1 landing on 10; lone march at 7 -> delta 3; a team already at gameTo gains 0; euchre at 9 clamps to 1).
* `throwInOutcome()` -> `{ result: throw_in, delta: [0,0] }` (fresh each call). **Not called by the reducer** (reducer builds the throw-in path itself).
* `applyDelta(score, delta)` -> componentwise sum (fresh tuple).
* `winnerTeam(score, gameTo)` -> `0` if `score[0] >= gameTo`, else `1` if `score[1] >= gameTo`, else None.
* `isGameOver` = winnerTeam != None. `isInTheBarn(score, team, gameTo)` = `score[team] == gameTo-1`. `isEuchre(result)` = `result == euchre`.
* `rawPoints(result)`: point 1, lone_point 1, march 2, euchre 2, lone_march 4, throw_in 0.

## 11. The reducer (`reduce.ts`) - full state machine

### 11.1 Public API (Rust signatures)

```rust
pub struct ApplyResult { pub state: GameState, pub steps: Vec<Step> }
pub struct ApplyOptions { pub turn_id: Option<String> }
pub struct GameInit { pub game_id: String, pub seed: String, pub cfg: Option<EngineConfigOverrides>, pub internal_token: Option<String>, pub first_dealer: Option<Seat> }
pub struct RuleError { pub code: RuleCode, pub legal: Vec<LegalMove> }   // Display = code string; std::error::Error
pub fn create_game(init: GameInit) -> GameState;
pub fn start_hand(state: &GameState) -> ApplyResult;
pub fn apply(state: &GameState, seat: Seat, mv: Move, opts: Option<&ApplyOptions>) -> Result<ApplyResult, RuleError>;
pub fn advance(state: &GameState) -> ApplyResult;
pub fn move_id(mv: &Move) -> LegalMoveId;
pub fn say_for_bid(state: &GameState, seat: Seat, mv: &Bid) -> String;
pub fn new_game(seed: &str, cfg: Option<EngineConfigOverrides>) -> GameState;  // index.ts:204: createGame({gameId: format!("g-{seed}"), seed, cfg})
```
Internal `invariant(cond, msg)` failures throw a plain `Error("euchre engine invariant violated: <msg>")` - **a programmer bug, never a rules event**. In Rust, `panic!` or return a separate `EngineError` (do not fold into `RuleError`). `isRuleError` => `Result::Err(RuleError)` pattern.

### 11.2 Hash-chain turn nonce and token (deterministic)

```ts
mintTurnId(state) = fnv1a(`${state.internalToken}#${state.turnId}#a`).toString(36)
                  + fnv1a(`${state.internalToken}#${state.turnId}#b`).toString(36)     // concatenated, lowercase base36, no padding
deriveToken(seed, gameId) = concat for i in 0..4 of fnv1a(`${seed}#${gameId}#token#${i}`).toString(36)
initial turnId = fnv1a(`${seed}#turn0`).toString(36)
```
`next(state, changes)` = new state with `changes` applied and `turnId = mintTurnId(state)` (**mints from the OLD turnId**; every transition that returns via `next` advances the chain exactly once). Rust: `format_radix(u32, 36)`, digits `0-9a-z`, no leading zeros (`0` -> `"0"`).

### 11.3 createGame (:321)

* `cfg = DEFAULT_ENGINE_CONFIG` overlaid with overrides.
* `firstDealer = init.firstDealer ?? pickFirstDealer(mulberry32(fnv1a(`${seed}#firstDealer`)))`.
* `deckOrder = shuffleForHand(seed, 0)`.
* `hand` = `{handNo:0, phase:'lobby', dealerSeat:firstDealer, turnSeat:null, deckOrder, cut:null, hands:{0:[],1:[],2:[],3:[]}, kitty:kittyOf(deckOrder) /*provisional*/, upCard:null, upCardTurnedDown:false, turnedDownSuit:null, trump:null, makerSeat:null, aloneSeat:null, sittingSeat:null, dealerDiscard:null, passes:0, bids:[], trick:{index:0,ledSuit:null,plays:[],winnerSeat:null}, trickLog:[], tricksWon:[0,0], result:null, delta:null}`.
* state = `{schema:1, v:0, gameId, internalToken: init.internalToken ?? deriveToken(seed, gameId), seed, cfg, firstDealer, status:'active', score:[0,0], handNo:0, misdealStreak:0, winnerTeam:null, hand, turnId: fnv1a(`${seed}#turn0`).toString(36), turnDeadlineAt:null, timerId:null, watchdogId:null, revealId:null, pending:null, recentMoveIds:[], appliedSeq:0}`.

### 11.4 startHand(state) (:395) - `lobby | hand_score(next) -> cutting`
Idempotent given `state`: `dealerSeat = state.hand.dealerSeat`, `handNo = state.handNo`, `deckOrder = shuffleForHand(seed, handNo)`. New `HandState` as in createGame's hand but `phase:'cutting'`, `turnSeat: cutterOf(dealerSeat)`, `handNo`. Returns `{state: next(state,{hand}), steps: []}`. (Does NOT rotate the dealer; `rotateToNextHand` does.)

### 11.5 rotateToNextHand(state, misdealStreak) (:433)
`handNo' = state.handNo+1`; `next(state, {handNo', misdealStreak, hand: {...state.hand, handNo: handNo', dealerSeat: nextSeat(state.hand.dealerSeat)}})`. Then callers invoke `startHand` on that result (which therefore consumes a **second** `next()`/turnId mint: the final nonce is two mints after the pre-rotate state).

### 11.6 apply(state, seat, move, opts) (:506) - validation ladder (exact order)

`reject(code)` throws `RuleError(code, legalMoves(state, seat))` - i.e. `legal` is the legal set **of the claimed `seat`** (empty if not their turn).

Step A - who may speak:
1. `state.status != active` -> `wrong_phase`
2. `opts.turnId` is Some and `!= state.turnId` -> `stale_turn`
3. `seat == hand.sittingSeat` -> `sitting_out`
4. `hand.turnSeat == null` -> `wrong_phase`
5. `hand.turnSeat != seat` -> `not_your_turn`

Step B - does the move kind belong to the phase:
| phase | rule |
|---|---|
| `cutting` | `move.t != cut` -> `wrong_phase` |
| `bid_round_1` | `move.t` not in {pass, orderUp} -> `wrong_phase` (so `call` in round 1 is `wrong_phase`) |
| `bid_round_2` | `move.t` not in {pass, call} -> `wrong_phase` (so `orderUp` in round 2 is `wrong_phase`) |
| `dealer_discard` | bid move (pass/orderUp/call) -> `bidding_closed`; else `move.t != discard` -> `wrong_phase` |
| `trick_play` | bid move -> `bidding_closed`; else `move.t != play` -> `wrong_phase` |
| anything else | `wrong_phase` |

Step C - specific codes, in order:
1. `pass && isStuckDealer(state, seat)` -> **`dealer_must_call`** (never `bidding_closed`)
2. `call && move.suit == hand.turnedDownSuit` -> `illegal_move`
3. `discard && !hands[seat].contains(card)` -> `card_not_in_hand`
4. `play`: not in hand -> `card_not_in_hand`; then `!getLegalPlays(hands[seat], trick.ledSuit, trump).contains(card)` -> `must_follow_suit`.

Step D - backstop: `findLegalMove(state, seat, moveId(move)) == None` -> `illegal_move` (this is what rejects natural-trump violations such as a non-natural `orderUp`/`call`, and `orderUp` from a seat whose legal set was reduced to `[pass]`).

Step E - dispatch by `move.t`: cut -> `applyCut`; pass -> (`phase==bid_round_1` ? `applyPassRound1` : `applyPassRound2`); orderUp -> `applyOrderUp`; call -> `applyCall`; discard -> `applyDiscard`; play -> `applyPlay`.

### 11.7 moveId (:166) and isBidMove
`pass`->"pass"; `orderUp`->`alone ? "orderUp+alone" : "orderUp"`; `call`->`alone ? "call:<suit>+alone" : "call:<suit>"`; `cut`->`cut ? "cut:yes":"cut:no"`; `discard`->`"discard:<card>"`; `play`->`"play:<card>"`. `isBidMove` = t in {pass, orderUp, call}.

### 11.8 sayForBid(state, seat, bid) (:201) - verbatim table lines
`aloneSuffix = (bid is not pass && bid.alone) ? " Alone." : ""`.
* `pass`: `turningDown = phase=='bid_round_1' && seat==dealerSeat && passes==3` (passes **before** increment) -> `"Turn it down."` else `"Pass."`.
* `orderUp`: `"{orderLabel(seat, dealer)}.{aloneSuffix}"` e.g. `"I assist."`, `"Order it up. Alone."`, `"I take it."`.
* `call` with `isStuckDealer(state, seat)`: `"{SUIT_NAME[suit]}, I guess.{aloneSuffix}"` (even for the colour mate).
* `call` with `turnedDownSuit != null`: `sameColor(suit, turnedDownSuit)` (reflexive but the turned-down suit itself is unreachable) -> `"Next.{alone}"` else `"Crossing the creek.{alone}"`.
* `call` otherwise (unreachable in practice): `"{SUIT_NAME[suit]}.{alone}"`.
Note the alone suffix is `" Alone."` appended AFTER the terminating period, e.g. `"Next. Alone."`, `"Hearts, I guess. Alone."`.

### 11.9 Transitions

Common helpers: `recordBid` appends `{seat, move, say}` to `bids`; `bidStep(seat, move, say) = {t:'bid', seat, id: moveId(move), say}`; `emptyTrick(i) = {index:i, ledSuit:null, plays:[], winnerSeat:null}`; `removeCard` removes the **first** occurrence (invariant failure if absent); `openingLead(h) = firstActiveFrom(eldestOf(h.dealerSeat), h.sittingSeat)`.

**applyCut(state, seat, cut)** (:593) - `cutting -> deal -> bid_round_1` atomically.
* `deckOrder = cut ? cutForHand(h.deckOrder, seed, h.handNo) : h.deckOrder`.
* intermediate state `afterCut = next(state, {hand:{...h, cut, deckOrder, kitty:kittyOf(deckOrder), phase:'deal', turnSeat:null}})`.
* `dealt = runDeal(afterCut)`. Result: `state = dealt.state` (two `next()` mints total), `steps = [{t:'cut', seat, cut}, ...dealt.steps]`.

**runDeal(state)** (:451, also reachable via `advance` in phase `deal`): `{plan,hands,kitty,upCard} = dealHand(h.deckOrder, h.dealerSeat, seed, h.handNo)`; hand becomes `phase:'bid_round_1'`, `turnSeat: eldestOf(dealer)`, `hands`, `kitty`, `upCard`, `passes:0`, `bids:[]`, `trick: emptyTrick(0)` (other fields unchanged, including `cut`, `deckOrder`). Steps: `[{t:'dealt', packets: dealtPackets(h.deckOrder, plan)}, {t:'upCardTurned', card: upCard}]`. One `next()` mint.

**applyPassRound1** (:630): `say = sayForBid(...)` (computed on pre-state), `bids' = bids+record`, `passes' = passes+1`.
* `passes' < 4`: hand `{passes', bids', turnSeat: nextSeat(seat)}`; steps `[bid]`.
* `passes' == 4` (the dealer turned it down): `suit = printed suit of upCard` (invariant: upCard non-null); hand `{phase:'bid_round_2', turnSeat: eldestOf(dealer), upCardTurnedDown:true, turnedDownSuit: suit, passes:0, bids'}`; steps `[bid, {t:'turnedDown', suit}]`. The up-card stays in `hand.upCard`/`kitty[0]`.

**applyPassRound2** (:668): same increment. `passes' < 4`: advance turn, `[bid]`. `passes' == 4` (only reachable when `stickTheDealer == false`; invariant `!stickTheDealer`): **throw-in**: `thrownIn = next(state,{hand:{...h, passes:4, bids:bids'}})`; `streak = min(cfg.misdealLimit, misdealStreak+1)`; `opened = startHand(rotateToNextHand(thrownIn, streak))`; result state `opened.state`, steps `[bid, {t:'throwIn'}, ...opened.steps]` (opened.steps is `[]`). No score change; `winnerTeam` unchanged; the next hand opens in `cutting` with the dealer moved left and a **new** shuffle for the new `handNo`. Three `next()` mints in total (thrownIn, rotate, startHand). <!-- critic: was "Four"; reduce.ts:687-689 has exactly three next() calls on the throw-in path -->
 The engine never speaks a "reshuffling" line for `misdealLimit` - the cap only bounds `misdealStreak` (e.g. five consecutive throw-ins with limit 3 give streaks `[1,2,3,3,3]`).

**setTrump(h, suit, makerSeat, alone)** (:702): `aloneSeat = alone ? makerSeat : null`, `sittingSeat = alone ? partnerOf(makerSeat) : null`; hand `{trump:suit, makerSeat, aloneSeat, sittingSeat}`; step `{t:'trumpSet', suit, makerSeat, aloneSeat, sittingSeat, viaOrderUp:false}`. `alone` can only be declared with the winning call, never later.

**applyOrderUp(state, seat, alone)** (:734) - round 1 acceptance. `upCard` non-null (invariant). Trump = **printed suit of upCard**. Maker = seat. `trumpSet.viaOrderUp = true`. Steps start `[bid, trumpSet]`.
* **Normal path** (`sittingSeat != dealerSeat`): hand `{phase:'dealer_discard', turnSeat: dealerSeat, bids', hands: hands with dealerSeat's hand = old dealer hand ++ [upCard]}` (appended at END, six cards), plus trump/maker/alone/sitting. Always routes to `dealer_discard` whoever called (including the dealer themself).
* **Edge: dealer is the sitting seat** (dealer's partner ordered up alone): the discard collapses to a no-op to avoid deadlock (a sitting seat has an empty legal set). Hand `{phase:'trick_play', turnSeat: openingLead(...), bids', dealerDiscard: upCard, trick: emptyTrick(0)}` plus trump/maker/alone/sitting; **dealer hand is NOT modified (stays 5)**; steps `[bid, trumpSet, {t:'dealerDiscarded', seat: dealerSeat}]`.

**applyCall(state, seat, suit, alone)** (:775) - round 2: hand `{trump:suit, makerSeat:seat, aloneSeat, sittingSeat, phase:'trick_play', turnSeat: openingLead(trumped hand), bids', trick: emptyTrick(0)}`; steps `[bid, trumpSet(viaOrderUp:false)]`. No pickup, no discard (`dealerDiscard` stays null).

**applyDiscard(state, seat, card)** (:805): hands[seat] with first occurrence of `card` removed (invariant: dealer ends with exactly 5); hand `{phase:'trick_play', turnSeat: openingLead(h), hands, dealerDiscard: card, trick: emptyTrick(0)}`; steps `[{t:'dealerDiscarded', seat}]` (no card).

**applyPlay(state, seat, card)** (:834): invariant trump non-null. `hands` without card; `plays = trick.plays ++ [{seat, card}]`; `ledSuit = ledSuitOf(plays, trump)` (effective suit of first play).
* Incomplete (`plays.len() < activeSeatCount(sitting)`): hand `{hands, trick:{...trick, ledSuit, plays}, turnSeat: nextActiveSeat(seat, sitting)}`; steps `[{t:'cardPlayed', seat, card}]`.
* Complete: `winnerSeat = trickWinner(plays, ledSuit, trump)` (invariant: winner != sittingSeat); `tricksWon[teamOf(winner)] += 1`; `trick' = {index: trick.index, ledSuit, plays, winnerSeat}`; hand `{phase:'trick_resolve', turnSeat:null, hands, trick: trick', trickLog: trickLog ++ [trick'], tricksWon}`. `sealsEuchre = tricksWon[1 - teamOf(makerSeat)] == 3` (strict `== 3`, after increment; makerSeat non-null invariant). Steps `[cardPlayed, {t:'trickWon', index, winnerSeat, plays, sealsEuchre}]`. The completed trick remains in `hand.trick` as well as `trickLog` (so UIs/invariants must dedupe by `index`).

**advance(state)** (:900) - total, engine-only:
* `status != active` -> `{state (same), steps: []}`.
* `lobby` -> `startHand(state)`. `deal` -> `runDeal(state)`. `trick_resolve` -> `advanceTrickResolve`. `hand_score` -> `advanceHandScore`.
* any other phase (cutting, bid_round_1, dealer_discard, bid_round_2, trick_play, game_over) -> identity `{state, steps: []}` (same object/equal).

**advanceTrickResolve** (:923): invariant `trick.winnerSeat != null`. If `trickLog.len() < 5`: hand `{phase:'trick_play', turnSeat: winnerSeat, trick: emptyTrick(trick.index+1)}`, steps `[]`. Else `enterHandScore`.

**enterHandScore** (:949): invariants maker non-null, `trickLog.len()==5`. `{result, delta} = scoreHand(makerSeat, aloneSeat, tricksWon, state.score, cfg.gameTo)`; `score' = applyDelta(state.score, delta)`; hand `{phase:'hand_score', turnSeat:null, result, delta}` (the fifth trick stays in `hand.trick`); state `{hand, score:score', winnerTeam: winnerTeam(score', gameTo)}`; steps `[{t:'handScored', result, delta, score:score', makerSeat, aloneSeat, tricksWon}]`. `status` is **not** changed here.

**advanceHandScore** (:984): if `state.winnerTeam != null`: hand `{phase:'game_over', turnSeat:null}`, `status:'complete'`, steps `[{t:'gameWon', team: winnerTeam, score: state.score}]`. Else `startHand(rotateToNextHand(state, 0))` (streak cleared; `handNo+1`; dealer moves left).

### 11.10 Phase table summary

| Phase | Actor | Moves | -> |
|---|---|---|---|
| lobby | engine | (advance) | cutting |
| cutting | cutterOf(dealer) | cut:yes/no | deal (atomic) -> bid_round_1 |
| deal | engine | (advance, defensive) | bid_round_1 |
| bid_round_1 | eldest, +1, +2, dealer | pass, orderUp(+alone) | next seat / dealer_discard / (dealer sits: trick_play) / 4th pass -> bid_round_2 |
| dealer_discard | dealer | discard:<6 cards> | trick_play |
| bid_round_2 | eldest.. dealer | pass, call:<suit != turnedDown>(+alone) | next seat / trick_play / 4th pass (stick off) -> throw-in -> cutting(next hand) |
| trick_play | turnSeat (skip sitting) | play:<card> | next active seat / trick_resolve |
| trick_resolve | engine | (advance) | trick_play (winner leads) / hand_score |
| hand_score | engine | (advance) | game_over / cutting (next hand) |
| game_over | none | none | terminal (`status=complete`) |

Mapping to actor timing (not engine concern): `trick_resolve`, `hand_score` and `deal` are the server-held "tempo gate" phases the actor releases by calling `advance`.

## 12. Invariants (`invariants.ts`) - port as `assert_invariants(&GameState) -> Result<(), InvariantError>` plus `assert_legal_non_empty`

`InvariantError { invariant: String /*"V4"*/, detail }`, Display `"{inv} violated: {detail}"`. Helper `allTricks(s)` = `trickLog`, plus `hand.trick` iff it has plays and no `trickLog` entry shares its `index` (dedupe, because `hand.trick` aliases the last logged trick during `trick_resolve`/`hand_score`). `dealt = phase not in {lobby, cutting}`.

| ID | Check |
|---|---|
| V4 | if `aloneSeat != null`: `aloneSeat == makerSeat`, `sittingSeat == partnerOf(aloneSeat)`, `trump != null`. Also `sittingSeat == null || aloneSeat != null` |
| V5 | if `dealerDiscard != null && phase != dealer_discard`: dealer hand len <= 5 unless dealer is the sitting seat |
| V8 | sitting seat appears in no trick's plays |
| V9 | for consecutive tricks (i>=1) with prev.winner and next plays non-empty: `next.plays[0].seat == prev.winnerSeat` and `prev.winnerSeat != sittingSeat` |
| V10 | (when dealt) `deckOrder.len()==24`, distinct; `kitty.len()==4`; universe of (all hands + played cards + kitty + dealerDiscard) has exactly 24 distinct cards; the "live" multiset (hands + played + dealerDiscard) has no duplicate (the up-card legitimately sits in both kitty and a hand after pickup, which is why kitty is excluded from this check); in `hand_score`/`game_over` with `result` not null and != throw_in, played count == 20 (no loner) or 15 (loner) |
| V11 | scores in `[0, gameTo]`; if `hand.delta != null`: both >= 0, at most one non-zero, each component in `{0,1,2,4}` **or** the clamped-loner exception (`d==3 && result==lone_march && score[team]==gameTo`) |
| V13 | (dealt) `3 + (upCardTurnedDown || dealerDiscard != null ? 1 : 0)` is 3 or 4 (tautological recompute) |
| V14 | (dealt and phase != deal) `upCard != null` and `upCard == kitty[0]` |
| V16 | with trump set: 7 cards have effective suit == trump; for each other suit: 5 if it is the colour mate (its jack is the left bower) else 6 |
| V17 | with trump set, for each trick with plays and ledSuit: the non-zero `cardValue`s in the trick are pairwise distinct |
| V18 | `trickLog<=5`, `bids<=8`, `passes<=4`, `trick.plays<=4` |
| V19 | `assertLegalNonEmpty(state, legalMovesFn)`: return OK if `turnSeat==null`, `status != active`, or turn seat is the sitting seat; else `legalMoves(state, turn).len() >= 1` else error `"seat {turn} must act in phase {phase} but has no legal moves"` |

Notes: V1,V2,V3,V6,V7 are enforced inside legal/reduce; V12 is a projection property; V20/V21/V22 are sequence properties tested in the suite.

## 13. Optimistic helpers (`optimistic.ts`) - client-side preview; port into `euchre-engine` (or protocol) for the Rune client

`OPTIMISTIC_FIELDS = ['hand','handCounts','trick','kittyCount','passes','trump','makerSeat','aloneSeat','sittingSeat','upCardTurnedDown','turnedDownSuit']`. Contract V22: for every legal move except `cut`, `applyOptimistic(project(s, seat), move)` equals `project(apply(s, seat, move).state, seat)` on every field above. **`legal` is not in the contract: every optimistic branch sets `legal: []`.** Never validates legality; `move` must come from `view.legal`. `cut` returns the view unchanged (identity).

`applyOptimistic(view, move) -> PublicGameView` where `you = view.you`:
* **play**: if `!view.hand.contains(card)` return view unchanged. Else `hand` minus first occurrence of card; `handCounts[you] = max(0, handCounts[you]-1)`; `trick = appendPlay(view.trick, you, card, view.trump, view.sittingSeat)`; `legal=[]`.
  `appendPlay`: `plays = trick.plays ++ [{seat,card}]`; `ledSuit = trick.plays.is_empty() ? effectiveSuit(card, trump) : trick.ledSuit`; `expected = sitting==null ? 4 : 3`; `complete = plays.len() >= expected && ledSuit != null && trump != null`; `winnerSeat = complete ? trickWinner(plays, ledSuit, trump) : trick.winnerSeat`; result `{index: trick.index, ledSuit, plays, winnerSeat}`.
* **discard**: if not held return view. `hand` minus the card, `handCounts[you]` decrement, `kittyCount = view.upCardTurnedDown ? view.kittyCount : 4`, `legal=[]`.
* **pass**: `passes = view.passes+1`. If `passes >= 4 && view.phase=='bid_round_1' && view.upCard != null`: `{passes:0, kittyCount:4, upCardTurnedDown:true, turnedDownSuit: printedSuit(upCard), legal:[]}`; else `{passes, legal:[]}`.
* **orderUp**: `trump = view.upCard ? printedSuit(upCard) : view.trump`; `sitting = alone ? partnerOf(you) : null`; `counts = handCounts copy`; `dealerSitsOut = sitting != null && sitting == view.dealerSeat`; if `!dealerSitsOut`: `counts[dealerSeat] += 1`; if `sitting != null`: `counts[sitting] = 0`; `hand = (you == dealerSeat && !dealerSitsOut && upCard) ? view.hand ++ [upCard] : view.hand`; result sets `hand, trump, makerSeat:you, aloneSeat: alone ? you : null, sittingSeat: sitting, handCounts: counts, kittyCount: dealerSitsOut ? 4 : view.kittyCount, legal:[]`.
* **call**: `sitting = alone ? partnerOf(you) : null`; `counts[sitting]=0` if sitting; sets `trump: move.suit, makerSeat:you, aloneSeat, sittingSeat, handCounts, legal:[]`.
* **cut**: identity.

## 14. Projection (`index.ts:247-321`) and public exports

`project(state, seat) -> PublicGameView` (security boundary):
```
isSittingOut = seat == hand.sittingSeat; count(s) = (s == sittingSeat) ? 0 : hands[s].len()
{ v: state.v, you: seat, status, phase: hand.phase, handNo: hand.handNo, dealerSeat, turnSeat, turnId: state.turnId, score: copy,
  trump, makerSeat, aloneSeat, sittingSeat, upCard, upCardTurnedDown, turnedDownSuit,
  hand: isSittingOut ? [] : hands[seat].clone(), handCounts: [count(0),count(1),count(2),count(3)],
  kittyCount: 3 + ((upCardTurnedDown || dealerDiscard != null) ? 1 : 0),
  trick: clone (plays cloned), trickLog: clones, tricksWon: copy, passes, bids: clone,
  legal: legalMoves(state, seat), result, delta: copy-or-null, winnerTeam: state.winnerTeam }
```
(`handNo` is the **hand** handNo, not `state.handNo`; they are equal except transiently.) Rules: never add fields capable of carrying foreign cards/buried kitty/dealer discard; `kittyCount` must not depend on **which** card was discarded (V13); `upCard` never nulled (V14); sitting partner sees empty hand and count 0 but their dead cards stay in `state.hand.hands`.

`projectSteps(steps, seat) -> Vec<Step>`: map each step; non-`dealt` steps pass through unchanged; `dealt`: each packet becomes `{seat, count, cards: (p.seat == seat && p.cards.is_some()) ? Some(cards.clone()) : None}`.

Public export surface of `index.ts` (port to `lib.rs` `pub use`):
* Types: ActionKind, AIDecisionSource, AutoReason, Bid, BidRecord, CardId, DealPacket, DealtPacket, EngineConfig, GamePhase, GameState, GameStatus, HandResult, HandState, LegalMove, LegalMoveId, Move, PendingDecision, Play, PlayerAction, PublicGameView, Rank, TableObservation, TablePlayer, RuleCode, Seat, SeatRole, Step, Suit, Team, Trick; `DEFAULT_ENGINE_CONFIG`; `ApplyOptions, ApplyResult, GameInit`; `DealResult`; `ScoreOutcome`; `OptimisticField`.
* cards: CARD_IDS, DECK, RANKS, SUITS, bowersOf, effectiveSuit, isLeftBower, plainRank, rankOf, sameColor, suitOf, trumpRank.
* seats: HUMAN_SEAT, SEATS, activeSeatCount, biddingOrder, cutterOf, eldestOf, firstActiveFrom, isActiveSeat, nextActiveSeat, nextSeat, opponentTeam, partnerOf, seatRole, teamOf.
* deal: assertDeckOrder, cutAt, cutForHand, dealFrom, dealHand, dealPlan, dealtPackets, fnv1a, handRng, hashSeed, mulberry32, newDeck, pickFirstDealer, shuffle, shuffleForHand, startsWithThree.
* legal: RANK_NAME, SUIT_NAME, SUIT_NOUN, cardName, cardNameLower, findLegalMove, getLegalPlays, isStuckDealer, legalMoveIds, legalMoves, orderLabel, whyIllegal.
* trick: cardValue, currentlyWinning, isTrickComplete, ledSuitOf, playsPerTrick, trickWinner.
* score: applyDelta, isEuchre, isGameOver, isInTheBarn, rawPoints, scoreHand, throwInOutcome, winnerTeam.
* reduce: RuleError, advance, apply, createGame, isRuleError, moveId, sayForBid, startHand; `newGame(seed, cfg?)` (index.ts).
* projection: project, projectSteps. optimistic: applyOptimistic, OPTIMISTIC_FIELDS. invariants: assertInvariants, assertLegalNonEmpty, InvariantError.
* Not exported from index (internal): `kittyOf`, `mintTurnId`, `deriveToken`, `next`, `withHandChanges`, `replaceHand`, `removeCard`, `emptyTrick`, `recordBid`, `bidStep`, `setTrump`, `openingLead`, `runDeal`, apply*/advance* helpers.

## 15. Golden vectors (from the real TS engine under bun) - use as Rust unit tests

```
fnv1a("")        = 2166136261          fnv1a("a") = 3826002220
fnv1a("foobar")  = 3214735720          fnv1a("seed#0") = 3952383447
fnv1a("héllo")   = 4058363231          fnv1a("😀") = 3409036472
(4294967295).toString(36) = "1z141z3"  (0).toString(36) = "0"
mulberry32(1)  first 5 = 0.6270739405881613, 0.002735721180215478, 0.5274470399599522, 0.9810509674716741, 0.9683778982143849
mulberry32(0)  first 3 = 0.26642920868471265, 0.0003297457005828619, 0.2232720274478197
hashSeed("abc",0) = 2666227160   hashSeed("abc",1) = 2683004779   hashSeed("seed-1",3) = 1258440548
fnv1a("abc#0#cut") = 4023222183  (=> cut index 4 + 4023222183 % 16 = 11)
shuffleForHand("abc",0) = KC KD AD 9H QD AH 9D TD TC QH JD AS TH TS JS QS 9S JC 9C KS JH QC KH AC
shuffleForHand("abc",1) = JS TH QD KD AS JH JD JC 9S QC 9C AC 9H KS KC 9D TD KH TC AH QH TS AD QS
cutForHand(shuffleForHand("abc",0),"abc",0) = AS TH TS JS QS 9S JC 9C KS JH QC KH AC KC KD AD 9H QD AH 9D TD TC QH JD
startsWithThree("abc", 0..3) = [false, true, false, true]
firstDealer: seed "abc" -> 1 ; seed "seed-1" -> 0     (pickFirstDealer(mulberry32(fnv1a(seed+"#firstDealer"))))
```
`createGame({gameId:"g-abc", seed:"abc"})`: `internalToken="26xgdh1wxuoi1my8zj1cynak"`, `turnId="13swn7p"`, `firstDealer=1`.
After `advance`: `phase=cutting, dealer=1, turnSeat=0 (cutter), turnId="dz47z7e93to6"`.
After `apply(seat 0, cut:false)`: `turnId="pkpgihoqqnfk"`, `upCard="JH"`, `kitty=[JH,QC,KH,AC]`, hands
`{0:[AH,9D,QS,9S,JC], 1:[TD,TC,QH,9C,KS], 2:[KC,KD,JD,AS,TH], 3:[AD,9H,QD,TS,JS]}`;
dealPlan (dealer 1, threeFirst=false): `[{2,2},{3,3},{0,2},{1,3},{2,3},{3,2},{0,3},{1,2}]`; steps: `cut{seat 0,cut false}`, `dealt{packets: (2:[KC,KD]) (3:[AD,9H,QD]) (0:[AH,9D]) (1:[TD,TC,QH]) (2:[JD,AS,TH]) (3:[TS,JS]) (0:[QS,9S,JC]) (1:[9C,KS])}`, `upCardTurned JH`.
After `apply(seat 0, cut:true)` (bump) instead: hands `{0:[9S,JC,AD,9H,QD], 1:[9C,KS,JH,AH,9D], 2:[AS,TH,QC,KH,AC], 3:[TS,JS,QS,KC,KD]}`, `upCard="TD"`.

**Full-match replays** (picker = `legal[floor(mulberry32(fnv1a(seed))() * len)]` drawn once per seat move; `advance` whenever `turnSeat==null`; `createGame({gameId:"g-"+seed, seed, cfg, firstDealer})`; invariants asserted every step). `moveHash = fnv1a(moves.join(","))` where each move is `"<seat><legalMoveId>"`:

| seed / cfg / firstDealer | seat moves | transitions (apply+advance) | moveHash | first 6 moves | final score | state.handNo | winner | final dealer | firstDealer | final turnId |
|---|---|---|---|---|---|---|---|---|---|---|
| `abc`, default, drawn | 213 | 280 | 3877649361 | 0cut:no,2orderUp,1discard:TD,2play:TH,3play:9H,0play:AH | [7,10] | 10 | 1 | 3 | 1 | 1mxxun81nrwnq5 |
| `full-10`, default, drawn | 244 | 317 | 4165239875 | 0cut:yes,2orderUp,1discard:TS,2play:JC,3play:9S,0play:KS | [9,10] | 11 | 1 | 0 | 1 | cejxtyc4kc4z |
| `x1`, `{gameTo:5,stickTheDealer:false}`, 2 | 86 | 111 | 2702995405 | 1cut:yes,3pass,0pass,1pass,2orderUp+alone,2discard:9C | [2,5] | 3 | 1 | 1 | 2 | 1iqy1gi1igyfrj |
| `nat`, `{requireNaturalTrump:true}`, 0 | 133 | 176 | 2991084946 | 3cut:no,1pass,2orderUp+alone,1play:KH,2play:9H,3play:JD | [10,3] | 6 | 0 | 2 | 0 | 1k11rk91j72yhc |

`handScored` sequence for `abc` (result:delta:score): `point:1/0:1-0, euchre:2/0:3-0, euchre:0/2:3-2, lone_point:0/1:3-3, euchre:0/2:3-5, euchre:0/2:3-7, point:1/0:4-7, euchre:2/0:6-7, point:1/0:7-7, lone_point:0/1:7-8, euchre:0/2:7-10`.
`full-10`: `euchre:0/2:0-2, lone_point:1/0:1-2, euchre:2/0:3-2, euchre:2/0:5-2, euchre:0/2:5-4, point:0/1:5-5, euchre:0/2:5-7, point:0/1:5-8, point:1/0:6-8, euchre:2/0:8-8, lone_point:1/0:9-8, euchre:0/2:9-10`.
`x1`: `euchre:0/2:0-2, point:0/1:0-3, euchre:2/0:2-3, euchre:0/2:2-5`. `nat`: `euchre:0/2:0-2, euchre:2/0:2-2, euchre:2/0:4-2, lone_point:0/1:4-3, euchre:2/0:6-3, euchre:2/0:8-3, euchre:2/0:10-3`.
Observation: final `state.handNo` equals number of scored hands minus 1 (no rotation after the winning hand) plus throw-ins. (Note: the TS picker uses the test helper's own mulberry32 = the exported one; the e2e/optimistic tests use a *different* signed-int `a|=0` mulberry variant for picking only; same algorithm result, not needed for the engine.)

## 16. Existing tests to port 1:1 (vitest -> `cargo test`). Paths: `src/lib/euchre/__tests__/`

Shared harness patterns to reimplement as test utilities: `runGame(initial, picker)` (loop: `turnSeat==None` -> `advance`, else `legalMoves` non-empty (V19) -> `apply(picker(...))`; panic if state unchanged / budget 20000 exhausted), `runUntil(pred)`, `randomPicker(seed)` (as in section 15), `passHeavyPicker` (passes with `rnd()<0.8` while bidding using stream `mulberry32(fnv1a(seed+"#pass-heavy"))`, else random), `alonePicker` (prefers any id ending `+alone`), `aloneAtPicker(seed, skip)`, `opened(seed, firstDealer?, cfg?)` = `advance(createGame({gameId:"g-"+seed, seed, cfg, firstDealer}))`, `afterCut`, `passes(n)`, `patchHand`, deep-freeze purity (in Rust: assert inputs unchanged via clone-and-compare, since `&` borrows already enforce non-mutation), `expectRule(code, fn)` (asserts `RuleError.code` and `.legal`).

### cards.test.ts (399 lines)
`deck constants`: SUITS is S H D C; RANKS 9 T J Q K A; DECK 24 in canonical order (spelled out); 24 distinct, no 2-8/joker; CARD_IDS same array as DECK; frozen.
`rankOf/suitOf`: split all 24; suitOf reports printed suit even for left bower; round-trip rank+suit==card.
`sameColor`: full 4x4 truth table and reflexive; symmetric.
`effectiveSuit`: left bower reports trump (JD/H, also JC/S etc); right bower identity; other-colour jack untouched; non-jacks never promoted; identity when trump null for all 24; differs from suitOf for exactly one card per trump; matches hand-written bower table over all 96 (card,trump) pairs; only returns printed suit or trump; `JD 9C TC KC AC` void in diamonds with hearts trump.
`trump census (V16)`: 7/5/6/6 distribution; the seven trump named exactly for every trump.
`trumpRank`: spec sample values (JH/H=8, JD/H=7, AH/H=6, 9H/H=2, AS/H=0); orders right>left>A>K>Q>T>9 for every trump; exactly 7 non-zero per trump; `>0` iff effectiveSuit==trump over 96 pairs; 0 for every off-suit card incl. wrong-colour jack; right bower single highest (8), left second (7).
`plainRank`: jack below queen (JS=3, QS=4); full A6 K5 Q4 J3 T2 9 1 table; depends on rank alone across suits; strictly ascending over RANKS.
`bowersOf`: [right,left] for every trump; agrees with trumpRank [8,7].
`isLeftBower`: true for exactly one card per trump over 96 pairs; false for right bower; false for every card when trump null.

### seats.test.ts (360)
`seat constants`: HUMAN_SEAT==0; SEATS 0 1 2 3 and frozen. `nextSeat`: 0->1->2->3->0; four steps return to start from every seat. `partnerOf`: 0<->2, 1<->3; involution, never self; partners same team. `teamOf/opponentTeam`: {0,2}->0, {1,3}->1; opponentTeam flips, involution; adjacent seats opponents. `seatRole`: 0 dealer 1 eldest 2 partner 3 third for every dealer; seat 1 eldest when seat 0 deals; bijection per dealer. `eldestOf/cutterOf`: eldest left of dealer, cutter right; both opponents of dealer. `biddingOrder`: eldest -> partner -> third -> dealer for every dealer; dealer last/eldest first; fresh array each call. `firstActiveFrom` (loner trick-1 lead; dealer=0): loner eldest (alone=1,sitting=3)->lead 1; dealer's partner (alone=2,sitting=0)->1; third seat (alone=3,sitting=1)->**2 (a defender)**; dealer (alone=0,sitting=2)->1; never returns sitting seat for every (from,sitting); identity if active; skips exactly one clockwise when sitting; holds for every dealer. `nextActiveSeat`: plain nextSeat when nobody sits; never returns sitting (V8) for every pair; skips exactly one; never skips if next active; total even from the sitting seat; cycles exactly 3 active seats under a loner and returns to lead; cycles all 4 without loner. `activeSeatCount`: 4 / 3; equals number of seats isActiveSeat admits; implies 20/15 cards per hand. `isActiveSeat`: false only for sitting seat.

### trick.test.ts (430)
`cardValue trump band 102..108`: seven hearts trump score 108..102 right-bower first; `100+trumpRank`; strictly inside 102..108; same whether or not trump is led. `led band 51..56`: six spades 56..51 when spades led/hearts trump; `50+plainRank`; whole trump band outranks whole led band. `zero band`: `9C` beats `AH` (clubs led, diamonds trump); 0 for every card neither trump nor led; off-suit ace is 0; left bower scores 0 in its printed suit once that suit isn't trump. `trump null`: led band only; never above 56 over 96 pairs; would-be left bower ordinary.
`trickWinner`: spec cases 20-26 (nine of led beats off-suit ace [trump D, led C, 9C AH KH QH -> 9C]; any trump beats plain [trump H, led S: AS 9H KS QS -> 9H]; left bower outranks nine of trump [AS 9H JD KS -> JD]; right beats left [AS JD JH KS -> JH]; leading left bower leads trump and wins [trump H led by JD: JD AH KH 9H -> JD, ledSuit H]; spades trump JD ordinary diamond jack below queen [9D TD JD AD led D -> AD]; 3-play loner [trump C led H: KH(1) AH(3) 9C(0) -> seat 0]); single-play trick won by that play; throws plain Error on empty; no mutation; always the max cardValue; winner always above zero band; independent of order for trump/led bands; resolves a zero-value tie deterministically and never to a discard; total when all plays score 0 (first play holds).
`ledSuitOf`: null for empty; effective suit (left bower lead leads trump); printed suit for wrong-colour jack; printed when trump null; fixed by first play; agrees with effectiveSuit over 96 pairs.
`playsPerTrick/isTrickComplete`: 4 / 3; incomplete until last active seat played; completes at three under loner.
`currentlyWinning`: null on empty; leader after one card; tracks takeover card by card; never disagrees with trickWinner on any prefix; derives led suit itself (left-bower lead).

### score.test.ts (386)
Table-driven over 12 rows `(made, alone) -> (result, points, to)`: (5,F)march 2 makers; (4,F)point 1; (3,F)point 1; (2,F)(1,F)(0,F) euchre 2 defenders; (5,T) lone_march 4; (4,T)(3,T) lone_point 1; (2,T)(1,T)(0,T) euchre 2 defenders - from every maker seat. Then: lone euchred concedes exactly 2; going alone changes label at 3-4 not points; alone doubles march 2->4; no super-euchre (defenders all 5 still 2); reads `made` off makers' index not index 0; identical for a seat and its partner; `aloneSeat` value irrelevant (only null vs non-null). **Clamp (V11)**: case 34 march at 9 wins at 10 (score [9,4], makers team 0, made 5 -> delta [1,0] -> [10,4]); case 35 lone march from 7 -> delta [3,0] -> [10,2]; case 36 euchre credits defenders (makerSeat 0, made 1, score [5,5] -> delta [0,2]); euchre at 9 clamps (defenders win at 10); team already at gameTo gains 0; clamps against non-default gameTo; exhaustive V11 (delta non-negative, single component, equals clamped raw); never mutates inputs. `throwInOutcome`: case 37 nobody scores (`throw_in`, [0,0]); score untouched when applied; fresh outcome each call. `applyDelta`: componentwise add; fresh tuple/no mutation; composes with scoreHand to reproduce post-clamp score. `winnerTeam/isGameOver`: null while short; team 0 the instant it reaches gameTo; team 1 likewise; non-default gameTo; agrees with isGameOver at every score. `isInTheBarn`: true at exactly gameTo-1; false elsewhere; per-team; tracks non-default gameTo. `isEuchre`: true only for "euchre". `rawPoints`: matches table for six results; equals unclamped delta from a fresh score.

### legal.test.ts (796)
`getLegalPlays`: case 8 (`JD 9C TC KC AC`, hearts trump, led D) all five legal; case 9 (led H) only `JD`; case 10 on lead (null) all five; led C -> the four clubs; case 11 (`AS KS 9H TD QC`, spades trump, led S) -> AS KS; case 12 same hand clubs trump -> one legal card per lead (led C -> QC, led H -> 9H, led D -> TD); case 13 (spades trump, led S, hand `JC 9D`) -> JC only; case 14 (spades trump, led C, hand `JC AH`) -> both; measured by effectiveSuit never suitOf; no obligation to trump/overtrump; preserves hand order; fresh array/no mutation; works before trump exists (printed suits); returns [] only for empty hand; case 19 property: never empty, always a subset, never admits a renege.
`whyIllegal`: spec example verbatim; null for legal play; not-in-hand message; comma/"and" joined list; singular form for a single held card; prefers bower explanation when left bower held; agrees with getLegalPlays on every card over every lead/trump.
`naming`: SUIT_NAME/SUIT_NOUN/RANK_NAME exact+frozen; cardName names printed card; 24 distinct names; orderLabel dealer "I take it", partner "I assist", else "Order it up".
`isStuckDealer (V3)`: true only dealer/bid_round_2/passes==3/flag on; false for non-dealer; false below 3 passes; false in round 1; false with flag off.
`legalMoves guards`: empty for not-to-act seats; empty unless status active; case 18 empty for sitting seat in EVERY phase even on its turn; [] in every engine-only phase.
`cutting`: exactly Bump / Run 'em. `bid_round_1`: pass/orderUp/orderUp+alone with role-correct label; dealer "I take it"; same three ids for all four seats. `dealer_discard`: all six cards incl. up-card. `bid_round_2`: case 15 (turnedDownSuit H) 7 entries and no `call:H`; labels "Spades"/"Spades, alone"; excludes turned-down suit whichever; case 16 stuck dealer 6 entries, NO pass; case 17 stick off -> 7 entries with pass; non-dealer at passes==3 keeps pass; all four suits when nothing turned down.
`trick_play`: wraps getLegalPlays with `play:` ids; whole hand on lead; whole hand when void (left bower doesn't follow printed suit).
`V19`: never empty for the acting seat in every phase across all four seats. `V21 purity`: deep-frozen state not mutated in any actionable phase; deep-frozen hand not mutated; fresh arrays/objects each call. `legalMoveIds/findLegalMove`: ids == legalMoves.map(id) same order; findLegalMove returns offered move or None; finds every reported id and nothing else.

### house-rules.test.ts (93) - natural trump
Fixture: `createGame({gameId:'rules', seed:'rules', firstDealer:0, cfg:{requireNaturalTrump:true}})`, advance, cut:no, advance; then patch `upCard:'9H'` and `hands[1] = [JD,9C,TC,QC,AC]` (left bower of hearts but no natural heart).
* left bower alone cannot order up: `legalMoves(s,1)` ids == `["pass"]`.
* left bower alone cannot name that suit in round two (phase bid_round_2, turnedDownSuit 'S'): `call:H` absent, `call:C` present.
* dealer may count the natural up-card: dealer (seat 0 on turn) legal includes `orderUp`.
* stuck dealer chooses (phase bid_round_2, turnSeat 0, passes 3, turnedDown 'S'): ids == `[call:H, call:H+alone, call:D, call:D+alone, call:C, call:C+alone]`.
* matches finish with natural trump for stick in {true,false}: 20 seeds `house-{n}`, up to 3000 steps, policy "prefer first legal call/orderUp else `moves[(seed+step) % len]`", status `complete`.

### reduce.test.ts (1198)
`createGame`: lobby, schema 1, status active, scores 0, handNo 0, misdealStreak 0, no winner, null upCard/trump/maker/dealerDiscard, empty trick `{index:0,ledSuit:null,plays:[],winnerSeat:null}`, all hands empty, legalMoves empty for all seats; deckOrder is the 24-card permutation == `shuffleForHand(seed,0)` and `kitty == deckOrder[20..]`; honours explicit firstDealer (all four) else drawn from seed (hand.dealerSeat == firstDealer); cfg overrides (`gameTo:5, stickTheDealer:false` keep `deckVariant:'24'`, `misdealLimit:3`).
`lobby -> cutting`: advance opens first hand with **no steps**, phase cutting, turnSeat == cutterOf(dealer), deckOrder == shuffleForHand, turnId changed, legal ids `[cut:yes, cut:no]`; other three seats get []; `startHand` is a pure re-open (same handNo, dealer, equal hands).
`cutting -> deal -> bid_round_1`: run 'em deals from untouched permutation (cut=false, deckOrder unchanged, bid_round_1, turnSeat eldest, passes 0, bids [], upCard == deckOrder[20], kitty == deckOrder[20..], five cards each, 24 distinct); bump rotates by `cutForHand`, differs from original, same multiset, upCard == deckOrder'[20]; steps order `[cut, dealt, upCardTurned]` with `cut == {t:'cut', seat: cutterOf(1), cut:false}`, 8 packets summing 20, 2 packets/seat summing 5, packet cards concatenated == hand, packet seats `[2,3,0,1,2,3,0,1]` for dealer 1, `upCardTurned.card == hand.upCard`; non-cutter seats -> `not_your_turn`, `pass`/`orderUp`/`play` by cutter -> `wrong_phase`.
`bid_round_1`: eldest ids `[pass, orderUp, orderUp+alone]` role-correct labels; passes rotate one seat left and count; four passes turn the up-card down and open round 2 at eldest; every orderUp routes to dealer_discard whoever called; role-correct spoken lines; refuses round-2 vocabulary (call -> wrong_phase) (V2).
`dealer_discard`: offers six incl. up-card; discarding the up-card leaves five and original deal intact; discarding a dealt card keeps up-card in dealer hand; rejects discard of unheld card (`card_not_in_hand`); rejects bidding after trump set with `bidding_closed` not `wrong_phase`; rejects discard from non-dealer.
`bid_round_2`: pass + 3 suits x {plain, alone}, never turned-down suit; naming turned-down suit -> `illegal_move`; refuses round-1 vocabulary (orderUp -> `wrong_phase`) (V2); successful call skips discard and leads straight to trick_play; says `"Next."` for colour mate and `"Crossing the creek."` otherwise; auction bounded at eight records (V18).
`stick the dealer` (stuck = dealer 0, round 1 four passes + three round-2 passes = `passes(afterCut(opened(seed,0)),7)`): pass removed from dealer set (6 ids all `call:`), `isStuckDealer(s,0)` true/(s,1) false; dealer pass -> `dealer_must_call` (never `bidding_closed`), `err.legal` ids == legalMoveIds(s,0), state unchanged; forced call accepted, stuck line matches `/, I guess\.$/` even for colour mate, phase trick_play, trump == suit, maker 0, turnSeat eldest(0), `bids[7].say` is the stuck line; stick off: dealer may pass (7 ids incl pass) and the hand is thrown in: steps `[bid, throwIn]`, score [0,0], handNo 1 (both), dealer nextSeat(0), misdealStreak 1, phase cutting, turnSeat cutterOf(dealer'), deckOrder == shuffleForHand(seed,1), bids [], trump/upCard null, winnerTeam null, status active; misdealLimit cap: stick off, limit 3, five consecutive throw-ins -> streaks `[1,2,3,3,3]`, dealers `[1,2,3,0,1]`, handNo 5, score [0,0].
`trick_play` (fixture `bowerFixture`: hearts trump, seat 1 led AH, seat 2 holds `JD 9C TC KC AC`, others `0:[9S TS QS KS AS] 1:[9H TH QH KH] 3:[9D TD QD KD AD]`, turnSeat 2): left bower is the only legal follow to a trump lead; renege -> `must_follow_suit` with the real legal set; card not held rejected before follow-suit check (`card_not_in_hand`); bids -> `bidding_closed`, other kinds `wrong_phase`; trick resolves to left bower and parks in `trick_resolve`; `trickWon.sealsEuchre` only when defenders reach three; `advance` releases the hold and gives the lead to the trick winner.
`advance`: identity in every phase with a seat to act; identity once match complete; scores the hand after the fifth trick then opens the next; `handScored` with the table row and `gameWon` when score reached; re-deals defensively from a bare `deal` phase.
`acting seat validated`: rejects every seat != turnSeat in every phase with a turn; stale turn nonce rejected without touching state (`stale_turn`); rejects everything once complete (`wrong_phase`); state byte-identical after a battery of rejections.
`moveId`: round-trips every offered move over twenty matches (`moveId(m.move) == m.id`); total over the Move union.
`determinism (V20)`: same seed -> identical deal; replays an identical full game from same seed and move sequence; turn nonces from a hash chain so states reproducible. `purity (V21)`: never mutates deep-frozen state across a whole match; legalMoves/project hand back fresh arrays.

### game.test.ts (806)
`going alone` for 4 cases (dealer 0; cutter 3, eldest 1; seed `alone-{maker}` / `alone-play-{maker}` / `alone-reject-{maker}` via `orderUpAfter(seed, 0, skip, true)` = cut:no then `skip` passes then orderUp alone):
| case | skip | maker | sitting | lead | dealer sits out |
|---|---|---|---|---|---|
| loner is eldest | 0 | 1 | 3 | 1 | no |
| loner is dealer's partner | 1 | 2 | 0 | 1 | **yes** (phase goes straight to trick_play, `dealerDiscard == upCard`, dealer hand stays 5) |
| loner is third seat | 2 | 3 | 1 | **2 (a defender)** | no |
| loner is the dealer | 3 | 0 | 2 | 1 | no |
Per case: (a) maker/aloneSeat/sittingSeat/lead correct, `lead == firstActiveFrom(eldestOf(0), sitting)`; non-sitting-dealer path goes to dealer_discard (turnSeat 0, six cards), discard hands[0][0] -> trick_play, turnSeat==lead, sitting hand still 5; (b) three plays a trick, fifteen distinct cards, five tricks, no sitting seat in any trick/winner, sitting partner sees nothing (legalMoves [] in every state, not on turn, projected hand [] and count 0), all active hands empty at hand end while the dead hand stays 5, leaders after trick 1 == previous winner (V9), result in `{lone_point, lone_march, euchre}` with the correct raw delta; (c) every action from sitting seat (pass, orderUp, call, cut, discard, play) -> `sitting_out` and state unchanged. Also: never lets the loner take the lead from sitting partner (seed `folk-rule`, skip 2: maker 3, sitting 1, lead 2); an ordinary call keeps all four seats, four plays a trick, 20 distinct cards, result in `{point,march,euchre}`.
`a full match`: ends with exactly one team on target; `gameWon` emitted exactly once as the last step; rotates the deal one seat left after every scored hand; clamps the winning score (V11); never awards more than the table row; reaches target score exactly for gameTo in {5,7,10,11}; every hand scores under stick-the-dealer (no throw-ins).
`fuzz` (headless, invariants asserted on every state, plus per-hand checks that `scoreHand` agrees with `handScored`): batches `default` 200 games (maxHands 25), `to-5` 40 (15), `to-11` 30 (30), `no-stick` 40 (60), `pass-heavy-stick` 40 (30), `pass-heavy-throwin` 40 (stick off, 40, expects throw-ins > 0). Seeds `fuzz-{label}-{i}`, `firstDealer = i % 4`. Asserts: complete, phase game_over, winner score == gameTo, other < gameTo, >=1 hand, hands <= maxHands, tricks == hands*5, tricks <= (maxHands+throwIns)*5, each hand 20 or 15 cards played, each score move totals 1-4 with a single team, deltas count == hands, loners > 0, >1 distinct delta, results subset of the five rows (200-game batch must hit all five: euchre, lone_march, lone_point, march, point), throw-ins 0 unless batch expects some. Plus: never deals a card twice in 200 opening deals (`deal-fuzz-{i}`, dealer i%4, cut `i%2==0`): 24 distinct cards, `upCard == kitty[0]`, 5 per seat.

### project.test.ts (733) - security boundary (belongs with `project`/`project_steps`)
`card-id scanner` (regex `[9TJQKA][SHDC]` finds every card id wherever embedded; no false positives on any engine-generated string - labels/says are Title case, turnId is lowercase base36); `project shape` (reports seat, mirrors public state, legal set only for seat on turn and never empty (V19), no mutation of deep-frozen state); `up-card persistence (V14)` (stays identified from the turn to the hand end in both branches; survives dealer discarding the up-card); `kittyCount (V13)` (3 before the up-card leaves the table, 4 after, both branches; two states differing only in the discard project identically); `sitting partner (V8)` (blanked from the moment the call lands but kept in state; hidden from owner for whole hand); `redaction (V12)` (negative control: detector fires on every category; sweeps a full match every state/every seat/steps; loner matches (alonePicker); all four sitting seats and first dealers (aloneAtPicker); stick-off throw-ins) - a card is entitled iff in own hand (unless sitting), the up-card, or played face up; `projectSteps` (blanks every deal packet not addressed to receiver keeping `count`; passes non-dealt steps through; no card field on `dealerDiscarded`; never mutates steps).

### optimistic.test.ts (151) - V22
60 random legal games (seeds `v22-{g}`, picker = signed-int mulberry variant seeded `1 + g*7919`), at every decision check EVERY legal move except `cut`: `applyOptimistic(project(s,seat), m)` vs `project(apply(s,seat,m).state, seat)` field-by-field (JSON-equal) over `OPTIMISTIC_FIELDS`; must run > 5000 checks; identity for a card the seat does not hold; never mutates the view.

### scripts/e2e-play.ts (569 lines) - independent harness (port as an integration test / `examples/e2e.rs`)
Drives the public API like the actor: `createGame`, loop `turnSeat==null ? advance : apply`, re-derives invariants from first principles, fuzzes N games (default 300). Checks per step: `project(s, seat)` JSON never contains another seat's cards, `kitty[1..3]`, or the dealer's discard (the up-card is public even in the dealer's hand); exactly one seat has a legal move and it equals `turnSeat`; `advance` makes progress in engine-only phases; `apply` returns a new state; `assertLegalNonEmpty` after each step; after `game_over` every seat has `legalMoves == []`; an off-turn seat has none; a bogus `play` is rejected (RuleError); counts hands, euchres (sampled from `hand.result` during `hand_score`), and loner calls; prints a summary. Uses `effectiveSuit`, `trumpRank`, `plainRank`, `DECK`, `assertInvariants`, `InvariantError`.

## 17. Divergences between docs/02 and the code (code wins)

* Doc `deal.ts` snippet lacks `assertDeckOrder` calls and the 20-card sum check; code has both.
* Doc only documents `cutAt` (stream cut); the reducer actually uses **`cutForHand`** (hash-derived, restart-safe), not `cutAt`.
* Doc names `packages/euchre-core/src/*`; real path is `src/lib/euchre/*`; doc `rng.ts`/`deck.ts` files do not exist separately.
* Doc never mentions `requireNaturalTrump`; code and `house-rules.test.ts` implement it (printed-suit check; dealer in round 1 exempt; stuck dealer exempt in round 2).
* Doc 7 says `dealer_discard` -> `turnSeat = firstActiveFrom(eldest, sittingSeat)`; code uses `openingLead` = same expression. Doc says human cuts only when dealer==1 (actor concern, not engine).
* Doc 9 case 37 says throw-in test is `result === "throw_in", delta [0,0]` - in the reducer the throw-in is **not** recorded in `hand.result`/`delta` (the whole `HandState` is replaced by `startHand`); only the `{t:'throwIn'}` step is emitted.
* Doc V11 permits delta `{1,2,4}`; code additionally allows a clamped lone march delta of 3.
* No `defendAlone`, `superEuchre`, `farmersHand`, `railroading`, `dealerMustHoldTrump` code paths exist (config literals only).

## 18. Things that touch time/randomness/IO (explicit list)

* Randomness: only `mulberry32`/`fnv1a`-derived values: per-hand shuffle (`seed#handNo`), per-hand cut index (`seed#handNo#cut`), deal alternation (`hashSeed & 1`), first dealer (`seed#firstDealer`), default `internalToken` (`seed#gameId#token#i`), turn nonces (hash chain of token + previous turnId). No `Math.random`.
* Time: none in the engine. Actor-owned: `turnDeadlineAt`, `PendingDecision.requestedAt/revealAt`, timers (`timerId`, `watchdogId`, `revealId`). Engine never reads them.
* IO: none. The engine has zero dependencies. (`index.ts` header: "Nothing in here touches the network, the clock, or a random number generator".)
* Production should pass a CSPRNG `internalToken` into `GameInit` and a real UUID/ULID-like `gameId` (TS `newGame` uses `g-{seed}` for tests).
