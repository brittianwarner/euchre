# 02 — Game Rules & Engine

The complete euchre rulebook as implemented by `packages/euchre-core` (`@euchre/core`), plus the pure reducer API, the phase state machine, and the test plan. This document is normative: if the code and this document disagree, the code is wrong.

`@euchre/core` has **zero runtime dependencies** and is imported unchanged by four consumers — the `euchreTable` actor (authoritative), the three `aiSeat` actors (advisory candidate filtering), the browser (legality preview, optimistic play, coach hints), and the replay renderer. See `01-ARCHITECTURE.md` for who calls what, `03-AI-AGENTS.md` for how the LLM chooses among these moves, `04-FRONTEND-UX.md` for how `LegalMove[]` drives the UI, and `05-IMPLEMENTATION-PLAN.md` § M1 for the milestone that closes this package.

Types referenced throughout are defined verbatim in `00-OVERVIEW.md` § Shared Types and live in `packages/euchre-core/src/types.ts`. All code below compiles under `tsc --strict --target es2022 --moduleResolution bundler` with `typescript@5.9.3`.

---

## 1. The physical game

### 1.1 Deck

24 cards: `9 T J Q K A` in each of `♠ ♥ ♦ ♣`. No joker. `CardId` is `` `${Rank}${Suit}` `` — `JS`, `TC`, `9D`, `AH`. `cfg.deckVariant` is the literal type `"24"`; the 25-card *benny* variant is out of scope and has no code path.

Colour pairs are load-bearing, not cosmetic: `♠↔♣` (black), `♥↔♦` (red). `sameColor(a, b)` and the private `COLOR_MATE` map in `cards.ts` are the only place this relationship is encoded.

### 1.2 Seats and teams

Four seats, clockwise, fixed for the life of the match:

| Seat | Chair | Occupant |
|---|---|---|
| `0` | South | the human (`HUMAN_SEAT`) |
| `1` | West | AI |
| `2` | North | AI, the human's partner |
| `3` | East | AI |

Teams are `teamOf(s) = s & 1`: team `0` is `{0, 2}` (the human and North), team `1` is `{1, 3}`. `partnerOf(s) = (s + 2) & 3`. Partners sit opposite. `score` is indexed by `Team`, so `score[0]` is always the human's team.

Seat *chairs* are fixed; seat *roles* rotate with the dealer. `seatRole(s, dealer)` returns `0` dealer, `1` eldest (left of dealer, bids and leads first), `2` dealer's partner, `3` third seat (right of dealer, the cutter).

### 1.3 Dealer rotation and the cut

`firstDealer` is chosen once at game creation from the seeded RNG (`Math.floor(rnd() * 4)`) and stored immutably. After every scored hand and every throw-in the deal moves one seat **left** (clockwise): `dealerSeat = nextSeat(dealerSeat)`. There is no draw-for-deal ceremony in v1.

The seat to the dealer's **right** — `(dealerSeat + 3) & 3`, i.e. third seat — is offered the cut. It may take it (`cut:yes`, "Bump.") or decline (`cut:no`, "Run 'em."). Declining is a recorded move, not a pass; either advances the phase. The shuffle has **already happened and been persisted** before this phase opens (§ 6.1): `cut:yes` deterministically rotates the persisted order at a seeded index, `cut:no` leaves it untouched, and neither reshuffles. A crash during `cutting` cannot change anyone's hand.

### 1.4 The deal

Five cards to each seat, dealt clockwise starting with eldest, in **two passes** using the canonical 3-then-2 / 2-then-3 alternation: on the first pass consecutive players receive packets of 3, 2, 3, 2 (or 2, 3, 2, 3); on the second pass each receives the complement, so everyone ends with 5. Which alternation is used is derived deterministically from `hashSeed(seed, handNo) & 1` — no extra state, fully replayable.

The four undealt cards are the **kitty**, `deckOrder[20..23]`. `kitty[0]` is turned face up as the **up-card**. `kitty[1..3]` are buried and are never revealed to any client, ever, including in post-game replay (invariant **V12**).

```ts
// packages/euchre-core/src/deck.ts
import type { CardId, Seat } from "./types";
import { nextSeat } from "./seats";

export interface DealPacket { readonly seat: Seat; readonly count: number; }

/** Canonical 3-2 / 2-3 alternation. Sums to 5 per seat, 20 total. */
export function dealPlan(dealerSeat: Seat, startsWithThree: boolean): DealPacket[] {
  const order: Seat[] = [];
  let s = nextSeat(dealerSeat);
  for (let i = 0; i < 4; i++) { order.push(s); s = nextSeat(s); }
  const first = order.map((seat, i) => ({ seat, count: (i % 2 === 0) === startsWithThree ? 3 : 2 }));
  const second = first.map((p) => ({ seat: p.seat, count: 5 - p.count }));
  return [...first, ...second];
}

export function dealFrom(deckOrder: readonly CardId[], plan: readonly DealPacket[]): {
  hands: Record<Seat, CardId[]>;
  kitty: [CardId, CardId, CardId, CardId];
} {
  const hands: Record<Seat, CardId[]> = { 0: [], 1: [], 2: [], 3: [] };
  let i = 0;
  for (const p of plan) for (let k = 0; k < p.count; k++) hands[p.seat].push(deckOrder[i++]!);
  const kitty: [CardId, CardId, CardId, CardId] = [
    deckOrder[20]!, deckOrder[21]!, deckOrder[22]!, deckOrder[23]!,
  ];
  return { hands, kitty };
}
```

The plan is emitted to the client as a `{ t: "dealt", packets }` `Step` so the deal animates faithfully; `projectSteps()` carries only counts, never card identity, so the choreography channel cannot leak the deal.

---

## 2. Card ranking — bowers and effective suit

### 2.1 The rule

Once a trump suit exists, the ranking is:

**Trump, high → low:** Right Bower (J of trump) · Left Bower (J of the *same colour*, other suit) · A · K · Q · T · 9 — **seven cards**.

**Plain suits, high → low:** A · K · Q · J · T · 9 — **six cards**, except the suit that is the same colour as trump, which has **five**, because its jack has defected to trump.

The left bower **is** a trump card for every purpose: it is led as trump, it must be followed as trump, and its printed suit is irrelevant. With hearts trump, a hand of `JD 9C TC KC AC` is **void in diamonds**; leading `JD` leads *hearts*.

### 2.2 `effectiveSuit()` — the load-bearing function

Nothing outside `cards.ts` may read `suitOf()` for a suit comparison. Voidness, follow-suit legality, led-suit determination and the trick comparator all route through `effectiveSuit`. This is mechanically enforced by the `no-suit-compare` ESLint rule (see `05-IMPLEMENTATION-PLAN.md`, M0).

```ts
// packages/euchre-core/src/cards.ts
export type Suit = "S" | "H" | "D" | "C";
export type Rank = "9" | "T" | "J" | "Q" | "K" | "A";
export type CardId = `${Rank}${Suit}`;

export const rankOf = (c: CardId): Rank => c[0] as Rank;
export const suitOf = (c: CardId): Suit => c[1] as Suit;

const COLOR_MATE: Readonly<Record<Suit, Suit>> = { S: "C", C: "S", H: "D", D: "H" };
export const sameColor = (a: Suit, b: Suit): boolean => a === b || COLOR_MATE[a] === b;

/** The suit a card BEHAVES as. Identity when trump is null or the card is not the left bower. */
export function effectiveSuit(card: CardId, trump: Suit | null): Suit {
  const s = suitOf(card);
  if (trump !== null && rankOf(card) === "J" && s === COLOR_MATE[trump]) return trump;
  return s;
}

/** 0 = not trump. Right 8, left 7, A 6, K 5, Q 4, T 3, 9 2. */
export function trumpRank(card: CardId, trump: Suit): number {
  if (effectiveSuit(card, trump) !== trump) return 0;
  const r = rankOf(card);
  if (r === "J") return suitOf(card) === trump ? 8 : 7;
  return { A: 6, K: 5, Q: 4, T: 3, "9": 2, J: 0 }[r];
}

/** Plain-suit order A K Q J T 9. Meaningful only when the card is not trump. */
export function plainRank(card: CardId): number {
  return { A: 6, K: 5, Q: 4, J: 3, T: 2, "9": 1 }[rankOf(card)];
}
```

### 2.3 `cardValue()` — the total order inside one trick

`cardValue` collapses trump rank, led-suit rank and "cannot win" into a single comparable integer. It is the *only* comparator used by trick resolution and by the heuristic ranker.

```ts
// packages/euchre-core/src/trick.ts
import { effectiveSuit, plainRank, trumpRank, type CardId, type Suit } from "./cards";
import type { Play, Seat } from "./types";

/**
 * Strength of `card` in a trick whose led suit is `led` under `trump`.
 * Bands: 102..108 trump · 51..56 led suit · 0 cannot win.
 * Comparable only within one trick. Ties are impossible: 24 distinct cards.
 */
export function cardValue(card: CardId, led: Suit, trump: Suit | null): number {
  const eff = effectiveSuit(card, trump);
  if (trump !== null && eff === trump) return 100 + trumpRank(card, trump);
  if (eff === led) return 50 + plainRank(card);
  return 0;
}

export function trickWinner(plays: readonly Play[], led: Suit, trump: Suit): Seat {
  let best = plays[0]!;
  let bestValue = cardValue(best.card, led, trump);
  for (let i = 1; i < plays.length; i++) {
    const p = plays[i]!;
    const v = cardValue(p.card, led, trump);
    if (v > bestValue) { bestValue = v; best = p; }
  }
  return best.seat;
}
```

Note the consequence encoded in the `0` band: a card of neither the led suit nor trump can never win, so `9C` beats `AH` when clubs are led and diamonds are trump.

> **Spec note:** the shared-types block declares `trumpRank`/`plainRank`; `cardValue` is the composed comparator built from them and is what `trick.ts` and `heuristic.ts` actually call. Both are exported. `cardValue` takes `trump: Suit | null` so it is safe to call during `bid_round_1` scratch evaluation, where no trump exists yet.

---

## 3. Bidding

Both rounds proceed **eldest → dealer's partner → third seat → dealer**. `hand.passes` counts passes *within the current round* and is reset to `0` on entry to `bid_round_2`.

### 3.1 Round 1 — the up-card

Each seat in turn may `pass`, `orderUp`, or `orderUp+alone`. Accepting makes `suitOf(upCard)` trump. The move id is the same for all four seats; only the spoken label differs by role:

| Caller | `label` | Spoken |
|---|---|---|
| eldest or third seat | `Order it up` | "Order it up." |
| dealer's partner | `I assist` | "I assist." |
| dealer | `I take it` | "I take it." |

A successful round-1 call sets `trump`, `makerSeat`, and (if `alone`) `aloneSeat` + `sittingSeat = partnerOf(aloneSeat)`, then transitions to **`dealer_discard` — always, never optionally**, regardless of who called.

Four passes: the dealer **turns it down**. `upCardTurnedDown = true`, `turnedDownSuit = suitOf(upCard)`, phase → `bid_round_2`, `turnSeat` → eldest, `passes` → `0`.

**The up-card stays publicly identified for the entire hand** — `PublicGameView.upCard` is never nulled. Every player saw it, and knowing it is load-bearing for counting the seven trump; it also feeds every `aiSeat`'s card-counting memory. `upCardTurnedDown` distinguishes "buried under the kitty, its suit now forbidden in round 2" from "taken into the dealer's hand".

### 3.2 Dealer pickup and discard

The dealer takes the up-card into hand (6 cards) and discards exactly one **face down**. The discard may legally be the up-card itself — the dealer declining to use it while the suit remains trump.

`dealerDiscard` is stored in `HandState` for the journal and is **never revealed to any client, ever** (V12). The `dealerDiscarded` `Step` has **no card field**, so leaking it is a compile error. `kittyCount` is computed as `3 + (upCardTurnedDown || dealerDiscard !== null ? 1 : 0)` — a function of *whether* a discard exists, never of *which* card it was, so the count cannot be differenced against the visible up-card to identify it (**V13**). The UI treats the discard as an ordinary tap on a temporarily six-card fan; `discard:9C` is a `LegalMove` id like any other.

### 3.3 Round 2 — naming a suit

Each seat may `pass` or `call:<suit>` / `call:<suit>+alone` for any suit **except `turnedDownSuit`**. Three legal suits, six call moves, plus `pass` — seven `LegalMove`s at a normal seat.

Calling the **same colour** as the turned-down suit is **"Next."**; calling the opposite colour is **"Crossing the creek."** Both are surfaced as the spoken line in `BidRecord.say` and as a seat-dependent bonus in `rankMoves` (`+0.45` for next from eldest/third seat, `+0.15` otherwise — see `03-AI-AGENTS.md` § heuristics).

### 3.4 Stick the dealer

`cfg.stickTheDealer` defaults to **`true`**. When three seats have passed in round 2 and the dealer is to act, `pass` is **removed from the legal set**. The dealer must name one of the three legal suits.

A `pass` arriving in this state is rejected with the machine code **`dealer_must_call`**, never `bidding_closed`. These are different situations and the client shows different copy. The UI does not disable the Pass button — it **omits** it, and fires a `"You're stuck."` system chat line first.

With `cfg.stickTheDealer === false` (implemented and tested, not exposed in v1 UI), a fourth pass throws the hand in (§ 5.2).

### 3.5 Going alone

`alone` is not a separate turn. It rides on the winning bid as `orderUp+alone` or `call:H+alone`. `cfg.loner` is `"makerOnly"`: only the maker may go alone, and only simultaneously with the successful call. `cfg.defendAlone` is the literal `false` — the engine branch exists behind the flag and is unit-tested, but it is unreachable in v1 and absent from the UI.

Loner mechanics, each individually tested:

1. The sitting partner's five cards remain in `state.hand.hands` for the journal and the teaching replay, but are **dead** — they are never playable.
2. `project(state, sittingSeat)` reports `handCounts[sittingSeat] === 0` and an empty own `hand`.
3. `legalMoves(state, sittingSeat)` returns `[]`. Any action from that seat is rejected with `sitting_out`.
4. `nextActiveSeat()` skips the seat, so every trick has exactly **three** plays.
5. **Trick-1 lead** is `firstActiveFrom(eldest, sittingSeat)`. When the loner's sitting partner *is* eldest, the opening lead passes clockwise to the next active seat — which is the **dealer's partner**, a defender — not the loner and never the sitting seat.
6. Hand-end card-count assertion (V10): 20 cards played normally, **15** under one loner.

```ts
// packages/euchre-core/src/seats.ts
export const nextSeat = (s: Seat): Seat => ((s + 1) & 3) as Seat;
export const partnerOf = (s: Seat): Seat => ((s + 2) & 3) as Seat;
export const teamOf = (s: Seat): Team => (s & 1) as Team;

export const firstActiveFrom = (s: Seat, sittingSeat: Seat | null): Seat =>
  s === sittingSeat ? nextSeat(s) : s;

export const nextActiveSeat = (s: Seat, sittingSeat: Seat | null): Seat => {
  const n = nextSeat(s);
  return n === sittingSeat ? nextSeat(n) : n;
};
```

> **Spec note:** the folk rule "if the sitting partner would lead, the loner leads" is **not** what this engine does for trick 1. The lead passes clockwise to the next active seat, matching Bicycle/WEF practice and the binding decision. For tricks 2–5 the question cannot arise: the winner leads, and the sitting seat can never win a trick (V9).

---

## 4. Play of the hand

### 4.1 Follow-suit legality

Eldest (or `firstActiveFrom(eldest, sittingSeat)`) leads trick 1. Play proceeds clockwise, skipping the sitting seat. `trick.ledSuit` is `effectiveSuit(firstPlay.card, trump)`.

A player **must follow the led suit if able**, measured by `effectiveSuit`. If void, they may play *anything* — there is no obligation to trump and no obligation to overtrump.

```ts
// packages/euchre-core/src/legal.ts
import { effectiveSuit, type CardId, type Suit } from "./cards";

/**
 * The card-level primitive. `ledSuit === null` means this seat is on lead.
 * Returns a new array; never mutates `hand`. Never returns [] for a non-empty hand.
 */
export function getLegalPlays(
  hand: readonly CardId[],
  ledSuit: Suit | null,
  trump: Suit | null,
): CardId[] {
  if (ledSuit === null) return hand.slice();
  const following = hand.filter((c) => effectiveSuit(c, trump) === ledSuit);
  return following.length > 0 ? following : hand.slice();
}
```

> **Spec note:** `getLegalPlays` is the card-level primitive; `legalMoves(state, seat)` (§ 6.3) is the exported move-level API used by every consumer, and it wraps `getLegalPlays` for the `trick_play` phase. Both live in `legal.ts` and both are covered by the 100%-branch coverage gate.

### 4.2 Trick resolution

`trickWinner(plays, ledSuit, trump)` (§ 2.3): highest trump if any trump was played, otherwise the highest card of the led suit. The winner takes the trick, `tricksWon[teamOf(winner)]++`, and the winner leads the next trick. The trick is pushed to `trickLog` (capped at 5) and `trick` is reset with `index + 1`.

A trick is complete when `plays.length === (sittingSeat === null ? 4 : 3)`.

### 4.3 Reneging

Reneging is **structurally impossible**: illegal cards are absent from the server-computed legal set, unclickable in the client, and never offered to the LLM. The numeric penalty `{ normal: 2, vsLoner: 4, endsHand: true }` remains in `invariants.ts` as a defensive assertion plus a telemetry event. If it ever fires, that is a bug report, not a rules event.

---

## 5. Scoring, game end, and misdeals

### 5.1 The scoring table

Applied at `hand_score`. `makers = teamOf(makerSeat)`, `made = tricksWon[makers]`.

| Situation | `HandResult` | Points | To |
|---|---|---|---|
| Makers take 3 or 4 tricks | `point` | **1** | Makers |
| Makers take all 5 (**march**) | `march` | **2** | Makers |
| **Lone** maker takes 3 or 4 | `lone_point` | **1** | Makers |
| **Lone** maker takes all 5 | `lone_march` | **4** | Makers |
| Makers take 0–2 (**euchred**) | `euchre` | **2** | Defenders |
| **Lone** maker takes 0–2 | `euchre` | **2** | Defenders |
| Four passes, `stickTheDealer: false` | `throw_in` | **0** | — |

`cfg.superEuchre` is the literal `false`; the 4-point all-tricks-to-defenders row does not exist in v1.

```ts
// packages/euchre-core/src/score.ts
import { teamOf } from "./seats";
import type { HandResult, Seat, Team } from "./types";

export interface ScoreOutcome {
  readonly result: HandResult;
  /** Post-clamp, so score[i] + delta[i] is always the new score. */
  readonly delta: readonly [number, number];
}

export function scoreHand(
  makerSeat: Seat,
  aloneSeat: Seat | null,
  tricksWon: readonly [number, number],
  score: readonly [number, number],
  gameTo: number,
): ScoreOutcome {
  const makers = teamOf(makerSeat);
  const defenders = (1 - makers) as Team;
  const made = tricksWon[makers]!;
  const alone = aloneSeat !== null;

  const raw: [number, number] = [0, 0];
  let result: HandResult;
  if (made <= 2) { result = "euchre"; raw[defenders] = 2; }
  else if (made <= 4) { result = alone ? "lone_point" : "point"; raw[makers] = 1; }
  else { result = alone ? "lone_march" : "march"; raw[makers] = alone ? 4 : 2; }

  const next: [number, number] = [
    Math.min(gameTo, score[0]! + raw[0]),
    Math.min(gameTo, score[1]! + raw[1]),
  ];
  return { result, delta: [next[0] - score[0]!, next[1] - score[1]!] };
}
```

### 5.2 Game end and misdeals

`cfg.gameTo` is the literal `10`. A team wins the instant its score reaches 10. Scores **clamp**: a march at 9 wins at 10, not 11 — this is why `delta` is reported post-clamp.

Misdeal handling reduces to the only case a digital deal can produce. Cards cannot be exposed, miscounted, dealt out of turn, or the deck fouled — the engine deals from a persisted 24-card permutation and asserts the count. The one real case is **exhaustion**: with `cfg.stickTheDealer === false`, all four seats pass in round 2. Then emit `{ t: "throwIn" }`, `result = "throw_in"`, `delta = [0, 0]`, `handNo++`, `dealerSeat = nextSeat(dealerSeat)`, `misdealStreak++`, re-enter `startHand`. `misdealStreak` caps consecutive throw-ins at `cfg.misdealLimit` (3), after which the engine emits a "reshuffling" system line; it resets to `0` on any scored hand. `cfg.farmersHand` is `"off"` and `cfg.railroading` is `false` — neither has a code path. Lay-down loners are not auto-resolved: the loner plays the remaining tricks, because the read pause is where the drama lives.

---

## 6. The engine

### 6.1 Determinism and seeded shuffling

Every hand is a pure function of `(seed, handNo)`. `hashSeed` is FNV-1a over `` `${seed}#${handNo}` ``; `mulberry32` is the PRNG; `shuffle` is Fisher–Yates driven by it. The RNG closure lives in `c.vars` (rebuilt in `createVars` from `c.state.seed`), never in `c.state` — which is why the seed is persisted and the closure is not. `deckOrder` is computed and **persisted before the `cutting` phase opens**, so a crash cannot reshuffle a hand a player has already partly seen.

```ts
// packages/euchre-core/src/rng.ts
export function hashSeed(seed: string, handNo: number): number {
  const s = `${seed}#${handNo}`;
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; }
  return h >>> 0;
}

export function mulberry32(a: number): () => number {
  let t = a >>> 0;
  return () => {
    t = (t + 0x6d2b79f5) >>> 0;
    let x = t;
    x = Math.imul(x ^ (x >>> 15), x | 1);
    x ^= x + Math.imul(x ^ (x >>> 7), x | 61);
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
}

export function shuffle<T>(items: readonly T[], rnd: () => number): T[] {
  const a = items.slice();
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(rnd() * (i + 1));
    const t = a[i]!; a[i] = a[j]!; a[j] = t;
  }
  return a;
}

/** Deterministic single cut. Never within 4 of either end, so it is always a real cut. */
export function cutAt(deck: readonly CardId[], rnd: () => number): CardId[] {
  const i = 4 + Math.floor(rnd() * (deck.length - 8));
  return [...deck.slice(i), ...deck.slice(0, i)];
}
```

A replay is therefore `{ seed, moves[] }` — under 3 KB per 10-point match. The raw seed never reaches a client; `playerProfile.getReplay()` re-derives frames server-side (`01-ARCHITECTURE.md`).

### 6.2 Reducer signature and action union

The engine is three pure functions over `GameState`. None of them mutate their argument; each returns a fresh state plus the `Step[]` that describes the transition.

```ts
// packages/euchre-core/src/reduce.ts
export interface ApplyResult { readonly state: GameState; readonly steps: readonly Step[]; }

/** lobby | hand_score(next) -> cutting. Shuffles, persists deckOrder, resets HandState. */
export declare function startHand(state: GameState): ApplyResult;

/** THE seat move. Throws RuleError on anything illegal. */
export declare function apply(state: GameState, seat: Seat, move: Move): ApplyResult;

/** Engine-only advance out of a server-held phase (trick_resolve, hand_score). */
export declare function advance(state: GameState): ApplyResult;
```

`Move` is the action union (from `types.ts`):

```ts
export type Bid =
  | { readonly t: "pass" }
  | { readonly t: "orderUp"; readonly alone: boolean }
  | { readonly t: "call"; readonly suit: Suit; readonly alone: boolean };

export type Move =
  | Bid
  | { readonly t: "cut"; readonly cut: boolean }
  | { readonly t: "discard"; readonly card: CardId }
  | { readonly t: "play"; readonly card: CardId };
```

> **Spec note:** the binding decision names `apply(state, seat, move)` as the single mutation entry point, which covers every *seat* move. `startHand` and `advance` are the two engine-only transitions (there is no seat to attribute them to) and live in the same `reduce.ts`. The `euchreTable` actor calls `startHand` from the `startHand` queue handler and `advance` from the `tick` queue handler; neither is reachable from a browser. This preserves "one rulebook, one writer" — all three are in `reduce.ts` and nowhere else.

Every `Move` maps to a stable `LegalMoveId` string, shared verbatim by the client hit-test whitelist, the AI's `z.enum`, the heuristic ranking key, the coach and the replay log:

`pass` · `orderUp` · `orderUp+alone` · `call:H` · `call:H+alone` · `cut:yes` · `cut:no` · `discard:9C` · `play:JS`

### 6.3 `legalMoves()`

The single source of truth for "what may happen next". It returns `[]` for any seat that is not to act — which is exactly what `PublicGameView.legal` needs, since the field is empty unless `turnSeat === you`.

```ts
// packages/euchre-core/src/legal.ts (continued)
import { SUITS, rankOf, suitOf } from "./cards";
import { partnerOf } from "./seats";
import type { CardId, GameState, LegalMove, Rank, Seat, Suit } from "./types";

const SUIT_NAME: Readonly<Record<Suit, string>> = {
  S: "Spades", H: "Hearts", D: "Diamonds", C: "Clubs",
};
const RANK_NAME: Readonly<Record<Rank, string>> = {
  "9": "Nine", T: "Ten", J: "Jack", Q: "Queen", K: "King", A: "Ace",
};
/** "Jack of diamonds" — the spoken/UI label, also read aloud by HandA11y. */
const cardName = (c: CardId): string => `${RANK_NAME[rankOf(c)]} of ${SUIT_NAME[suitOf(c)].toLowerCase()}`;

function orderLabel(seat: Seat, dealer: Seat): string {
  if (seat === dealer) return "I take it";
  if (seat === partnerOf(dealer)) return "I assist";
  return "Order it up";
}

export function legalMoves(state: GameState, seat: Seat): LegalMove[] {
  const h = state.hand;
  if (state.status !== "active") return [];
  if (seat === h.sittingSeat) return [];
  if (h.turnSeat !== seat) return [];

  switch (h.phase) {
    case "cutting":
      return [
        { id: "cut:yes", move: { t: "cut", cut: true }, label: "Bump" },
        { id: "cut:no", move: { t: "cut", cut: false }, label: "Run 'em" },
      ];

    case "bid_round_1": {
      const order = orderLabel(seat, h.dealerSeat);
      return [
        { id: "pass", move: { t: "pass" }, label: "Pass" },
        { id: "orderUp", move: { t: "orderUp", alone: false }, label: order },
        { id: "orderUp+alone", move: { t: "orderUp", alone: true }, label: `${order}, alone` },
      ];
    }

    case "dealer_discard":
      return h.hands[seat].map((c) => ({
        id: `discard:${c}`, move: { t: "discard", card: c }, label: cardName(c),
      }));

    case "bid_round_2": {
      const stuck = state.cfg.stickTheDealer && seat === h.dealerSeat && h.passes === 3;
      const out: LegalMove[] = stuck
        ? []
        : [{ id: "pass", move: { t: "pass" }, label: "Pass" }];
      for (const s of SUITS) {
        if (s === h.turnedDownSuit) continue;
        out.push({ id: `call:${s}`, move: { t: "call", suit: s, alone: false }, label: SUIT_NAME[s] });
        out.push({
          id: `call:${s}+alone`, move: { t: "call", suit: s, alone: true },
          label: `${SUIT_NAME[s]}, alone`,
        });
      }
      return out;
    }

    case "trick_play":
      return getLegalPlays(h.hands[seat], h.trick.ledSuit, h.trump).map((c) => ({
        id: `play:${c}`, move: { t: "play", card: c }, label: cardName(c),
      }));

    default:
      return [];
  }
}
```

`legalMoves` is never empty in a phase where a seat is to act, except for the sitting seat — the guarantee that makes the AI's `z.enum(legalMoveIds)` always constructible and the 240 s abandon auto-play always able to move.

### 6.4 Rule errors

`apply` throws `RuleError(code, legal)` and nothing else. The `euchreTable` run loop converts it to a `UserError` with `metadata.legal`, completes the message with a typed rejection, and never leaves it unacked (`01-ARCHITECTURE.md`).

| `RuleCode` | Raised when |
|---|---|
| `not_your_turn` | `seat !== hand.turnSeat` |
| `wrong_phase` | the move kind does not belong to `hand.phase` |
| `illegal_move` | the move is not in `legalMoves(state, seat)` and no more specific code applies |
| `must_follow_suit` | a `play` whose `effectiveSuit` differs from `ledSuit` while a following card is held |
| `card_not_in_hand` | a `play` or `discard` naming a card the seat does not hold |
| `bidding_closed` | a bid arriving after trump is set |
| `dealer_must_call` | `pass` from the stuck dealer — **distinct from `bidding_closed`** |
| `sitting_out` | any action from `sittingSeat` |
| `stale_turn` | the caller's `turnId` nonce does not match `state.turnId` |

### 6.5 Projection

`project(state, seat)` is the only producer of `PublicGameView`; `projectSteps(steps, seat)` is the only producer of client-visible `Step[]`. The view type has **no field capable of holding** a foreign hand, a buried kitty card, or the dealer's discard, so a leak is a compile error. `projectSteps` blanks the card on any `dealt` packet not addressed to this seat; `dealerDiscarded` has no card field to blank. `applyOptimistic(view, move)` is the browser's local preview and must agree with `apply` on the human's own legal set (V22, test 38).

---

## 7. Phase state machine

`turnId` is a fresh nonce minted on every advance. `turnSeat` is `null` in every engine-only phase.

| Phase | Who acts | Legal actions | Transitions |
|---|---|---|---|
| `lobby` | SvelteKit server (trusted) | actor `create` + internal `startHand` | → `cutting`. `createState` mints `internalToken` and snapshots the three personas. The browser never creates a game. |
| `cutting` | third seat = `(dealer + 3) & 3` | `cut:yes`, `cut:no` | → `deal` on either move. `deckOrder` is already persisted. If the cutter is an AI and does not answer within 2200 ms, `onTempoGate` auto-applies `cut:no`. The human cuts only when `dealerSeat === 1`. |
| `deal` | engine only | none | Atomic. Deals per `dealPlan`, emits `dealt` + `upCardTurned` steps, asserts 24 cards. → `bid_round_1`, `turnSeat = nextSeat(dealerSeat)`. |
| `bid_round_1` | eldest → dealer's partner → third → dealer | `pass`, `orderUp`, `orderUp+alone` | Any call → `dealer_discard` (mandatory). Sets `trump = suitOf(upCard)`, `makerSeat`, and on `alone` also `aloneSeat` + `sittingSeat`. Four passes → `bid_round_2`, `upCardTurnedDown = true`, `turnedDownSuit` set, `passes = 0`, `turnSeat` = eldest. |
| `dealer_discard` | `dealerSeat` only | `discard:<one of six>`, incl. the up-card | → `trick_play`, `turnSeat = firstActiveFrom(eldest, sittingSeat)`. Hand size back to 5. `dealerDiscard` stored, never revealed. |
| `bid_round_2` | eldest → dealer's partner → third → dealer | `pass`, `call:<suit ≠ turnedDownSuit>`, `call:<suit>+alone` | Any call → `trick_play`, `turnSeat = firstActiveFrom(eldest, sittingSeat)`. Three passes with `stickTheDealer` → the stuck-dealer sub-state. Four passes with `stickTheDealer: false` → `throwIn`, `handNo++`, deal left, `misdealStreak++`, → `startHand`. |
| `bid_round_2` (stuck dealer) | `dealerSeat` only | `call:<suit ≠ turnedDownSuit>` (+`alone`) — **`pass` removed** | `pass` → `RuleError("dealer_must_call")`. Phase value is still `bid_round_2`; the sub-state is `stickTheDealer && seat === dealer && passes === 3`. → `trick_play`. |
| `trick_play` | `turnSeat`, skipping `sittingSeat` | `play:<card>` where `effectiveSuit === ledSuit`, or any card if void | After the last active seat plays → `trick_resolve`. |
| `trick_resolve` | engine only — a **real server-held phase** | none: *the next card does not exist yet* | Server holds 700–1000 ms (1100 ms when the trick seals a euchre) via `c.schedule.after(gate, "onTempoGate", turnId)`. `advance()` → `trick_play` with `turnSeat = winner` (asserted ≠ `sittingSeat`), or → `hand_score` after trick index 4. |
| `hand_score` | engine only — server-held | none | Applies the § 5.1 table, clamps at 10, emits `handScored`, journals `{seed, moves[]}` to `playerProfile`. Held 1400–2400 ms. → `game_over` if a score reached 10; else `handNo++`, deal left, `misdealStreak = 0`, → `startHand` → `cutting`. |
| `game_over` | engine only | none — table is read-only, `snapshot()` still serves the recap | `recordMatch` → `playerProfile` (retried by `flushProfile`), final `sync`, `gameEnd` to each `aiSeat` then destroy all three, clear `activeGameId`, self-`destroy()` after 10 minutes. |

### 7.1 Cross-cutting paths

| Path | Behaviour |
|---|---|
| **Human disconnect** | The game **waits**. There is no human turn timer — deliberately, so it is not re-added. `conn.onOpen` → `snapshot()` → hard resync with `steps: []`, so the client snaps without animating. |
| **Human idle 90 s** | `onNudge`: the partner speaks a nudge line. No state change, no `apply` call. |
| **Human idle 240 s** | `onAbandon` enqueues a `tick` that auto-plays `rankMoves(state, turnSeat)[0]` via `apply`. Two consecutive fires flip `status` to `"abandoned"`; the match is still journalled. |
| **Two tabs** | Both connect as seat 0 and both receive `sync`. The second submit fails the `turnId` nonce and returns `UserError("stale_turn")` with `metadata.legal`; the client snaps back. |
| **Actor restart** | `saveState({immediate:true})` runs before fan-out and before `complete()`, so a crash mid-mutation leaves the message unacked; `appliedSeq` lives in the same `c.state` as the mutation, making redelivery a no-op by construction. `createVars` rebuilds the RNG from `c.state.seed`. |
| **Crash between ack and arm** | Consistent state, no queued message, no armed timer — a permanent stall. The `onWake` reconciler re-derives the expected pending action from `(phase, turnSeat, turnId, pending)` and re-arms the timer, re-dispatches the AI, or releases a parked decision whose `revealAt` has passed. Individually tested. |
| **AI timeout** | Ladder: forced (no API call) → constrained LLM at 2200 ms → `thinking{extended:true}` at +400 ms → at **4000 ms from dispatch** `onAiTimeout` fires unconditionally and auto-plays `rankMoves()[0]` as `PlayerAction { kind: "auto", reason: "ai_timeout" }`. Identical path if the `aiSeat` is unreachable. A rejected AI move falls back to heuristic top-1 and is **never** retried as the same message. |

---

## 8. Invariants a validator must enforce

`invariants.ts` exports `assertInvariants(state)`, called after every `apply`/`advance` in dev and in every CI test run. Violations throw; they are never surfaced to a player.

1. **V1 — turn/phase.** The acting seat equals `hand.turnSeat`, and the move kind belongs to `hand.phase`. Otherwise `not_your_turn` / `wrong_phase`.
2. **V2 — bid vocabulary.** Round 1 admits only `pass | orderUp{alone}`. Round 2 admits only `pass | call{suit, alone}` with `suit !== turnedDownSuit`.
3. **V3 — stuck dealer.** With `stickTheDealer && phase === "bid_round_2" && seat === dealerSeat && passes === 3`, `pass` is not in the legal set and is rejected as `dealer_must_call`.
4. **V4 — alone.** `aloneSeat !== null` implies `aloneSeat === makerSeat`, `sittingSeat === partnerOf(aloneSeat)`, and both were set in the same transition as `trump`. `alone` can never be declared after the first lead.
5. **V5 — dealer discard.** Required **iff** a round-1 call succeeded. The discarded card was one of the dealer's six; `hands[dealerSeat].length === 5` afterwards; `dealerDiscard !== null` exactly once per hand.
6. **V6 — card ownership.** Every played or discarded card was in that seat's hand immediately before the move, and is removed from it immediately after.
7. **V7 — follow suit.** For every `play` with `trick.plays.length > 0`: `effectiveSuit(card, trump) === trick.ledSuit` **or** the seat held no card with `effectiveSuit === ledSuit`. Computed with `effectiveSuit`, never `suitOf`.
8. **V8 — sitting out.** `legalMoves(state, sittingSeat) === []`, `sittingSeat` never appears in `trick.plays`, and `project(state, sittingSeat).hand` is empty.
9. **V9 — lead succession.** `trickWinner` leads the next trick and is never `sittingSeat`. Asserted, not defended against.
10. **V10 — card accounting.** Exactly 5 tricks per hand; all active hands empty simultaneously; **20** cards played with no loner, **15** with one. Across the whole hand, `dealt(20) + kitty(4) === 24` and every `CardId` appears exactly once in `deckOrder`.
11. **V11 — scoring.** `delta` is drawn only from the § 5.1 table, is non-negative in both components, has at most one non-zero component, and `score[i] + delta[i] === min(gameTo, score[i] + raw[i])`. Scores are monotonic non-decreasing and clamp at `gameTo`.
12. **V12 — hidden information.** `kitty[1..3]` and `dealerDiscard` are never present in any `PublicGameView`, any projected `Step`, any chat line, or any replay frame — including post-game. Enforced structurally (no field exists) and verified by the 10 000-game redaction fuzz.
13. **V13 — discard non-inference.** `project()` output is byte-identical for two states differing only in *which* card the dealer discarded (holding the resulting hand multiset fixed). `kittyCount` is a function of `(upCardTurnedDown, dealerDiscard !== null)` only, and `upCard` is never nulled by the discard.
14. **V14 — up-card persistence.** `upCard` is non-null from the `upCardTurned` step until the hand ends, in every projection, regardless of `upCardTurnedDown` or the pickup.
15. **V15 — effective-suit purity.** `effectiveSuit(c, null) === suitOf(c)` for all 24 cards; `effectiveSuit` is the identity except for exactly one card per trump suit (the left bower).
16. **V16 — trump census.** With any trump set, exactly 7 of the 24 cards satisfy `effectiveSuit === trump`; the same-colour suit has 5; each off-colour suit has 6.
17. **V17 — comparator totality.** Within one trick, `cardValue` is injective over the played cards, so `trickWinner` is total and tie-free.
18. **V18 — bounded collections.** `trickLog.length ≤ 5`, `bids.length ≤ 8`, `recentMoveIds.length ≤ 32`, `passes ≤ 4`, `trick.plays.length ≤ 4`. Nothing in `GameState` grows with `handNo`.
19. **V19 — legal-set non-emptiness.** In any phase with a non-null `turnSeat`, `legalMoves(state, turnSeat).length ≥ 1`. This is what guarantees the abandon auto-play and the AI `z.enum` always have a value.
20. **V20 — determinism.** Replaying `{seed, moves[]}` through `startHand`/`apply`/`advance` reproduces every intermediate `GameState` bit-for-bit. No `Date.now()`, no `Math.random()`, no ambient input anywhere in `@euchre/core`.
21. **V21 — purity.** `apply`, `advance`, `startHand`, `legalMoves`, `project` and `projectSteps` never mutate their arguments. Asserted by deep-freezing inputs in tests.
22. **V22 — optimistic agreement.** For every state reachable by the human and every `LegalMove` in their set, `applyOptimistic(project(s, 0), move)` equals `project(apply(s, 0, move).state, 0)` on every field the client renders.

---

## 9. Test plan

`packages/euchre-core/src/__tests__`, `vitest@4.1.10`, `@vitest/coverage-v8@4.1.10` with a 100%-branch gate on `reduce.ts`/`legal.ts`/`score.ts`, `fast-check@4.9.0` for properties. All 40 cases run with no network and no actor.

**Effective suit and ranking** — `effective-suit.spec.ts`

1. `effectiveSuit("JD", "H") === "H"` — left bower of hearts. Also `JH→H`, `JS→S` with `trump: "S"`, `JC→S` with `trump: "S"`.
2. `effectiveSuit("JD", "S") === "D"` — a jack of the *other* colour is not a bower.
3. `effectiveSuit(c, null) === suitOf(c)` for all 24 cards.
4. `trumpRank("JH","H") === 8`, `trumpRank("JD","H") === 7`, `trumpRank("AH","H") === 6`, `trumpRank("9H","H") === 2`, `trumpRank("AS","H") === 0`.
5. With `trump: "H"`, exactly 7 cards have `effectiveSuit === "H"`; diamonds has 5; spades and clubs have 6 each. Repeated for all four trumps.
6. `plainRank("JS") === 3` and `plainRank("QS") === 4` — in a plain suit the jack sits below the queen.
7. Property (fast-check, 200 000 random `(card, trump)` pairs): `effectiveSuit` equals the reference bower predicate, and `trumpRank > 0 ⟺ effectiveSuit === trump`.

**Follow-suit legality** — `legal.spec.ts`

8. Hand `["JD","9C","TC","KC","AC"]`, trump `H`, led `D` → `getLegalPlays` returns all five cards: the hand is **void in diamonds** because `JD` is a heart.
9. Same hand, trump `H`, led `H` → returns `["JD"]` only. Exactly one legal play.
10. Same hand, trump `H`, `ledSuit: null` → returns all five (on lead, anything goes).
11. Hand `["AS","KS","9H","TD","QC"]`, trump `S`, led `S` → `["AS","KS"]`.
12. Hand `["AS","KS","9H","TD","QC"]`, trump `C`, led `C` → `["QC"]`; led `H` → `["9H"]`; led `D` → `["TD"]`.
13. Trump `S`, led `S`, hand contains `JC` and `9D` → legal set is `["JC"]`: the left bower **must** be played to follow trump.
14. Trump `S`, led `C`, hand is `["JC","AH"]` → legal set is `["JC","AH"]`: with spades trump the hand is **void in clubs**, so `JC` does not satisfy the club lead and everything is legal.
15. `legalMoves` in `bid_round_2` with `turnedDownSuit: "H"` returns 7 entries (`pass` + 3 suits × {plain, alone}) and contains no `call:H`.
16. `legalMoves` in `bid_round_2` for the stuck dealer (`stickTheDealer: true`, `passes === 3`) returns 6 entries and **no `pass`**; `apply(state, dealer, {t:"pass"})` throws `RuleError("dealer_must_call")`, not `"bidding_closed"`.
17. With `stickTheDealer: false` the same state returns 7 entries and `pass` is accepted.
18. `legalMoves(state, sittingSeat)` returns `[]` in every phase; `apply(state, sittingSeat, anyMove)` throws `RuleError("sitting_out")`.
19. Property (fast-check, 10 000 random deals × random trumps × random leads): the legal set is never empty for a non-empty hand, and never admits a renege.

**Trick resolution** — `trick-winner.spec.ts`

20. Trump `D`, led `C`, plays `9C, AH, KH, QH` → `9C` wins. A nine of the led suit beats an ace of anything else.
21. Trump `H`, led `S`, plays `AS, 9H, KS, QS` → `9H` wins: any trump beats any plain card.
22. Trump `H`, led `S`, plays `AS, 9H, JD, KS` → `JD` wins: the left bower is trump and outranks the `9H`.
23. Trump `H`, led `S`, plays `AS, JD, JH, KS` → `JH` wins: right beats left.
24. Trump `H`, led `H` (led by `JD`), plays `JD, AH, KH, 9H` → `JD` wins, and `trick.ledSuit === "H"`.
25. Trump `S`, led `D`, plays `9D, TD, JD, AD` → `AD` wins. With spades trump `JD` is an ordinary diamond jack, below the queen.
26. Three-play trick under a loner: trump `C`, led `H`, plays `KH(seat1), AH(seat3), 9C(seat0)` → `9C` wins for seat 0.

**Loner** — `loner.spec.ts`

27. Loner is eldest (`dealer=0, alone=1, sitting=3`) → trick-1 lead is seat 1.
28. Loner is dealer's partner (`dealer=0, alone=2, sitting=0`) → trick-1 lead is seat 1 (eldest is active).
29. Loner is third seat (`dealer=0, alone=3, sitting=1`) → eldest is sitting, so trick-1 lead is **seat 2, the dealer's partner — a defender**, not the loner.
30. Loner is the dealer (`dealer=0, alone=0, sitting=2`) → trick-1 lead is seat 1.
31. Under a loner, every trick has exactly 3 plays, `nextActiveSeat` never returns `sittingSeat`, and at hand end exactly **15** cards were played (V10).
32. `project(state, sittingSeat).hand === []` and `handCounts[sittingSeat] === 0` from the moment the call lands; the same seat's five cards are still present in `state.hand.hands` for the journal.

**Scoring** — `scoring.spec.ts` (table-driven over all seven rows)

33. `made=3 → point, +1 makers`; `made=4 → point, +1 makers`; `made=5 → march, +2 makers`; `made=5 alone → lone_march, +4 makers`; `made=3 alone → lone_point, +1 makers`; `made=2 → euchre, +2 defenders`; `made=0 alone → euchre, +2 defenders`.
34. Clamp: `score=[9,4]`, makers = team 0, `made=5` → `delta === [1,0]` and the new score is `[10,4]`, **not** `[11,4]`.
35. Clamp under a loner: `score=[7,2]`, lone march by team 0 → `delta === [3,0]`, new score `[10,2]`.
36. A euchre credits the **defenders** even when the makers are the human's team: `makerSeat=0, made=1, score=[5,5]` → `delta === [0,2]`.
37. Throw-in (`stickTheDealer: false`, four passes in round 2): `result === "throw_in"`, `delta === [0,0]`, `handNo` incremented, `dealerSeat` moved left, `misdealStreak === 1`.

**Bidding, misdeal, integration** — `misdeal.spec.ts`, `stick-the-dealer.spec.ts`, `full-game.spec.ts`, `optimistic-agreement.spec.ts`, `redaction.fuzz.spec.ts`

38. `applyOptimistic` agrees with `apply` on the human's own legal set across 10 000 random reachable states (V22).
39. Round-1 `orderUp` from any of the four seats always routes to `dealer_discard`; the dealer may legally discard the up-card itself, after which `hands[dealer].length === 5`, `dealerDiscard === upCard`, and `project().upCard` is unchanged (V13/V14).
40. Seeded full game to 10 plays headless in **under 50 ms**, ending with exactly one team at 10; and the 10 000-game redaction fuzz finds no `CardId` in any seat-`s` payload — live or replayed — that belongs to another hand, the buried three, or the dealer's discard.

Supplementary property gates run inside the above files: `misdealStreak` reaching `cfg.misdealLimit` still moves the deal left; `bids.length ≤ 8` after a full two-round auction; and deep-frozen inputs to `apply` never throw (V21).

---

## 10. The table script

The exact phrases spoken at a real euchre table. Engine-generated `chat` lines of `kind: "call"` use these **verbatim** — they are not LLM output and are never filtered. `03-AI-AGENTS.md` seeds every persona's phrasebook from this list; AI `banter` varies *around* it, never replaces it.

| Moment | Spoken |
|---|---|
| Before the deal | "Your deal." |
| Cut offered | cutter knocks: **"Bump."** — or declines: **"Run 'em."** |
| Up-card turned | *(silence; eyes to eldest)* |
| Eldest, round 1 | **"Pass."** / **"Order it up."** |
| Dealer's partner, round 1 | **"Pass."** / **"I assist."** |
| Third seat, round 1 | **"Pass."** / **"Order it up."** |
| Dealer, round 1 | **"I take it."** / **"Turn it down."** |
| Dealer picks up | "Pick it up." *(as they take the card and slide one out face down)* |
| Round 2, same colour as turned down | **"Next."** |
| Round 2, opposite colour | **"Crossing the creek."** |
| Round 2, plain call | **"Spades."** / "Hearts." / "Diamonds." / "Clubs." |
| Stuck dealer | table: **"You're stuck."** → dealer: "Hearts, I guess." |
| Going alone | **"Alone."** / "I'm going alone." / "By myself." |
| Partner sits out | *(slides cards face down)* **"Good luck, partner."** |
| On lead | "Your lead." |
| Partner's card is winning | **"Good."** *(meaning: don't waste one)* |
| Urging a trump | "Take it." / "Ours." |
| Trick won | **"That's ours."** |
| All five tricks | **"March!"** |
| Makers set | **"Euchre!"** / **"Set 'em."** |
| Score called | "Six-four." |
| Reaching 9 | **"We're in the barn."** |
| Idle nudge (90 s) | "Your lead, partner." |

**Table talk is illegal in euchre** — no hints about card content, verbal or otherwise. This is a rule, not etiquette, and it is the highest-risk LLM leak in the product. Personas may talk about *feel*, *tempo*, *score* and *history*; never about holdings. Enforcement is layered: public-state-only prompting, a 90-character cap, an instruction-level ban on claims about one's own cards, `screenBanter()` rejection of any rank+suit token naming a card outside `played ∪ {upCard} ∪ {trump suit name}`, and phrasebook substitution on rejection rather than regeneration. See `03-AI-AGENTS.md` § banter and `04-FRONTEND-UX.md` § talk log.
