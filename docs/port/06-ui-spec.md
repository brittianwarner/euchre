# 06 - UI / UX Spec (native Rune client port)

Source of truth: `/Users/brittianwarner/goods/euchre` (read-only). Every statement below was read from the Svelte source; file references are relative to that repo (`src/...`). Where the source has dead code or the older design doc (`docs/04-FRONTEND-UX.md`) disagrees with the shipped code, **the shipped code wins** and the discrepancy is called out in section 20.

Audience: a native implementer building `apps/desktop` with Rune who has not seen the Svelte code.

Conventions
- "You" is always the local player. `view.you` is the viewer's seat 0..3 (the TS source supports any seat, the UI always computes relative positions from `you`).
- Seats are numbered 0..3 clockwise. Team = `seat % 2`. Partner = `(seat + 2) % 4`. Relative offset `rel = (seat - you + 4) % 4`: 0 = You (bottom), 1 = Left opponent (west), 2 = Your partner (top), 3 = Right opponent (east).
- `PublicGameView` fields referenced below are exactly those in `src/lib/euchre/types.ts:462` (`v`, `you`, `status`, `phase`, `handNo`, `dealerSeat`, `turnSeat`, `turnId`, `score[2]`, `trump`, `makerSeat`, `aloneSeat`, `sittingSeat`, `upCard`, `upCardTurnedDown`, `turnedDownSuit`, `hand[]`, `handCounts[4]`, `kittyCount`, `trick{index,ledSuit,plays[{seat,card}],winnerSeat}`, `trickLog[]`, `tricksWon[2]`, `passes`, `bids[{seat,move,say}]`, `legal[{id,move,label}]`, `result`, `delta`, `winnerTeam`, plus optional `players[{seat,name,blurb,style}]`, `reviewTricks`, `awaitingTrickReview`, `rules{stickTheDealer,requireNaturalTrump}`, `observations`). The UI renders ONLY from this view; it never holds other hands.
- GamePhase values: `lobby, cutting, deal, bid_round_1, dealer_discard, bid_round_2, trick_play, trick_resolve, hand_score, game_over`. GameStatus: `active | complete | abandoned`. HandResult: `point | march | lone_point | lone_march | euchre | throw_in`.
- LegalMoveId strings: `pass`, `orderUp`, `orderUp+alone`, `call:<S|H|D|C>`, `call:<suit>+alone`, `cut:yes`, `cut:no`, `discard:<card>`, `play:<card>`. Legal labels (`src/lib/euchre/legal.ts:181-239`): `Bump`, `Run 'em`, `Pass`, order label (`I take it` for dealer, `I assist` for dealer's partner, `Order it up` otherwise), `<order>, alone`, card name for play/discard, `<SuitName>` and `<SuitName>, alone` for calls.
- CardId = rank char (`9 T J Q K A`) + suit char (`S H D C`), e.g. `TH`, `JS`.

---------------------------------------------------------------------------------------------------

## 1. Design tokens

### 1.1 Global (`src/app.css`)

| Token | Value |
|---|---|
| `--font-sans` | `'DM Sans', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif` |
| `--font-serif` | `'Source Serif 4', 'Iowan Old Style', Georgia, serif` |
| `--club-cream` | `#f4f3e9` |
| `--club-ink` | `#233e32` |
| `--club-forest` | `#153f36` |
| `--club-lime` | `#d4ed9b` |
| root `font-family` | `var(--font-sans)`; `font-synthesis: none`; `text-rendering: optimizeLegibility`; antialiased |
| body | `margin: 0; min-width: 320px` |
| focus ring | `:focus-visible { outline: 3px solid #87ac49; outline-offset: 4px }` (table hand overrides to `#f5d98e` on felt, `#295744` in rack) |
| selection | background `#d4ed9b`, text `#153f36` |

Fonts loaded from Google Fonts: DM Sans weights 400;500;550;600;650;700 and Source Serif 4 (opsz 8..60) weights 400;500;600;700 + italic 400. A native client should bundle both families (both are OFL). Tailwind (forms + typography plugins) is imported but essentially unused by these components; all styling is scoped CSS.

### 1.2 Palette actually used (grouped by role)

Light "club" surfaces (home, header strip, side panel, dialogs, `/new`):
- cream `#f4f3e9` (page bg, dialogs, saved-tables), card white `#fffdf6`, soft paper `#fffdf5`, hairline `#dce1d4`, `#d4dccb`, `#d9ddcf`, `#cad3c1`, `#d3d9c9`.
- ink `#233e32`, strong green button `#285641` (hover `#1a4431` / `#1b4531`), secondary text `#58695b`, `#53664c`, `#586b51`, `#69765f`, `#6d7865`, `#72766a`, `#5f695d`.
- panel hover `#e9eddf`, `#e6eadc`, `#e6eecf`; disabled `#dce3d3` bg / `#52624b` text / `#bdc9b2` border.
- Hero (home): bg `#153f36` (forest), text `#f5f1e5`, eyebrow `#d5e7bb`, lede `#d2dcd4`, note `#a8bcb0`, lime button `#d4ed9b` (hover `#e3f6be`), button text `#143e32`, muted `#9db3a4`, `#a1b9a6`, `#a8b893`, divider `#c6d7b92b`.
- Red suit accent `#b44835` (home), `#aa342e` (score hearts), `#b64e3c` (discard red), `#ba5e46` (confetti).
- Rule cards bg `#e9ecdf`, number color `#6b7664`.

Table (felt) surfaces:
- Table area background `#17452f`, text `#fff8e8` (`.table-layout`).
- Felt component (TableStage) text `#fffdf3`; light-green labels `#d9e6cb`, `#e1e8d9`, `#d3decf`, `#e0e9d7`, `#bdceb2`, `#b9d2c1`.
- Gold/cream accents on felt: winner outline `#eed591`, active seat `#e6d08c` / `#ffe5a4` / `#edd79a`, partner marker `#e4cf91`, trump glyph `#e0cf97`, lone marker `#f2d887`, quote speaker `#e9d79f`, trick-won text `#ffecb7`, dealer chip bg `#f4ead0` text `#35482f` shadow `0 2px 0 #b4a57a`, maker chip bg `#f4ead0` text `#243d31`, primary-on-felt button bg `#f5e8c3` text `#233e32` border `#ead8aa` (hover `#fff0c7`; disabled bg `#e8eadb`, text `#6a7466`, border `#d5ddc7`), selected-card outline on felt `#f5d98e`, card label text on felt `#f7f1df` / sub `#ddd5bd`.
- Seat underline `#ffffff26`; active seat tint `#ffffff0a`; led pill bg `#ffffff0c`; hand-area top border `#e2dbbd26`.
- Taken-stack: `repeating-linear-gradient(45deg,#e8e2ca 0 1px,#285743 1px 4px)`, border `2px solid #f8f0db`, shadows `2px 2px #c6c6b5, 4px 4px #e7e5d8`.
- Notice/error box: bg `#fff1d9`, border `#d5b68c`, text `#233e32`, radius 8.
- Illegal toast (unused `TableStatusBar`): bg `#fff1e9`, border `#e0bbac`, text `#904d37`.

Dark "scrapbook" surfaces (`/games`, `/games/[matchId]`, `/settings`, IdentityGate, tour dialog):
- page bg `#0b0906`, text `#f2e8d5`, serif font; muted `#93876f`, soft `#c9b89a`, accent gold `#e8c27a`, cards `rgba(15,20,14,.6)`, input bg `#241b10` border `#6a5638`, green button `#3d6b38` (hover `#4a7f44`) text `#f4f7e8`, error `#e0a06e`, success tint `rgba(61,107,56,.35)` text `#dff0d6`, error banner bg `rgba(74,32,24,.5)` border `rgba(224,140,110,.4)` text `#f7e2d8`, outcome chips: won bg `rgba(61,107,56,.35)` fg `#b7dcaa`; lost bg `rgba(107,56,56,.35)` fg `#e0b0a6`; abandoned bg `rgba(90,80,60,.35)` fg `#cbbf9e`.
- Tour dialog sheet bg `#12362b`, border `rgba(232,194,122,.35)`, primary button bg `#d4ee86` fg `#12362b` border `#e8c27a`.

Felt in the decorative layer: base `#245a49`, edge `#10382c`, plane tint `#a4c49e` (see section 16).

Global nav pill: bg `#15352eee`, border `#aec29338`, link `#d9e3d4`, hover bg `#d4ed9b1a`, active bg `#d4ed9b` fg `#153f36`.

### 1.3 Type scale (key styles)

| Use | Font | Size / weight | Notes |
|---|---|---|---|
| Home h1 | serif 500 | `clamp(52px,6.2vw,94px)`, lh 1.02, ls -0.055em | em in lime, weight 400 |
| Section h2 (home) | serif 500 | `clamp(34px,3.4vw,48px)`, lh 1.12, ls -0.045em | |
| Rule h3 | serif 500 | 26px lh 1.2 ls -0.025em | |
| Eyebrow | sans 600 | 10px ls 0.18em uppercase | home |
| Brand wordmark `euchre♣` | serif 600 | 38-40px ls -0.07em; club glyph 23-25px, margin-left 5-6px | |
| `/new` h1 | serif 500 | 38px lh 1.2 | |
| Hand heading h1 | serif 500 | 25px (28 on >=701px && >=501px tall) | |
| Instruction line | sans | 17px lh 1.3 (18 base) | |
| Seat name | sans 600 | 17px (15 mobile-ish, 13 <=700px) | |
| Seat detail | sans | 14px (13/11) | |
| Trump heading | serif 500 | 26px desktop, 21-25 smaller | |
| Score numerals | sans 600 | 36px (30 <=700, 25 <=420) tabular-nums | |
| Card label | sans 600 | 15px, small 13px | |
| Buttons | sans 600 | 17-19px | min-height 48-56px |

### 1.4 Spacing / radii / elevation
- Radii: felt/cards `7px` (card art), buttons `8px`, hero `18px`, dialogs `16px`/`0.9rem`, phone felt `12px`, `/new` panel `16px`, pill nav `999px`.
- Min tap target 44px everywhere; primary actions 48-56px.
- Card physical shadow (played): `0 2px 0 #d9d8ca, 0 4px 0 #abb3a0, 0 9px 15px #082b2350`. Hand card artwork: `0 1px 0 #fff, 0 3px 0 #c4c7b9, 0 7px 13px #16382c24`; selected: `0 4px 0 #bac2b1, 0 15px 20px #15301d30`. Up-card/deck: `0 3px #c6c9ba, 0 6px #aeb7a6, 0 12px 20px #0d281b55` + `transform: perspective(900px) rotateX(4deg) rotate(-3deg)`.
- Settings panel shadow (overlay mode): `-16px 0 48px #10251d30`; backdrop `#10251d55`; modal backdrops `#10251dcc` (review), `rgba(6,5,3,.68)` rules, `.72` tour.

---------------------------------------------------------------------------------------------------

## 2. Route map and navigation

| Route | File | Rendering | Purpose |
|---|---|---|---|
| `/` | `src/routes/+page.svelte` | SSR/static | Landing page |
| `/new` | `src/routes/new/+page.svelte` | SSR | New-game options form (GET `/play?stick=..&natural=..`) |
| `/play` | `src/routes/play/+page.server.ts` | server redirect | Creates a table then 302 to `/play/<gameId>` (see 2.2) |
| `/play/[gameId]` | `play/[gameId]/+page.svelte` | `ssr = false` (client only) | The table |
| `/games` | `games/+page.svelte` | SSR | Saved tables + lifetime stats + match history (email "identity") |
| `/games/[matchId]` | `games/[matchId]/+page.svelte` | SSR | One finished match: totals + per-hand list |
| `/settings` | `settings/+page.svelte` | SSR | Opponent persona editor + table prefs (email identity) |
| `/cardtest` | `cardtest/+page.svelte` | dev only | Renders `Card.svelte` alone in Threlte (omit from port) |
| `/api/rivet/*`, `/api/table-session/[gameId]` | | server | Transport / session mint (backend lane) |

Native client mapping: the desktop app is a single window with these screens: Home, New game, Table, Games (history), Match detail, Settings. A Rules dialog, Tour dialog and Review dialog are modals on the Table. A "Table settings" side panel is on the Table.

### 2.1 Global navigation (`src/lib/ui/GlobalNav.svelte`)
- Rendered on every route EXCEPT the live table (`page.route.id === '/play/[gameId]'`).
- Links (label / href / title): `Play` `/new` "Deal a new hand"; `Your games` `/games` "Your past games"; `Settings` `/settings` "Edit the opponents". `aria-current="page"` when `here === href || (href !== '/new' && here.startsWith(href))`.
- Non-home: fixed pill top-right (`top: max(.6rem, safe-area-top)`, `right: max(.6rem, safe-area-right)`, z 200). A leading `♣` link (20px, aria-label "Euchre home") to `/`. Pill: padding 5px, gap 3px, links min 44x44, padding 0 17px, font 12px/550, radius 999px.
- Home (`/`): absolute header, height 104px (86px <=760), max-width 1460, padding 0 50px (27px <=760, 22px <=380), left: brand `euchre♣` + small caption `THE CARD CLUB` (8px/600, ls .18em, max-width 48px, left border `#a8b2a0`, hidden <=760), right: the same three links, transparent, link color `#42523e`, hover bg `#e5e9d8`, radius 6px.
- Layout chrome: `+layout.svelte` renders `<GlobalNav/>` then the page; favicon `src/lib/assets/favicon.svg`.

### 2.2 Create-table flow (server)
`/play` load: (1) generate `gameId = crypto.randomUUID()`; (2) read optional email identity cookie; if present fetch `playerProfile.getSettings()` with 2000 ms race timeout -> `personasFromSettings`, else `defaultPersonas()` (warn "Persona settings unavailable; starting with kitchen-table players"); (3) create `euchreTable` actor key `['table', gameId]` with input `{ ownerUserId: guest identity, personas, profileUserId, cfg: { gameTo: 10, stickTheDealer: query.stick === 'true', requireNaturalTrump: query.natural === 'true' } }`; (4) remember table in cookie; (5) redirect 302 `/play/<gameId>`. In the native port this becomes `POST /tables` on the axum API (backend lane) followed by connect; the desktop client shows the "A seat at the table" loading state (section 14) while creating.

---------------------------------------------------------------------------------------------------

## 3. Screen: Home `/`

Title: `Euchre — A little nerve. A great hand.`; meta description: `Your new favorite card table. Play classic partnership euchre with computer opponents, a beautiful 3D table, and a little friendly competition.`

Page bg `#f4f3e9`, ink `#233e32`; main padding `104px 28px 0` (86px 14px 0 at <=760; at >=1500 horizontal padding `max(28px, (100vw-1460px)/2)`).

### 3.1 Hero (rounded 18px forest panel, min-height 620px, grid 1.1fr / 1fr, padding `70px 5.3% 96px`, decorative dot grain: `radial-gradient(#bdd4b1 .5px, transparent .5px)` 5px grid, opacity .25, masked by `linear-gradient(120deg, transparent, #000)`)
Left column copy (verbatim):
- Eyebrow: `THE TABLE IS YOURS` preceded by a 6px lime dot with `0 0 0 4px #d4ed9b15` halo.
- H1: `A little nerve.` / line break / `A great ` + `<em>hand.</em>` (em colored lime, weight 400).
- Lede: `Five cards. Four seats. Endless good calls.` (break on desktop) ` The classic card game, with a fresh seat at the table.` (15px/1.75, `#d2dcd4`).
- Actions: primary link `Let’s play ↗` -> `/new` (lime button, radius 7, padding 17x23, gap 48 between text and arrow, 14px/650; hover bg `#e3f6be`, translateY(-2px), 200ms); secondary link `New to euchre? ↓` -> anchor `#how-to-play` (12px, underlined, offset 5px).
- Note: `Play against computer opponents · No account needed` (10px `#a8bcb0`).
Right column: hero image `static/art/euchre-hero.webp` (1200x1000), absolutely centered at width 135%, masked with `radial-gradient(ellipse 64% 59% at 50% 50%, black 48%, transparent 95%)`; alt text: `Classic playing cards and a dealer button on a green felt table, with both red jacks at the front`. Caption (8px, ls .22em, `#a1b9a6`) `GOOD COMPANY. BETTER CARDS.` (hidden <=760). Lime circular "bower stamp" 100x100px rotated 14deg at right/bottom with ring `0 0 0 6px #153f36`: lines `THE RIGHT` / `J♥` (serif 36px, heart 25px) / `KIND OF LUCK` (7px/700 ls .11em).
Bottom strip (absolute, 26px from bottom, top border `#c6d7b92b`): `A CLASSIC, DEALT DIFFERENTLY.` | `♠ ♥ ♣ ♦` (18px, ls 7px, hearts/diamonds `#a8b893`) | `FIRST TO TEN. MAKE IT COUNT.` (last hidden <=760). 8px ls .16em `#9db3a4`.

### 3.2 Introduction (max 1120, 2 columns gap 100px, padding 85px 24px)
- Eyebrow `SMALL DECK. BIG PERSONALITY.`; H2 `All the feeling.` / `None of the shuffling.`
- Paragraph 1: `A proper game of partnership euchre, made for the browser. Read the table, trust your partner, and know when to go it alone.` (Native port: replace "made for the browser" with "made for the desktop"; keep tone.)
- Paragraph 2 (muted `#72766a`, 13px): `Settle into a 3D table with three computer players. Your next good hand is one deal away.`

### 3.3 How to play (id `how-to-play`)
Eyebrow `PULL UP A CHAIR`; H2 `Easy to learn.` / `<em>Hard to put down.</em>`. Three cards (bg `#e9ecdf`, radius 10, padding 28, 3-column gap 16 -> 1 column <=760): number `01 /`, `02 /`, `03 /` (10px `#6b7664`), big suit glyph top-right 36px (`♣`, `♦` in `#b44835`, `♠`).
1. `Find your partner.` - `Four players. Two teams. The player across the table is on your side. You each get five cards.`
2. `Make the call.` - `Choose a trump suit or pass. The team that calls trump needs to take at least three of the five tricks.`
3. `Play your hand.` - `Follow suit if you can. The highest card wins the trick, and trump beats everything else. First team to ten wins.`
Disclosure `The jacks have a little secret +` (the `+` rotates 45deg when open): two columns:
- `The jack of trump is the **right bower**, the strongest card. The other jack of the same color is the **left bower**, the second strongest. Both belong to the trump suit.`
- `Take three or four tricks after calling trump for one point, or all five for two. Get stopped and the other team earns two. Feeling bold? Go alone and take all five for four points.`

### 3.4 Footer
`euchre♣` wordmark (serif 600 32px) | `A good hand is better shared.` (11px, hidden <=760) | link `See you at the table ↗` -> `/new`.

Responsive: <=1050 tighter paddings; <=760 single column hero, card-scene height 310px, caption and last bottom-strip item hidden, 1-col cards; <=380 smaller padding. `prefers-reduced-motion`: no button transition.

---------------------------------------------------------------------------------------------------

## 4. Screen: New game `/new`

Title `Choose your table · Euchre`. Panel: max-width 640, centered, padding 48x24, bg `#f4f3e9`, radius 16, ink `#233e32`.
Content (verbatim):
- Link `← Your saved games` -> `/games`
- H1 `Your table. Your rules.` (serif 500 38px)
- `Traditional 24-card euchre. You and your partner play to 10.` (18px/1.5)
- Checkbox 1 (24px box, accent `#285641`, 20px label): `Stick the dealer` / small `After three passes in round two, the dealer must name trump.`
- Checkbox 2: `Require a natural trump to call` / small `The left bower alone does not qualify. The dealer may count the up-card when picking it up.`
- Conditional note when BOTH checked: `The stuck dealer may call without a natural trump, so the hand can always proceed.`
- Submit button (full width, min-height 56, bg `#285641`, white text, radius 8, 20px/600): `Deal me in →`. Submits GET `/play?stick=<bool>&natural=<bool>` (hidden inputs `stick`, `natural` hold `"true"`/`"false"`). Both default OFF.
- Footer line: `Take your time. Your turn waits for you, and your table is saved in this browser.` (native port: replace "in this browser" with "on this device/server" - keep the sentence otherwise).
Game config sent to backend: `cfg = { gameTo: 10, stickTheDealer, requireNaturalTrump }`.

---------------------------------------------------------------------------------------------------

## 5. Screen: The table `/play/[gameId]`

The most complex screen. `src/routes/play/[gameId]/+page.svelte` (+ `TableStage`, `CardRack`, `BidPanel`, `ScoreBoard`, `ScoreCards`, `RulesPanel`, `HandReview`, `Hint`, `GameFeedback`, `GameAnnouncer`, `Walkthrough`, `SoundToggle`). The route key is `gameId`; changing it remounts everything (`{#key page.params.gameId}` in `play/+layout.svelte`).

### 5.1 Overall frame (desktop >= 1200 wide)

```
+-- .table-page (100dvh, flex row, overflow hidden, bg #f4f3e9) -----------------------------------+
| .game-content (flex 1, column)                                         | .settings-rail (0 -> 340)|
|  +-- header.game-header (min-height 64, padding 8 24, cream) -------+  |  (side panel, see 5.9)  |
|  |  ScoreBoard (flex 1)              |  RulesPanel btn + Settings btn |  |                         |
|  +--------------------------------------------------------------------+  |                         |
|  +-- .table-layout (flex 1, grid rows: 1fr  |  clamp(280px,42dvh,420px)) |                         |
|  |   bg #17452f, color #fff8e8, padding 8 24 (max(8, safe-bottom)), gap 8 |                         |
|  |   [FeltLayer: decorative canvas behind everything, z 0]               |                         |
|  |   [notice / error banners absolute top 8px, left/right 24px, z 6]     |                         |
|  |   row 1: TableStage (the felt)                                        |                         |
|  |   row 2: .hand-area (heading + instruction + CardRack + BidPanel)     |                         |
|  +--------------------------------------------------------------------+  |                         |
+---------------------------------------------------------------------------------------------------+
```
- `.table-layout` is one continuous felt-green surface under both TableStage and hand area (`.felt` has transparent bg, no border/shadow). A thin top border `1px #e2dbbd26` separates the hand area.
- `.hand-area` on >=701px wide and >=501px tall: max-width 1100, centered, padding-inline 16; its card rack max-width 980 (page) / 850 (rack).
- `.table-footer` (`A little friendly competition.` / `Select a card. Then press Play.`) exists but is `display: none` in the final CSS cascade. Do not render.
- Header: `ScoreBoard` left, `.table-tools` right (gap 10): `How to play` button (RulesPanel trigger) and `Settings ⌄` button. Header container query: `<=850px` content width wraps and tools row goes full width, right-aligned; `<=700px` viewport: padding 6x12, tools full width space-between, buttons min-height 44.
- Settings button: min-height 52, padding 10x16, border `1px #cad3c1`, radius 8, 17px, transparent. It has `aria-controls="table-settings"` `aria-expanded`.

### 5.2 Vertical budget by form factor
| Condition | Table layout rows | Notes |
|---|---|---|
| default (desktop) | `minmax(0,1fr)` / `clamp(280px, 42dvh, 420px)` | |
| `701+ wide, <=850 tall` | same rows; header min-height 70, felt paddings shrink, `.table-footer` off, hand heading h1 26px inline with eyebrow | played cards 108px wide |
| `<=700 wide` (phone) | `minmax(0,.92fr)` / `minmax(0,1.08fr)`, gap 6, padding `6 10 max(8, safe-bottom)` | hand h1 hidden; bidding hides hand heading |
| `<=700 wide, <=750 tall, portrait` | `1fr / 230px` (bidding: `250px`) | hand rack becomes ONE row of `--card-count` columns, gap 8 |
| `<=360 wide, <=650 tall, portrait` | `1fr / 190px` (bidding `220px`) | |
| landscape and height <=500 | `1fr / 125px`, padding `4 10`, header min-height 60 | hand area becomes 2 columns `180px | 1fr`: left = Play button + instruction (+bid controls when bidding), right = rack across both rows; card labels hidden |

### 5.3 Header: ScoreBoard + ScoreCards (`ScoreBoard.svelte`, `ScoreCards.svelte`)
Layout (left to right): brand link `euchre♣` (aria-label `Euchre home`, -> `/`; serif 600 38px; hidden <=700px wide or when container <=700) | divider line `1px #d3d9c9` | team A block: ScoreCards + label `Your team` + big number | `—` divider (`#78856d`) | big number + label `Other team` + ScoreCards | `First to 10` (15px `#596952`; hidden <=1050px viewport or container <=1100).
- `team = view.you % 2`; left block shows `view.score[team]`, right block `view.score[1 - team]`.
- Container `aria-label="Match score"`.
- **ScoreCards**: inline SVG viewBox `0 0 78 50`, shown 72x46px (42x28 at <=700). Two playing-card rects (35x48, rx 4, fill `#fffdf6`, stroke `#99a58a`), second offset x=40. Left card "6" with 6 hearts (fill `#aa342e`), right card "4" with 4 spades (fill `#243e32`); rank numerals 9px at (4,10). Pip positions: six = (10,13)(24,13)(10,24)(24,24)(10,35)(24,35); four = (10,13)(24,13)(10,35)(24,35); glyph size 13, drawn at y+4, text-anchor middle. Pip i is lit (opacity 1) when `i < min(score,6)` for the six card and `i < max(0, score-6)` for the four card; unlit opacity 0.08. `role="img"` `aria-label="{score} points, counted with a six and a four"`. (Cribbage-style score card: 0..10 maps 6 + 4 = 10 pips.)

### 5.4 TableStage (the felt) - the core component

`aria-label="The table"` section. Content grid rows (default): `auto auto minmax(0,1fr) auto` (status / seats-layer placeholder / played cards or bid area / trick result). On >=701 wide and >=501 tall the seats layer is absolutely positioned (see 5.4.2) and the grid is `auto 0 minmax(0,1fr) auto`, padding `8px 20px 0`.

#### 5.4.1 Status row (`.table-status`)
Left `.trump` block:
- Eyebrow `Hand {handNo + 1}` (14px, ls .055em, `#d9e6cb`).
- Strong: if `trump` set: `{glyph} {SuitName} are trump` (glyph in `#e0cf97`, `aria-hidden`); else `Choosing trump`. Glyphs `H♥ D♦ S♠ C♣`. Serif 500 26px.
- If `trick.ledSuit`: led marker pill `↗ {glyph} Led` (`aria-label="{SuitName} led"`; 18px, padding 4x10, radius 20, bg `#ffffff0c`; the word `Led` is 13px small). On phones the marker is replaced by the `mobile-led` text `Led {glyph}` in the trick-score row (see below).
- `.trump` max-width 42% on desktop.
- If `aloneSeat !== null`: `★ {seatName(aloneSeat)} alone` (bold, `#f2d887`), e.g. `★ Your partner alone`, `★ You alone`.
Right `.trick-score` block (right aligned):
- Caption `Tricks this hand` (hidden <=700; at <=360 shows `Tricks` short caption). On <=700 phones shows `mobile-led`: `Led {glyph}` if ledSuit else `Tricks`.
- Strong: `Your team <b>{tricksWon[team]}</b> · Other team <b>{tricksWon[1-team]}</b>` (18px; b = 25px tabular).

#### 5.4.2 Seats (four `.seat` blocks, DOM order = relative offsets 1,2,3,0, i.e. left opp, partner, right opp, you)
Data attributes `data-seat-anchor={seat}` used as animation anchors (centre of the block is where plays and pickup flights originate/land).
Per seat content (centered text):
- Row 1 `.seat-name`: small circular initial marker (SVG 28x28, circle r15 `currentColor` at .18 opacity + first letter of display name, 16px/600; colors: default `#b9d2c1`, partner `#e4cf91`, active seat `#edd79a`; hidden <=700px), then the display name (ellipsized, max-width 130px; `title` = `{name}{ — blurb}`), then badges:
  - maker badge (round cream chip with trump glyph) if `seat === makerSeat && trump`, `title/aria-label="Named trump"`.
  - lead arrow `↗` (`title/aria-label="Leads this trick"`) if `seat === leadSeat`, where `leadSeat = trick.plays[0]?.seat ?? (phase === 'trick_play' ? turnSeat : null)`.
  - dealer chip `D` (round cream chip 18px desktop (24 base, 20 <=700, 16 tiny); screen-reader text `Dealer` completing "D"+"ealer"; `title="Dealer"`). On <=700px it is absolutely positioned at top -5px right 0 of the seat.
  - Short names (<=360px wide): `player.name` or `['You','Left','Partner','Right'][rel]`.
- Row 2 `.seat-detail` (min-height 18-22px): `{Role} · ` prefix (role in 75% opacity: `You | Left | Partner | Right`, only when `players` metadata exists) then one of, in priority:
  1. `Sitting out · partner alone` if `sittingSeat === seat`
  2. `Your turn` if `turnSeat === seat && seat === you`; `Thinking…` if `turnSeat === seat` and not you
  3. `Passed` if bidding phases and that seat's latest bid in the current round is `pass` (`bids` for round 1; `bids.slice(4)` for `bid_round_2`; latest entry per seat via reverse find)
  4. `{n} card(s)` using `handCounts[seat]` (`1 card`, `0 cards`, `5 cards`)
- Taken-stack: a 24x16 striped card-back glyph at top -9px right 5px (desktop: right 4 bottom -18), opacity 0 normally and 0.8 when `trickLog.some(t => t.winnerSeat === seat)`; `data-stack={seat}`; `aria-hidden`. It is the target of the trick-gather animation.
Active seat styling: when `turnSeat === seat`: seat bottom border `#e6d08c` and bg `#ffffff0a` (radius `7px 7px 0 0`) on the mobile/stacked layout; on desktop absolute layout the name gets `#ffe5a4`.
Display names: `playerName(view, seat)`: `You` if you; else `players[seat].name`; else seatName fallback. `seatName(seat, you)` = `['You','Left opponent','Your partner','Right opponent'][rel]`.

Desktop (>=701w and >=501h) seat positions inside `.seats` (`position:absolute; inset: 60px 14px 50px; pointer-events: none`), each `.seat` absolute, `width:180px`, `transform: translateX(-50%)`:
| nth-child | who | left | top/bottom |
|---|---|---|---|
| 1 | Left opponent | 13% | top 38% |
| 2 | Partner | 50% | top -36px |
| 3 | Right opponent | 87% | top 38% |
| 4 | You | 50% (width 120px) | bottom -12px; `visibility: hidden` while the trick/played-cards layer is shown (bidding phases show it) |
Stacked/mobile layout (<=700w or <=500h): seats are a 4-column grid (gap 10-24) ABOVE the played-cards row, max-width 850.

#### 5.4.3 Played-cards area (non-bidding phases) - the trick
Shown when `!bidding`, where `bidding = phase in {lobby, cutting, deal, bid_round_1, bid_round_2, dealer_discard}`. Four `.played-slot`s in seat order [left opp, partner, right opp, you].
Desktop absolute slots inside the played-cards region (`position:relative`, width 22% / height 80% of region, `transform: translate(-50%,-50%) rotate(r)`):
| slot | who | left | top | rotate |
|---|---|---|---|---|
| 1 | Left opponent | 28% | 50% | -7deg |
| 2 | Partner | 43% | 35% | 3deg |
| 3 | Right opponent | 72% | 50% | 7deg |
| 4 | You | 57% | 68% | -3deg |
(The tilt and stagger give the trick a hand-laid look; the winner stack for each seat lives on the seat's `taken-stack`.)
Stacked layout: 4-column row (gap 10-24), each slot sized by container query so card width = `min(100cqw, (100cqh-8px)*5/7)`, plus a caption under each card (`.played-owner`, 11-12px/650, `#f3ead4`: `You`/`Partner`/`Left`/`Right`) shown at <=700w or <=500h. Max card width 145px (108px on 701+w and <=850h).
Per slot:
- Card image `/art/cards/{card}.png`, aspect 5:7, radius 7, shadow as in 1.4, alt text `{seatName} played {Rank of suit}` + ` — winning card` when `winner === seat`.
- Winning card highlight: outline `3px solid #eed591`, offset 5px, applied once `trick.winnerSeat === seat`.
- Empty slot: invisible placeholder, text `Sitting out` if `sittingSeat === seat` (15px `#d9e6cb`), else a blank space.
- After the collect animation completes (`collectedKey === trickKey`) the played cards are `visibility: hidden` (they have flown to the winner's stack).

#### 5.4.4 Bidding area (`.bid-table`, replaces played cards while `bidding`)
Layout: desktop flex row (column on 701w+... final cascade: row, gap 28, padding 14 0) centered, container-sized; mobile row gap 16.
Case A - up-card visible: `view.upCard && !view.upCardTurnedDown && phase !== 'dealer_discard'`:
- Up-card image `/art/cards/{upCard}.png` (alt `Up card: {Rank of suit}`), height `min(100cqh,190px)`, aspect 5:7 (105x147 / 100x140 on small), with the tilted/stacked card shadow transform (see 1.4).
- Text block: eyebrow `The up card` (hidden <=700); h2 `{Rank of suit}` (serif 500, `clamp(24px,3vw,32px)`, 23px desktop, 22/18px mobile); paragraph `{seatName(dealerSeat)} {pick|picks} it up if its suit becomes trump.` where verb is `pick` when dealer is you, else `picks` (hidden on desktop/mobile final cascade where `display:none` for `.bid-table p`; still part of the data model - Rune may show it when space allows, 17px `#e0e9d7`, max-width 300).
- Pickup animation: see 6.3.
Case B - otherwise: a "deck" tile (137x192 card-back placeholder: border `6px #f4f0df`, outline `1px #ffffff70`, `repeating-linear-gradient(45deg,#3a6448 0 2px,#2a4f3b 2px 5px)`, centered `♣` 48px `#f2e8cd`, same tilt+shadow) and text:
- eyebrow `A seat at the table`
- h2: `bid_round_2` -> `{seatName(dealerSeat)} turned down {cardName(upCard) | 'the up card'}.`; `dealer_discard` -> `{seatName(dealerSeat)} picked up {cardName(upCard) | 'the up card'}.`; otherwise `A fresh hand awaits.`
- p: if `bid_round_2 && turnedDownSuit`: `{SuitName} were turned down. Choose another suit.` else `You and your partner play together. First team to 10 wins.`
(`seatName` for dealer = you reads "You turned down..." / "Left opponent picked up ...".)

#### 5.4.5 Trick result / status line (`.trick-result`, bottom row, min-height 40-62, centered, 16px `#e0e9d7`; `.won` state 18px `#ffecb7`)
Priority:
1. `trick.winnerSeat !== null`: strong `{seatName(winner)} {win|wins} the trick.` followed by next-lead text:
   - trick index < 4: if `reviewTricks`: `Press Continue before the next lead.` else `{seatName(winner)} {lead|leads} next.`
   - index == 4: `That’s the last trick of this hand.`
   (`win`/`lead` for "You", else `wins`/`leads`.) `.next-lead` is hidden at <=360w.
   Plus the **Continue** button (only when `reviewTricks && phase === 'trick_resolve'`): label `Continue →` (`Collecting… →` while the gather animation runs), disabled unless `awaitingTrickReview && !collecting && connected`; cream button `#f5e8c3` on `#233e32`, radius 7, min-height 48 (42-44 smaller), 650 weight; disabled opacity .65, cursor wait.
2. else if a table-talk bubble is active (`talk`): `<strong>{playerName(talk.seat)}</strong> “{text}”` (see 9). 
3. else if `aloneSeat !== null`: `{seatName(aloneSeat)} {are|is} going alone.` (`are` for You).
4. else: bidding -> `Take your time. There’s no rush.`; playing -> `Trick {index+1} of 5 · The winner leads the next trick.`
On phones, during bidding the `.trick-result` line is hidden.

### 5.5 Hand area (`.hand-area`)
Grid rows `auto auto minmax(0,1fr) auto`: heading / instruction / rack / bid controls. `aria-label="Your hand and controls"`. Adds class `bidding` when `phase` starts with `bid` or is `cutting`.

**Heading row** (`.hand-heading`, flex, gap 12):
- Eyebrow `Your hand` (hidden >=701w&>=501h; 16-18px `#e9e9d5` otherwise).
- H1 text (first match wins): `Sending your move…` if submitting; `A good game, well played.` if `status !== 'active'`; `Review the trick.` if `phase === 'trick_resolve' && reviewTricks`; `Your turn.` if `turnSeat === you && phase !== 'trick_resolve'`; else `Make yourself at home.`. 28px serif (25-26 smaller; hidden <=700w).
- Right side action:
  - when `status === 'active'` and phase in `trick_play | trick_resolve | dealer_discard`: confirm button `{Playing… | Discard | Play} {cardName(selected) | 'selected card'} →`. Label word: `Discard` in `dealer_discard`, else `Play`; while `store.submitting` shows `Playing…`. Example: `Play Jack of spades →`, no selection: `Play selected card →`. Disabled unless a selected card has a matching legal move (`play:` or `discard:`), not submitting and connected. Style on felt: bg `#f5e8c3`, text `#233e32`, border `#ead8aa`, radius 8, min-height 48 (56 base), min-width 230, padding 8x18, 17px/600, gap 18 between label and arrow.
  - when `status !== 'active'`: link-styled button `Play again →` -> `/new`.
**Instruction line** (`role="status"`, 17px, min-height 0): `Waiting for the table to confirm your move…` if submitting; else the illegal-card message if one is active (see 5.7); else `turnInstruction(view)` (section 12 has all strings).
**Inline error** (`role="alert"`) if `store.error`: absolute at bottom 12px, `.err` style (cream-orange box).
**CardRack** (section 5.6).
**Bid controls** (`BidPanel`, section 5.8).

### 5.6 Card rack (`CardRack.svelte`)
- `aria-label="Your cards"`, CSS var `--card-count = slots.cards.length` (max 5 normally; 6 for the dealer after pickup).
- **Slot retention** (`reconcileHandSlots`, `table-presentation.ts`): a hand keeps a fixed ordered list of slots per `handNo`. New hand number -> reset to `view.hand`. If new cards appear in `view.hand` (e.g. dealer pickup) they are APPENDED to the slot list; cards that leave the hand leave an EMPTY hidden slot so remaining cards never shift (hit targets never move). Keep this exactly: it is a deliberate anti-misclick behavior.
- Order shown is the order the server sends in `view.hand` (no client sorting, no grouping by suit).
- Layout: CSS grid `repeat(--card-count, minmax(0,1fr))`, gap 18px (8-14 smaller; phones <=700 portrait: 3 columns x 2 rows gap `6px 14px` unless the short-portrait rule forces one row), width `min(100%, 850px)` centered. Each cell is a size container; artwork width = `min(100cqw, (100cqh - 46px) * 5/7)` (46px reserved for the label; 12px on phones where labels hide).
- Each held card is ONE button (`type=button`, `data-card-id`). Its artwork is a non-interactive child. Contents: card image (`/art/cards/{card}.png`, `alt=""`, `draggable=false`), then a label block: line 1 `{cardName}` (15px/600), line 2 small (13px, min-height 17): `Selected ✓` if selected; `Must follow suit` if there are card moves but this card is not legal (`legal = cardMoves.has(card) && !disabled`) - styled `#735548`; otherwise empty. Labels hidden <=700w and at landscape <=500h.
- **States**: 
  - default: hover (only on `hover: hover` devices) and not selected/unavailable: artwork `translateY(-4px)`.
  - `selected`: artwork `perspective(1000px) translateY(-8px) rotateX(2deg)`, outline `3px solid #295744` (on felt `#f5d98e`), offset 3px, bigger shadow; label small bold. Transition 180ms ease on transform+box-shadow. Reduced motion: no transition and no selected transform.
  - `unavailable` (not legal): cursor `help`; stays clickable to show the reason.
  - `empty slot`: hidden placeholder (visibility hidden).
- `aria-pressed={selected}`, `aria-disabled={!legal}`, `aria-label`: `Select {Rank of suit}. {legal ? 'Press to select, then use Play. Or double-click to play.' : unavailableReason(card)}`.
- `unavailableReason(card)` (rack, a11y label): `Please wait until the table is ready.` (disabled) -> `This match is finished.` (status != active) -> `Press Continue to finish reviewing this trick.` (trick_resolve && reviewTricks) -> `Wait for your turn.` (not your turn) -> `Finish bidding before selecting a card.` (phase neither trick_play nor dealer_discard) -> `whyIllegal(...)` or `This card is not available right now.`
- **Click**: if rack `disabled` (submitting or not connected) ignore. If the card has no legal move -> call `onIllegal(card)` (shows the reason message, section 5.7). Else `onSelect(card)` (store `{handNo, card}`; selection survives only while `handNo` matches and the card is still in hand).
- **Double-click**: if not disabled and a legal move exists for that card -> submit it immediately (`play:<card>` or `discard:<card>`). (Single-click selection fires first.)
- **Keyboard** (`onkeydown` on each card button): `ArrowLeft`/`ArrowRight` move focus to previous/next card button (wrapping), `Home` first, `End` last (prevents default). Native buttons give Enter/Space = click = select. After selecting, tab/focus to the Play button and press Enter/Space to play.
- **Deal animation** (per newly-added card, only when its `handNo:card` key was not present at first render; skipped under reduced motion and during `dealer_discard`): `transform translateY(-24px) rotateY(18deg) scale(.92)`, opacity 0 -> `none`, opacity 1; duration 580ms; delay `index * 75ms`; easing `cubic-bezier(.22,.7,.25,1)`; fill backwards. Sound `playDeal()` fires once when index 0 animates. The very first snapshot after connect/reconnect never animates (dealtKeys pre-seeded; keep at most 30 keys).

### 5.7 Illegal-card feedback
Tapping an unavailable card sets an `illegalMessage` for 3200 ms that replaces the instruction line. Message (`illegalReason(view, card)` in the page): `This game is finished.` (status != active) -> `Press Continue to finish reviewing this trick before playing.` (trick_resolve && reviewTricks) -> `It's not your turn yet.` (turnSeat != you; note straight apostrophe) -> `It's not your turn to discard.` (phase dealer_discard but not your turn... only reachable when not turn) -> `Finish bidding before you play a card.` (phase != trick_play) -> `whyIllegal(hand, card, ledSuit, trump)` or `That card can't be played right now.`
`whyIllegal` strings (`legal.ts:114`):
- not in hand: `That card isn't in your hand.`
- left bower must-follow: `{SuitName} were led. Your {Rank of suit} is a {suit noun} right now — you have to follow.` (noun singular lowercase: spade/heart/diamond/club)
- otherwise: `{SuitName} were led. You still hold the {cardnamelower}[, the ... and the ...] — you have to follow suit.` (join: `a`, `a and b`, `a, b and c`)

### 5.8 Bid panel (`BidPanel.svelte`)
Rendered only if the current phase is `cutting | bid_round_1 | bid_round_2`, `turnSeat === you`, and there are non-card legal moves. Container `aria-label="Bidding"`, max-width 850, one flex row (gap 16): buttons row (flex 1, nowrap, gap 10) then the go-alone checkbox on the right (order 2).
- **Go alone** checkbox (`Go alone`, 22px box, accent `#285641`, 17px label, min-height 48) appears only when at least one legal move id contains `+alone`. State is per decision (`${handNo}:${phase}`): checking it swaps each visible button's move to its `+alone` variant if one exists (`orderUp` -> `orderUp+alone`, `call:H` -> `call:H+alone`); `pass` and cut moves have no alone variant and stay as-is. The checkbox resets automatically when phase or hand changes.
- **Buttons** (one per non-alone legal move, `type=button`, min-height 56 (46 phones), flex 1, padding 10x16, radius 8, 19px/600 (15-17 phones)):
  - `pass` and `cut:no`: secondary style (bg `#fffdf6`, border `#85957b`, text `#334d3e`, hover `#e9eddf`).
  - others: primary style (on felt: bg `#f5e8c3`, text `#233e32`, border `#ead8aa`; elsewhere `#285641` with `#fffdf4` text, hover `#1b4531`).
  - Labels: `pass` -> `Pass`; `orderUp` -> `Make {suit lowercase} trump` using the up-card suit (e.g. `Make hearts trump`; fallback `Make it trump`); `call` -> `{SuitName}` (`Spades`, `Hearts`, `Diamonds`, `Clubs`); `cut` -> `Cut the deck` (cut:yes) / `Deal the cards` (cut:no); other -> the server `label`.
  - Disabled while `store.submitting` or not connected (opacity .55).
  - Stuck dealer (`stickTheDealer` and round 2, dealer, 3 passes): the server omits `pass` from `legal`, so no Pass button appears (omit, do not disable).
- On phones (<=700w): bids row wraps to full width, go-alone row right-aligned above/below.

### 5.9 Table settings side panel (`aside#table-settings`, "Table settings")
- Toggle via the `Settings ⌄` header button. Closed by: header button, the `×` button (`aria-label="Close settings"`, 44x44), `Escape`, or backdrop click (compact mode).
- **Wide mode (window >= 1200px)**: inline rail. Width animates `0 -> 340px` (320ms `cubic-bezier(.22,1,.36,1)`), border-left `1px #d4dccb`, bg `#fffdf6`; panel content translates `32px -> 0` and fades `0 -> 1` (180ms ease for opacity). Game stays interactive.
- **Compact mode (< 1200px)**: overlay. Rail is absolute right, width `min(340px,100%)`, slides `translateX(100%) -> 0` (320ms), shadow `-16px 0 48px #10251d30`; a backdrop button (`aria-label="Close settings panel"`, bg `#10251d55`, opacity 0->1 200ms) covers the game; `.game-content` shifts `translateX(-24px)` and becomes inert; role `dialog`, `aria-modal=true`; focus moves to the first button on open, Tab/Shift+Tab are trapped inside; on close focus returns to the Settings button. Wide mode role `complementary`.
- Reduced motion: all these transitions removed.
- Panel width 340, padding 24 (20 + safe-area compact), scrollable (`overscroll-behavior: contain`), 17px/1.5 `#233e32`.
Content in order:
1. Heading: eyebrow `Your table` (13px `#69765f`), h2 `Table settings` (serif 500 28px), close `×`.
2. Row `Table sounds` + `SoundToggle` (see 15.1).
3. Checkbox `Wait for me after each trick` + small `Review the cards, then press Continue.` (24px accent `#285641`; borders top/bottom `#dce1d4`). Checked = `view.reviewTricks ?? false`. Toggling sends `reviewTrick({action:'set', enabled})`; disabled while busy or disconnected. Shared with the server state (persists across reload).
4. **HandReview** launcher button `↶ Review hands` (section 11).
5. **Hint** toggle `Hints` (section 13).
6. If `view.players?.length`: section `Players at this table` (aria-label) with h3 `Around the table`; per player: 36px round avatar (bg `#285641`, text `#fff5d9`, first initial serif 500 20px), name (17px/700) with sub-line `{seatName} · {style}` (13px `#58695b`), blurb below. Then link `Choose player personalities →` -> `/settings`, then small `Changes apply to your next game.`
7. Links (top-bordered rows, 14px padding): `Start a new game` (`/new`), `Back to home` (`/`), `Your saved games` (`/games`).
8. Disclosure `Table talk` (`<details>`): last <=6 chat lines `<strong>{speaker}</strong> {text}` where speaker is `playerName` for seat 0..3 else `Table`; empty state `The conversation starts with the first call.`

### 5.10 Rules panel (`RulesPanel.svelte`)
Trigger: header button `How to play` (aria-label `How to play`, aria-haspopup dialog; min 120x52; bg `#fffdf5`, border `rgba(232,194,122,.55)`, 17.6px/700). Opens a modal dialog `aria-labelledby="rules-title"`, max-width `min(34rem,92vw)`, max-height `min(85vh,44rem)`, sheet bg `#f5f3e9`, border `rgba(232,194,122,.35)`, radius .9rem, shadow `0 12px 40px rgba(0,0,0,.55)`, backdrop `rgba(6,5,3,.68)`; closes on `×` (`aria-label="Close rules"`), Escape or backdrop click. Content:
- H2 `How to play`.
- If `view.rules`: `This table: stick the dealer {on|off} · Natural trump to call {required (left bower alone does not qualify)|not required}.`
- Context card (only while `status==='active'`) titled by phase:
  - `cutting`: **Right now: cutting for the deal** - `The player to the dealer’s right may cut (“bump”) or say “run ’em” to leave the deck as shuffled.`
  - `bid_round_1`: **Right now: the up-card is showing** - `{You may|Each player in turn may} "order it up" — {UpSuit} become trump, the dealer picks the card into hand and buries one of their own — or pass. Ordering it up alone leaves your partner out of the hand for a bigger score if you win it all.` (UpSuit = SuitName of upCard, or `that suit`).
  - `dealer_discard`: **Right now: the dealer discards** - `The dealer has 6 cards after picking up the up-card. One goes face down, unseen, for the rest of the hand — back down to 5.`
  - `bid_round_2`: **Right now: round two** - `The turned-down suit ({SuitName|that suit}) may not be named again. Each player may name any of the other three suits as trump, or pass. If it comes all the way back around to the dealer with everyone else passed, {rules.stickTheDealer !== false ? 'the dealer must name a suit — no more passing.' : 'the dealer may pass too; four passes mean a fresh deal.'}`
  - `trick_play`: **Right now: playing a trick** - led null: `You're on lead — play anything. Whatever suit you lead, everyone else must follow if they can.` else `{SuitName} was led. Follow it if you're holding one.`; when trump known append ` {TrumpName} are trump — the {left-bower name lowercase} counts as {trump lowercase} right now, not its printed suit.` (left bower = jack of same-colour suit: H<->D, S<->C).
  - `trick_resolve`: **Right now: the trick just finished** - `Watch the felt — the winner leads next.`
  - `hand_score`: **Right now: scoring the hand** - `The next hand deals in a moment.`
  - `game_over`: **Game over** - `First to 10 points takes the match.`
- Search box (`type=search`, placeholder `Search the rules…`, sr-only label `Search the rules`); filters sections by title/body substring (case-insensitive); with a query all matches are opened; no match: `No section matches "{query}".`
- Section accordion (title / body):
  1. `The deck` - `24 cards: nine, ten, jack, queen, king and ace, in all four suits. Nothing lower than a nine is used.`
  2. `Trump and the bowers` - `The jack of the trump suit — the right bower — is the single best card in the game. The jack of the OTHER suit that's the same color becomes trump too, and outranks every other trump except the right bower — that's the left bower. ` + if trump set: `Right now {TrumpName} are trump, so the {left bower lowercase name} counts as {trump lowercase}, not its printed suit, for the rest of this hand.` else (example uses Hearts): `For example: if Hearts were trump, the jack of diamonds would count as hearts, not its printed suit, for the rest of that hand.`
  3. `Following suit` - `If you can play a card of the suit that was led, you must — including the left bower, which counts as trump, not its printed suit, once trump is set. If you're void in the led suit, play anything.`
  4. `Bidding, round one` - `One card is turned face up. Starting left of the dealer, each player may "order it up" — making its suit trump for the hand — or pass. If the dealer's side orders it up, the dealer picks the card into hand and discards one card face down.`
  5. `Bidding, round two` - `If everyone passes, the up-card turns face down and can't be named. Starting left of the dealer again, each player may name any of the other three suits as trump, or pass. If it reaches the dealer with all three others having passed, the dealer must name a suit if stick the dealer is enabled; otherwise, four passes mean a fresh deal.`
  6. `Going alone` - `Whoever calls trump may go alone: their partner sits out entirely and they play all five tricks solo against both opponents. Win all 5 alone and it's worth 4 points instead of 2.`
  7. `Scoring` - `3 or 4 tricks: 1 point. All 5 (a march): 2 points. Alone and all 5: 4 points. The calling team wins fewer than 3 tricks — euchred — and the other team scores 2 instead. First to 10 wins the game.`
  8. `What you'll hear at the table` - `"Order it up." / "I assist." (dealer's partner) / "I take it." or "Turn it down." (dealer) / "Next." or "crossing the creek" (round two) / "You're stuck" (dealer forced to call) / "March!" / "Euchre!"`
- Bottom button (dashed border): `New to euchre? Take the quick tour again` -> closes and reopens the Walkthrough (section 14.3).

---------------------------------------------------------------------------------------------------

## 6. Animations and timing (authoritative)

### 6.1 Shared tempo contract (`src/lib/protocol/pacing.ts`, imported by both server and UI)
```ts
TABLE_MOTION = { play: 700, read: 1500, collect: 780, stagger: 45 }   // ms
TRICK_PRESENTATION_MS = play + read + collect + 3*stagger + 185 = 700+1500+780+135+185 = 3300 ms
```
`TRICK_PRESENTATION_MS` is the SERVER hold time: after the 4th card of a trick the actor stays in `trick_resolve` (winnerSeat known) for 3300 ms before advancing (unless `reviewTricks` is on, in which case it waits for `reviewTrick({action:'continue', turnId})`). The Rust backend MUST use the same constant; the client choreography is sized to fit inside it. (Hand-score hold and AI think delays are backend lane: hand-score hold 2500 ms; trick hold 3800 ms when the trick seals a euchre; AI think floors cut 700 / bid1 1900 / bid2 1900 / discard 1100 / play 1000 ms plus a deterministic 0-399 ms jitter; AI hard cap 4000 ms. See 03 section 9 and 04 section 3.) <!-- critic: replaced "doc 04 section 10 lists AI floors: 250-450 / 900-1400 / 900-1800 ms"; 04 section 10 is Notation and those numbers are not in the code. Values from euchre-table/tempo.ts:54-82 -->

### 6.2 Card play flight (`arrive` attachment in TableStage)
Triggered when a `played-card` image first mounts for key `${handNo}:${trick.index}:${card}` (dedupe set capped at 40 keys; keys present at first render are pre-seen, so a reconnect does not animate).
- Waits for image decode; hidden (`visibility:hidden`) until decoded, then animates.
- Skipped entirely under reduced motion.
- Origin: if the seat is YOU and a `CardOrigin` (the hand card's on-screen rect captured at submit, `performance.now()` within 2500 ms, same card) exists -> start at that rect: `dx = origin.left - rect.left`, `dy = origin.top - rect.top`, `scale = origin.width / rect.width`. Otherwise start at the seat anchor centre (`data-seat-anchor`): `dx = anchorCenterX - (rect.left + rect.width/2)`, same for y; fallback `dy = +130` (you) or `-95` (others) if no anchor; `scale = 0.75`.
- Keyframes (duration `700ms`, easing `cubic-bezier(.22,.7,.25,1)`):
  - 0%: `translate(dx,dy) scale(s) rotate(R)` opacity 0.4 where `R = 0deg` for you, `-6deg` for others
  - 82%: `translate(0,-8px) scale(1.025) rotate(1deg)` opacity 1
  - 100%: `none`, opacity 1
- On finish: `playPlace()` sound.
In the native port the opponent card starts at the opponent's seat; your card starts at the hand slot you played from.

### 6.3 Up-card pickup (dealer takes the up-card)
Svelte `out:pickup|global` transition on the up-card image when it leaves (phase moves to `dealer_discard` after an `orderUp`). Plays only if `view.handNo` still equals the card's hand and some bid is `orderUp`, the dealer anchor exists, and motion is not reduced (a reconnect directly into the discard phase never replays it). Duration `850ms`; progress `p = 1 - t` eased `e = p*p*(3-2p)`; the card is fixed at its start rect and transforms: `translate(dx*e, dy*e - sin(p*PI)*24)` `scale(1 - e*0.72)` `rotate(e*8deg)`, `opacity = min(1, t*5)` (so it fades only at the very end... i.e. fully opaque for first 80% then fades). `dx,dy` = vector from up-card centre to the dealer seat anchor centre. z-index 20, pointer-events none.

### 6.4 Trick gather ("collect") to winner's stack
Starts when `trick.winnerSeat !== null`:
- Auto mode (`reviewTricks` false): `setTimeout(collect, play + read = 2200 ms)` after the winner key (`handNo:trickIndex`) appears. Timer is cancelled if the view/key changes. Cards are visible with the winner outline during these 2.2 s (700 ms card flight + 1500 ms read pause).
- Review mode (`reviewTricks` true): no timer. The `Continue` button calls `continueTrick()`: runs `collect()` first (button label `Collecting…`), then `onContinue(turnId)` -> backend `reviewTrick({action:'continue', turnId})`. If the ack is not `queued`, restores: clear `collectedKey`, cancel animations, show error `Could not update the table. Please try again.` (`role=alert`).
- `collect()`: target = rect of `[data-stack=winner]`. For each played card (DOM order = seat order [left, partner, right, you] filtered to present plays), animate over `780ms`, delay `index * 45ms`, easing `cubic-bezier(.4,0,.2,1)`, `fill: forwards`:
  - 0%: `none`, opacity 1
  - 60%: `translate(dx*.55, dy*.55 - 20px) scale(.7) rotate(-5deg)`, opacity 1
  - 100%: `translate(dx, dy) scale(.18) rotateY(90deg)`, opacity 0
  `dx, dy` = vector from the card centre to the stack centre. After all finish -> `collectedKey = trickKey` (cards become `visibility:hidden`) and `playTrickTake()`. Under reduced motion the flight is skipped but the state change still occurs.
- Collect is idempotent per `trickKey` and guarded against re-entry.
- A new hand/unmount cancels all running animations.

### 6.5 Other timings
| Thing | Value |
|---|---|
| Rack hover lift | `translateY(-4px)`, 180ms ease |
| Rack selection lift | `perspective(1000px) translateY(-8px) rotateX(2deg)`, 180ms |
| Deal-in | 580ms, stagger 75ms/index |
| Banter bubble | shown 6500 ms; enter 180ms ease-out (`opacity 0, translateY(4px)` -> rest) |
| Illegal message | 3200 ms |
| Announcement banner (GameFeedback) | 4500 ms |
| Confetti | 24 pieces, fall 2400ms ease-in with delay `(i % 6) * 70 ms`, x = `(i*37)%100 %`, rotate 540deg + fade, colors `#e0c579 #497d69 #ba5e46` cycling, piece 8x14px, z 11 |
| Settings rail slide | 320ms `cubic-bezier(.22,1,.36,1)`; backdrop 200ms |
| Home button hover | 200ms |
| Nav link hover | 160ms |
| Resync poll | every 10 000 ms while connected and window visible |
| Snapshot request timeout | 4000 ms; action default timeout 12 000 ms |
| Session token refresh | refresh when within 10 s of `expiresAt` |

Reduced motion: honor OS "reduce motion" (and the stored `table.reduceMotion` pref if wired): no card flights, no deal-in, no pickup, no confetti, no settings slide; state changes are instant. Collect/pause logic and the 2.2 s read hold are preserved (comprehension time).

---------------------------------------------------------------------------------------------------

## 7. Interaction summary (table)

| Intent | Input | Result |
|---|---|---|
| Select a card | click / tap card; focus card + Enter or Space | selects (single selection; replaces previous); label shows `Selected ✓`; Play button enables |
| Play selected card | Click `Play {card} →` | `submitMove('play:<card>')` |
| Play immediately | double-click card | submit `play:<card>` / `discard:<card>` right away |
| Move between cards | Left/Right arrows, Home, End | moves focus, wraps |
| Illegal card | click it | selection NOT changed; explanatory message for 3.2 s; error cue sound optional |
| Order up / call suit / pass / cut | BidPanel buttons | `submitMove(<id>)`; `Go alone` checkbox swaps to `+alone` id |
| Discard | during `dealer_discard` (you are dealer): select a card then `Discard {card} →` (or double-click) | `submitMove('discard:<card>')` |
| Continue after trick | `Continue →` (only in review mode) | collect animation then `reviewTrick({action:'continue', turnId})` |
| Hint | `Hints` checkbox | shows `Try: ...` suggestion on your decisions (section 13) |
| Review hands | `↶ Review hands` | modal (section 11) |
| Toggle review-each-trick | settings checkbox | `reviewTrick({action:'set', enabled})` |
| Close panels | `Escape` | closes settings panel (if open and no `dialog[open]`); native dialogs close on Escape |
| Reconnect (loading failure) | `Reconnect to this game` button | full reload / reconnect |

`submitMove(moveId)` contract (page): ignore if no view, `store.submitting`, or not connected; look up the entry in `view.legal` (ignore if absent); record `lastDecision = {view, moveId}` (for the Hint streaks); if it is a `play` move capture the hand card's artwork rect for the flight origin; compute an optimistic view via `applyOptimistic(view, move)` (exact only for optimistic fields; it clears `legal`) stamped with `forV = view.v`, shown ONLY while the live view's `v` equals `forV` (any newer real sync replaces it); await `store.play(moveId)`; if it returns false drop the optimistic view (server truth is shown). `TableStore.play`: reject locally with `That move is not legal right now.` if the id is not in `view.legal`; set `submitting`; send actor action `submitMove({moveId, turnId: view.turnId, clientMoveId: <uuid>})`; on a falsy/failed ack: resync; if the turnId has advanced, treat as success (the move actually landed); else error `The table could not confirm your move. Your game is saved. Please try again.`; on thrown error: resync, same turnId check, else `Connection interrupted. Your game is saved. Reconnect and try again.`; after an ack with `v` newer than the view, resync.

The view store rejects stale syncs (`payload.view.v < view.v` ignored). "Last good view" is latched: once any view has been shown, a null never blanks the table.

---------------------------------------------------------------------------------------------------

## 8. Table talk / AI banter bubbles

Source: actor event `chat` payload `{ msgId, seat, kind, text, final: true }` <!-- critic: protocol/index.ts:256-270 ChatEvent carries `final: true`; kind is 'call'|'banter'|'system'. Streaming partials arrive as `chatDelta {msgId, seat, delta}` (02 §3.4). -->. The store keeps the last 6 lines (dedup by `msgId`). Only `kind === 'banter'` with seat 0..3 becomes the on-felt bubble.
- Bubble = the `.table-quip` line in the bottom status row of the felt (NOT a speech bubble near the seat). Content: speaker name (`playerName`, color `#e9d79f`, weight 600) then the quote in curly quotes `“…”`, 16px/1.4 (14px <=700), max-width 680, max 2 lines (line clamp), center, `aria-live="polite"` role status.
- Lifetime 6500 ms (newer banter replaces and restarts the timer). It is only displayed while `talk.handNo === view.handNo` (hidden once the next hand starts). It has lower priority than the trick-won message and higher priority than the alone/instruction text (see 5.4.5).
- Engine-authored `call` lines ("Order it up.", "I assist.", "Next.", "March!", "Euchre!") also arrive on `chat` but are NOT bubbled; they appear in the `Table talk` disclosure and are screen-reader-spoken via `bids[].say`.
- Banter max length 140 chars (`BANTER_MAX_CHARS`). Whether banter is enabled is a profile setting `table.banter` (default true; settings label `Let opponents talk at the table`).
Rune suggestion: also optionally render the quote as a small rounded tooltip anchored under the speaker's seat label for 6.5 s; keep the bottom line as the accessible/guaranteed channel.

---------------------------------------------------------------------------------------------------

## 9. Match and hand outcome moments (`GameFeedback.svelte`)

Fed by `lastSteps` (the `steps` array of the most recent `sync` event) and the view. Dedupes by key `${handNo}:${step.t}:${JSON(step)}` (set capped at 80). Steps handled:
- `trumpSet{suit,aloneSeat}`: `playTrumpCalled(suit)`; if aloneSeat != null: `playMarch()` and banner `{seatName(aloneSeat)} {are|is} going alone!` (`are` for You), no confetti.
- `handScored{result}`: `lone_march` -> `playMarch()` + banner `All five alone! Four points.` + confetti; `euchre` -> `playEuchre()`; `march` -> `playMarch()`; any other result (point, lone_point, throw_in) -> `playHandWon()`. (No banner for non-lone-march results.)
- `gameWon{team}`: `playGameWon()` + banner `Your team wins!` (confetti) if `team === you % 2`, else `A good game. Well played.` (no confetti).
Banner: fixed, centered horizontally at `top: 104px` (96px <=700w), padding 18x24, radius 14, bg `#fff8df`, text `#253c2d`, serif 600 24px (20 on phones), prefix star `★` `#967629`, shadow `0 8px 32px #00180d44`, `role=status`, pointer-events none, z 12, auto-hides after 4500 ms. Confetti layer fixed full-screen z 11, only when `!reducedMotion`.

### End of match
When `status === 'complete'` (or `abandoned`):
- Instruction line: `Your team wins the match!` (if `winnerTeam === you % 2`) or `The other team wins the match.`; abandoned: `This game has ended. Start a new game whenever you’re ready.`
- Hand heading: `A good game, well played.`; right-side button becomes link `Play again →` -> `/new`.
- Rack: cards unavailable (`This match is finished.`), controls hidden.
- Settings panel still offers Review hands, links to `/new`, `/games`, `/`.
- The match is archived in the player's profile and shows in `/games` (after the backend writes the durable record; completed tables are retained 30 days).
- (Unused `TableStatusBar` text, not shown in current UI: `You won the match` / `Match complete` / `Your turn` / `At the table`.)

Hand-end: after the 5th trick (`trick.index === 4`): result line `That’s the last trick of this hand.`; `phase === 'hand_score'` instruction `Hand complete. Adding the points…`; score numerals/ScoreCards update on the next view; next hand deals (rack deal-in animation, `Hand {n+1}` label).

---------------------------------------------------------------------------------------------------

## 10. Screen reader narration (`GameAnnouncer.svelte`)

Visually-hidden `role="status" aria-live="polite" aria-atomic="true"` region whose text is recomputed per `sync` from `steps` (only the steps in the last sync). Duplicate `dealt` steps in one sync collapse to one. Seat names: `You`, `Left opponent`, `Your partner`, `Right opponent`. Sentences:
- `cut`: `{who}: Bump.` / `{who}: Run 'em.`
- `dealt` (only if `hand.length > 0`): `Cards dealt. Your hand: {cardnamelower, ...}.`
- `upCardTurned`: `Up-card is the {cardnamelower}.`
- `turnedDown`: `{Suit} turned down. {Suit} may not be named.`
- `trumpSet`: `{Suit} are trump. You called it.` (maker is you) / `{Suit} are trump, made by {SeatName}.` + alone: ` You are going alone.` / ` {SeatName} is going alone.` + ` The {left bower cardnamelower} is now a {suit noun}.`
- `dealerDiscarded`: `You discard face down.` / `{SeatName} discards face down.`
- `cardPlayed`: `You play the {card}.` / `{SeatName} plays the {card}.`
- `trickWon`: `{Who} take|takes the trick. {n} of five.` + (n<5) ` Your lead.` | ` Partner leads.` | ` {SeatName} leads.` (partner = same team as you)
- `handScored`: `{Point.|Lone point.|March!|Lone march!|Euchre!|Thrown in.} ` + (`You score {n}.` | `They score {n}.` | `Nobody scores.`) + ` {you} to {them}.` + ` You are in the barn.` / ` They are in the barn.` if one side is in the barn (isInTheBarn) and game not over
- `throwIn`: `All passed twice. Thrown in — the deal moves left.`
- `gameWon`: `You win, {a} to {b}.` / `They win, {a} to {b}.`
- `bid`: silent (the `chat` call line carries it).
Native: expose the same strings through the platform accessibility announcement API (Rune a11y bridge if available; otherwise a hidden live text node mirrored to the OS where supported).

---------------------------------------------------------------------------------------------------

## 11. Hand review (`HandReview.svelte`)
Launcher: `↶ Review hands` button (44px min, bg `#fffdf6`, text `#233e32`, border `#bbc9ae`, radius 8, 16px/600). Opens modal `<dialog aria-labelledby="review-title">` width `min(720px, 100vw-24px)`, max-height `100dvh-24px`, padding 24, radius 16, bg `#f4f3e9`, backdrop `#10251dcc`.
- Header: h2 `Review a hand` (serif 500 28), close `×` (`aria-label="Close trick review"`).
- Selector: label `Hand` + select with options for `0..handNo`: `{n+1} · Current` for the current hand, `{n+1} · Finished` otherwise. Opening defaults to the current hand, or the previous hand when `handNo > 0`, the current hand has no completed tricks and no result.
- Loading `Opening the hand…` (`role=status`); failure `Could not load this hand. Please try again.`; archive absent for an old hand: `This older hand was not saved by the previous version.` (`role=alert`). Late responses are discarded (request counter).
- Data: `table.getHandReview(handNo)` -> `CompletedHandReview { handNo, dealerSeat?, tricksWon?, upCard?, trump, makerSeat, aloneSeat, tricks[], buried[], unplayed[], result, delta }`. For the current hand the live `view.trickLog` is used when no archive exists.
- Summary block:
  - thrown in (`result==='throw_in'` or no trump and no tricks): `**Everyone passed. No points were scored.** The deal moved to the next player.`
  - else: `**{TrumpName|'Unknown suit'} were trump.** {seatName(maker)} named trump[ and went alone].`; if tricks present: `Tricks: your team **{n}** · other team **{m}**.`; if `delta`: `**{Your team|The other team} scored {max(delta)} {point|points}{ — euchre | — all five tricks | ''}.**` (euchre = result `euchre`; all five = `march` or `lone_march`); (note: uses `delta[you%2] > 0` to choose team.)
  - if `dealerSeat` defined: `Dealer: {seatName(dealer)}.` + ` Up card: {cardName}.` when present.
- Per trick section: h3 `Trick {index+1} · {seatName(winner)} won` then a flex wrap of figures (4 per row, `(100% - 36px)/4` width, gap 12) each showing the card image with caption `{i+1}. {seatName(seat)}` in play order.
- No tricks: `No completed tricks in this hand yet. Choose an earlier hand above to review it.`
- `Buried cards` section: card images of `archive.buried` (after hand end only). `Cards that stayed in hand` if `unplayed` non-empty. Without an archive: `The buried cards are revealed only after this hand is finished.`
Privacy: buried/unplayed cards come ONLY from this separate post-hand contract; never from the live view.

---------------------------------------------------------------------------------------------------

## 12. All `turnInstruction(view)` strings (`table-presentation.ts`) - first match wins

1. `status === 'abandoned'`: `This game has ended. Start a new game whenever you’re ready.`
2. `status === 'complete'`: `Your team wins the match!` if `winnerTeam === you%2` else `The other team wins the match.`
3. `phase === 'hand_score'`: `Hand complete. Adding the points…`
4. `awaitingTrickReview`: `Press Continue to finish this hand.` (trick.index == 4) / `You lead next. Press Continue first.` (winner is you) / `Press Continue when you’re ready for the next trick.`
5. `phase === 'trick_resolve'`: winner null -> `Finishing this trick…`; else `{seatName(winner)} {win|wins} the trick.`
6. `sittingSeat === you`: `Your partner is going alone. Enjoy the hand.`
7. not your turn: turnSeat null -> `Getting the table ready…`; else `{seatName(turnSeat)} is {deciding on trump|taking a turn}…` (`deciding on trump` when phase starts with `bid`)
8. `dealer_discard`: `You’re the dealer. Select one card to discard.`
9. `cutting`: `Your choice: cut the deck, or let the dealer deal.`
10. `bid_round_1` with upCard: `Make {suit lowercase} trump, or pass.`
11. `bid_round_2`: legal includes pass -> `Choose a trump suit, or pass.` else `Everyone passed. Stick the dealer is on: choose trump.`
12. trick play: ledSuit set -> `{SuitName} were led. Follow suit if you can.` else `You lead. Select any card, then press Play.`
Pacing constants of copy: all use typographic apostrophes (’) except the illegal-card messages that use straight (').

---------------------------------------------------------------------------------------------------

## 13. Hints (coach mode) - `Hint.svelte`
- Toggle label `Hints` in the settings panel; OFF by default; persisted in `localStorage` key `euchre:hints:v1:enabled` (`'1'|'0'`) - native: local settings file. Streak key `euchre:hints:v1:streaks` (JSON map SituationClass -> int).
- Situation classes: `cut` (phase cutting), `bid1`, `bid2`, `discard` (dealer_discard), `lead` (trick_play, ledSuit null), `follow` (trick_play, ledSuit set); other phases none.
- Suggestion visible when enabled AND `status==='active' && turnSeat===you && legal.length > 1` AND the class is not retired. It is `rankMoves(view)[0]` from the AI crate (heuristic ranking): `Try: <strong>{label}</strong> — {gloss(why)}` where `label` is the legal move's label (fallback id), `why` is the heuristic's rationale; the `— why` part is omitted if `why` is empty or equals the label ignoring case. `gloss`: capitalize first letter, end with a period; the first time per session one of these jargon patterns appears, append ` — meaning {meaning}.` (once per term per session): `ruff` -> `trump a card in a suit that wasn’t led`; `lay off` -> `play a low card since you can’t win this one`; `third hand high` -> `you’re third to play, so play your best card`; `second hand` -> `you’re the second to play this trick`; `bower` -> `one of the two top trump jacks`.
- Auto-retire: after each real human decision (`lastDecision{view,moveId}`), compare `moveId` to `rankMoves(decision.view)[0].id`; correct -> streak[class]++ else reset to 0; when `streak >= 3` (RETIRE_AFTER) that class no longer shows hints; a miss resets and brings it back.
- Style: suggestion box in settings panel: padding 12, bg `#e6eadc`, border `#cad3c1`, text `#233e32`, 16px/1.5, `strong` `#285641`, role=status.
- Needs the Rust AI crate to expose `rank_moves(view) -> Vec<RankedMove{id, why}>` (see AI spec lane).

---------------------------------------------------------------------------------------------------

## 14. Loading, connection and error states

### 14.1 Connecting / no view yet (`.loading`, role status, full viewport centered, gap 20, padding 32)
- Big club glyph `♣` (54px).
- H1: `A seat at the table.` while not connected; `Your hand is on its way.` once connected.
- P (19px): `Connecting to your game…` / `Dealing your cards…`
- Button `Reconnect to this game` (min 48 high, bg `#285641`, white, 18px/600, radius 8, border `#bdc9ae`; action: reload page / reconnect) and link `Your saved games` -> `/games` (18px).
- Error line (`.err` style) if connection/session mint failed: shows `connectionError` (the message from the mint) or fallback `We couldn’t open this table. Try reconnecting, or open Your saved games.` Mint error messages: HTTP 403: `This table belongs to another browser. Open Your games to find your table.`; 401: `Please start a game in this browser first.`; other: `Your table is temporarily unavailable. Please reconnect.`; thrown fallback `Unable to open this table.`
- Native mapping: session mint = `POST /api/table-session/{gameId}` (backend lane; 05 section 4.2 and 11.2) <!-- critic: was `POST /tables/{id}/session`, which no spec defines --> returns `{actorId, token, gatewayToken?, expiresAt}`; connect to actor `euchreTable` with key `["table", gameId]`, `noCreate: true`, connection params `{ token }`; token cached until `expiresAt - 10s`; gateway token refreshed via `getToken({forceRefresh})`.

### 14.2 Reconnecting banner (connected -> disconnected while a view exists)
Banner `.notice` at top of the table layout (`role="status"`): `Reconnecting… Your table is saved. Controls will return when you’re connected.` Style: padding 14x18, bg `#fff1d9`, border `1px #d5b68c`, radius 8, 17px, text `#233e32`, absolute top 8px, left/right 24px, z 6. While disconnected: rack, bid buttons, Play, Continue are all disabled; the last good view stays on screen (never blanked). On reconnect (`isConnected` true) the client immediately calls `snapshot()` and applies it if `snap.v >= view.v` (snap, no animation). Failures show store error `Waiting for the table to reconnect. Your game is saved.` (snapshot rejects/timeout 4000 ms or undefined). While connected+visible, resync every 10 s.
- Review-action errors: `The table did not confirm. Please try again.` (internal throw) surfaced as `Could not update the table. Please try again.` (`role=alert` `.err` box).
- Submit errors: see section 7.

### 14.3 First-run tour (`onboarding/Walkthrough.svelte`)
Modal dialog (max-width `min(30rem,92vw)`, serif font, text `#f2e8d5`; sheet `#12362b`, border `rgba(232,194,122,.35)`, radius .9rem, shadow `0 12px 40px rgba(0,0,0,.55)`, backdrop `rgba(6,5,3,.72)`). Auto-opens ONCE, the first time a real hand is on the felt (`view.hand.length > 0 || phase not in {lobby, cutting, deal}`), unless seen (`localStorage euchre:onboarding:v1:seen === '1'`). Dismiss (Skip / Escape / backdrop / finish) marks seen. Manually reopenable from the rules panel (resets to slide 0, does not re-arm auto-show). Eyebrow `Quick tour · {n} of 6`; dots indicator (7.2px; inactive `rgba(232,194,122,.28)`, active `#e8c27a`); buttons: `Skip` (ghost) ... `Back` (ghost, step>0), `Next` / last `Deal me in` (primary `#d4ee86` on `#12362b`). Slides (title / body):
1. `Welcome to the table` - `Four players, two teams. You and Your partner play together. The left and right opponents are the other team. First team to 10 points wins.`
2. `The deck` - `Euchre uses 24 cards: nine, ten, jack, queen, king and ace, in all four suits. That's it — nothing lower than a nine is in the deck.`
3. `The bowers — the one rule to know` - `The jack of the trump suit is the best card in the game — it's called the right bower. But there is a second trump jack: the jack of the OTHER suit that shares its colour. If hearts are trump, the jack of diamonds turns into a heart for the rest of the hand — that's the left bower, and it's the second-best card on the table.`
4. `Bidding` - `Each hand turns one card face up. Round one: you may "order it up" to make its suit trump, or pass. If everyone passes, that card is turned down and round two lets you name any OTHER suit, or pass again. Confident in your hand? Go alone — your partner sits out and a perfect hand is worth double.`
5. `Scoring` - `Take 3 or 4 tricks: 1 point. Take all 5 — a march — 2 points. Go alone and take all 5: 4 points. Call trump and win fewer than 3 tricks and you’ve been euchred: the other team scores 2 instead.`
6. `You're ready` - `Select a card, then press Play. Cards you cannot play explain why when you tap them. How to play opens the rules, and Settings has hints and a Wait for me option so you can review each trick at your own pace.`

---------------------------------------------------------------------------------------------------

## 15. Sound (`src/lib/ui/sound/`)

### 15.1 Policy and toggle
Synthesised (WebAudio oscillators/noise), no asset files. `soundSettings.enabled` defaults to FALSE (opt-in; persisted key `euchre:sound-enabled` = `'1'|'0'`). Every cue is a no-op when disabled or the window is hidden. Audio context resumes on first pointer/key gesture. `SoundToggle`: 44px button, `aria-pressed`, `aria-label` `Mute table sound` / `Unmute table sound`, `title` `Sound on` / `Sound off`, 20px speaker icons (waves = on, X = off). Turning ON plays `playTrumpCalled('H')` as a test cue. (Native port: use rodio/cpal; synthesize the same graph or ship tiny samples.)

### 15.2 Cue recipes (gain scale 0..1 linear ramp attack then exponential decay to 0.0001)
| Cue | Trigger | Recipe |
|---|---|---|
| `playDeal` | first dealt card in the rack; hand change | noise burst 0.18s, gain .1, bandpass 2200 Hz Q .6 |
| `playPlace(cardId?)` | card lands | triangle tone 150 Hz (shifted by deterministic -3..+3 semitones hash of card id; hash FNV-1a 2166136261, prime 16777619, `(h%1000)/1000*2-1)*3`), 0.12s, gain .22, attack .004 + lowpass noise 0.07s gain .05 @900 Hz |
| `playLift` | hover (unused in live table) | highpass noise 3800 Hz 0.09s gain .045 Q .5 |
| `playTrickTake` | collect finished | bandpass noise sweep 3200 -> 500 Hz, 0.26s, gain .13, Q .8 |
| `playTrumpCalled(suit)` | trumpSet | triangle 0.3s gain .24 attack .006 + octave overtone; S 392.0, H 440.0, D 493.88, C 523.25 Hz |
| `playEuchre` | handScored euchre | sine 440 Hz 0.22s then 440*2^(-3/12) at +0.14s 0.24s, gain .22, overtones |
| `playMarch` | march / lone / alone call | triangle F4 349.23, A4 440, C5 523.25, E5 659.25 each 0.16s gain .2 at i*0.08s, overtones |
| `playHandWon` | other hand results | sine chord C5 523.25, E5 659.25, G5 783.99, 0.55s gain .14 attack .02, stagger 0.05s |
| `playGameWon` | gameWon | triangle G4 392 @0, B4 493.88 @.12, D5 587.33 @.24, G5 783.99 @.4 (0.5s, gain .2, overtones) then sine chord 392/493.88/587.33/783.99 at +0.55s 0.7s gain .12 |
| `playError` | (defined) | lowpass noise 0.08s gain .16 @550 Hz Q .4 |
Haptics `vibrate()` is a web feature - omit on desktop.

---------------------------------------------------------------------------------------------------

## 16. Decorative 3D layer (Threlte) and the 2D Rune equivalent

What is actually live on `/play/[gameId]`: ONLY `TableFelt3D.svelte` -> `FeltSurface.svelte`: a full-bleed orthographic canvas (dpr 1, on-demand render, `aria-hidden`, `pointer-events:none`, radius inherit, overflow hidden) behind the DOM table: one plane `2*aspect x 2` with a procedural felt texture, `MeshStandardMaterial color #a4c49e roughness .96`, ambient light 1.8, one point light `#fff1c9` intensity 5 at (-0.8,1.2,2). It is lazily imported after mount and silently omitted if WebGL fails ("CSS felt remains usable"). The felt texture (`feltTexture.ts`, 512 px): radial gradient center `#245a49` (r 0.15*S) -> edge `#10382c` (r 0.72*S), plus 900 deterministic 1px speckle dots at 5% white/black alpha (`grain(x,y)=frac(sin(x*12.9898+y*78.233)*43758.5453)`; dot i at `(grain(i,1.7)*S, grain(i,91.3)*S)`, white if `grain(i,5.2)>.5` else black). The "marked" variant (unused here) adds a dotted ring at r .455/.443 S (stroke `rgba(218,230,176,.22)`), a `♣` (36px Georgia, fill `rgba(223,234,197,.16)`) at y .38 S and italic `euchre` 20px at y .45 S.

The full 3D scene (`EuchreTable3D.svelte` + `TableScene.svelte` + `layout/*`, `scene/*`, `cards/*`, `static/art/{table,dealer,card-stock}.glb`) is NOT mounted by the shipped play route (it is leftover from the 3D design; README still describes a 3D table). Do not port it. Its useful facts if a 3D mode is wanted later: tempo constants (`TEMPO`: deal flight 180/stagger 50, card play flight 560 with arc .03 m, trick sweep 680 with stagger 80, winner pulse delay 150, hand-score delay 600 / counter 450, celebration 1400 point-march / 2400 euchre-loner, up-card reveal 260, game won 2400), felt radius 0.45 m, dealer-button positions per dealer seat `[0.22,0.003,0.21] [-0.27,0.003,-0.12] [-0.17,0.003,-0.26] [0.27,0.003,0.12]`, kitty position `[-0.13,0.003,-0.12]`, celebration ring colors point/march/lone `#e8c27a`, euchre `#c0554a`, throw_in `#8a8a8a`. Lighting: hemisphere sky `#fffaf0` ground `#586e60` 2.0; directional `#fff3df`; fill `#d8e8f5` 0.65; table rail color `#102820`.

### 16.1 Rune 2D equivalent (recommended)
Render the table region (`.table-layout`) as a single rounded-rect "felt" widget:
1. Base fill radial gradient centered ~(50%, 45%): `#2f6a55`-ish center fading to `#10382c` at the rim; the page's flat color is `#17452f` (use as the clear color / base) and the Threlte texture `#245a49 -> #10382c` as the decorative gradient on top. Concretely: `radial_gradient(center, r_inner = 0.15*S, #245a49 -> r_outer = 0.72*S, #10382c)` stretched to the felt rect.
2. Grain: pre-generate a 512x512 RGBA tile with the deterministic 900-dot speckle above (or a 256x256 tile with ~225 dots), tile it at 5% alpha; static, no per-frame cost.
3. Optional subtle rim: 1px inner stroke `rgba(218,230,176,.22)` inset ~12px with rounded corners (matches the "marked" tailoring) and a faint centered `♣` + `euchre` watermark at 16% alpha in serif.
4. Wood rail (decorative; the live app has no rail, the Blender GLB had walnut/brass): a 10-16px rounded border around the felt: `linear-gradient(180deg, #5a3a22, #3b2415)` with a 1px brass line `#b4a57a` on the inner edge and a 3px dark under-shadow `#153c2b` (this reuses the shipped felt-card `0 3px 0 #153c2b` bottom edge and top border `5px #44684e` from the pre-continuous-felt design). Keep it purely decorative with pointer events disabled; the felt must remain legible and cards must not overlap the rail.
5. Hero art on the Home screen: bundle `euchre-hero.webp` (1200x1000) as a static image with the same radial alpha mask.

---------------------------------------------------------------------------------------------------

## 17. Games history `/games` and match detail `/games/[matchId]`

Dark scrapbook theme (see 1.2), page container max-width 42rem, padding `2rem 1.25rem 4rem`, serif font. Identity is an email "nickname" (no verification; hashed to a profile key) held in an HTTP-only cookie; native port: store the chosen email in the local config and pass it to the profile actor/API.

### 17.1 `/games`
Title `Your games · Euchre`.
- Saved-tables band (cream panel `#f4f3e9`, radius 12, padding 20), shown if recent tables exist: h2 `Your saved tables`, p `Resume in this browser, exactly where the table is waiting.` then link rows (min-height 48): `Resume game · Hand {handNo+1} · {a}–{b} →` for `status==='active'` (`Open game · ...` otherwise); fallback when snapshot unavailable: `Reconnect saved table · {first 8 chars of gameId} →`. Up to 12 recent tables are remembered per browser (bookmarks grant no access; the table verifies ownership).
- If some tables could not be opened: `Some saved tables could not be opened. Their links are kept above; try reconnecting. Completed matches are also listed below.` (role status)
- Browser-identity games band (matches completed under the guest identity before choosing an email): h2 `Games from this browser`, p `Matches you completed before choosing an email stay here too.`, rows `{date} · {Won|Lost|Left unfinished} · {a}–{b} →` -> `/games/{matchId}?profile=browser`.
- No identity yet: **IdentityGate** (section 17.3) with heading `Whose scrapbook is this?` and lede `Enter your email address and we'll start keeping score — every match you play from here on will show up on this page.`
- With identity: header `Euchre` (uppercase 1rem, ls .08em, `#93876f`), h1 `Your games`, `PlayingAs` line.
  - Load failure banner: `Couldn't open your scrapbook just now. Try refreshing the page.`
  - Stats grid (`aria-label="Lifetime stats"`, shown only if `matches > 0`): tiles with value (1.35rem/600 `#e8c27a`) over label (.72rem `#93876f`): `{matches}` `Matches played`; `{round(winRate*100)}%` `Win rate`; `{won}–{lost}` `Record` (+ ` (+{abandoned} left unfinished)` when abandoned > 0); `{euchresFor}–{euchresAgainst}` `Euchres sent–taken`; `{lonersMade}/{lonersAttempted}` `Loners made`; `{handsPlayed}` `Hands played`.
  - List of match cards (page size 10, keyset cursor pagination): top row date (format `Intl` month short, day, year, hour, minute) + outcome chip (`Won` / `Lost` / `Left unfinished`); mid row `You {a} – {b} them` and `{n} hand(s)`; optional `At the table: {opp1} · {opp2} · {opp3}`; card links to `/games/{matchId}`.
  - Pager `aria-label="Older and newer games"`: `← Newer games` / `Older games →`; empty page: `No more games on this page.`
  - Empty state: h2 `Nothing here yet`; p `Play your first hand and it'll land right here — the date, the final score, whether you sent them home early, all of it. Think of this as the scrapbook on top of the fridge.`; CTA `Deal a hand` -> `/new`.
Backend calls: `playerProfile.getStats()` -> `ProfileStats{matches,won,lost,abandoned,winRate,euchresFor,euchresAgainst,lonersMade,lonersAttempted,handsPlayed}` and `listGames({limit:10, cursor})` -> `MatchListPage{rows[{matchId,playedAt,outcome,score,handsPlayed,opponents[]}], cursor}`.

### 17.2 `/games/[matchId]`
Title `Game on {date} · Euchre`. `← Back to your games`; header: date (weekday long + date + time), h1 `You won` / `They won` / `Left unfinished`, `You {a} – {b} them`, `At the table: ...`. Totals grid (`aria-label="Match totals"`): `Hands played`, `Euchres sent–taken`, `Loners made` (`made/attempted`), `Marches`, `Thrown in`, `Tricks won` (`{a}–{b}`). Ordered list of hands: `Hand {n+1}` + `Dealer: {You|West|North|East}`; line `{glyph} {SuitName} called by {seat}` (+ ` · alone`) or `No trump — thrown in`; `{a}–{b} tricks`; result line (verbatim): point `Took the majority — 1 point`; march `Swept all five tricks — 2 points`; lone_point `Went alone, took the majority — 1 point`; lone_march `Went alone, swept all five — 4 points`; euchre `Euchred the makers — 2 points`; throw_in `Thrown in — no points`; null `Unresolved`; plus delta `(+{n} us)` / `(+{n} them)` / `(no score)`. Note this page names seats by compass (`You, West, North, East`) while the live table uses `Left opponent/Your partner/Right opponent`; keep both as-is (or unify to the live-table names in the port; flag in 20).

### 17.3 IdentityGate + PlayingAs
- Gate card: max-width 28rem, margin 3.5rem auto, padding 1.75rem, radius .85rem, bg `rgba(15,20,14,.6)`, border `rgba(232,194,122,.22)`. H1 (clamp 1.4-1.8rem) heading, lede (`#c9b89a`), form: label `Your email`, email input (id `gate-email`, placeholder `you@example.com`, required, 44px high), submit `Remember me here`; error (role alert) `That doesn’t look like an email address yet.`; fine print: `No password, no verification email — this just labels the games this browser remembers as yours. Nothing is sent anywhere; it only tells this app which scrapbook to open.` (native: say "this device").
- PlayingAs: `Playing as <strong>{email}</strong>` + button `Not you? Switch profile` (signs out: clears identity).

---------------------------------------------------------------------------------------------------

## 18. Settings `/settings`

Title `Settings · Euchre`; dark scrapbook theme; max-width 42rem. No identity -> IdentityGate with heading `Whose opponents are these?` and lede `Enter your email address and the three seats you set up here will be waiting every time you play.` With identity: `Euchre` brand line, h1 `Settings`, PlayingAs; load failure `Couldn't open your settings just now. Try refreshing the page.`

Disclaimer box (verbatim): `What this does and doesn't do: everything below shapes how your three opponents sound and how readily they bid or gamble. **It cannot change the rules of euchre, let a seat see cards it shouldn't, or make an illegal move legal** — those are enforced by the game itself and no amount of text here reaches them. Treat these prompts the way you'd treat a note passed to an actor, not a rulebook.`

Form (POST save). Banners: error `{form.error}` (role alert); success `Saved. Your next match will use these.` (role status). Save failure copy: `Couldn’t save that — one of the fields may be too long or an odd value.`; not identified: `Tell us who you are before saving.`
Sections as cards (bg `rgba(15,20,14,.55)`):
1. **The table** - hint `How the game presents itself. Nothing here touches how anyone plays.`:
   - `Pace` select: `Brisk` / `Normal` / `Relaxed` (values `brisk|normal|relaxed`, default `normal`)
   - checkboxes: `Let opponents talk at the table` (banter, default on), `Show each seat's reasoning after a hand` (showRationale, default on), `Sound` (default on), `Reduce motion` (default off).
   (These are stored in the profile; note the live table currently reads sound from its own local toggle and motion from the OS preference, and pace on the backend - flag in 20.)
2. **House prompt** - hint `Sets the overall mood of the table — applies quietly to all three opponents at once.`; textarea rows 3, maxlength `HOUSE_PROMPT_MAX_CHARS = 1000`; counter `{n} / 1000` (class `near` when > 90%).
3. Three persona cards for seats 1,2,3 with headings `West`, `North · your partner`, `East`:
   - `Name` text, max `PERSONA_NAME_MAX_CHARS = 32` + counter
   - `One-line blurb (shown at the table)` text, max `120` + counter
   - three sliders (range `DIAL_MIN=0` .. `DIAL_MAX=1`, step 0.05): `How eagerly they bid — {pct}%` (aggression), `How much they'll gamble on a call — {pct}%` (risk), `How much they talk — {pct}%` (chattiness); pct = round(value*100)
   - `Difficulty` select over `rookie|casual|expert` (shown lowercase) + explainer text: rookie `Plays a little unpredictably. Easygoing, forgiving of mistakes at the table.`; casual `Solid, sensible play most of the time — the middle of the road.`; expert `Weighs every legal option every time. The strongest seat at the table.`
   - Presets: text `One-tap starting points — fills in the blurb, prompt and dials below, doesn't save until you do:` then chips (each with a `title` = hint). Applying a preset overwrites blurb, prompt, difficulty and the three dials but NOT the name.
   - `Character prompt — untrusted text, style only` textarea rows 4, max `PERSONA_PROMPT_MAX_CHARS = 2000` + counter.
4. Submit `Save changes`.
Preset table (`settings/presets.ts`; label / hint / blurb / difficulty / aggression / risk / chattiness; prompts verbatim in the TS file - the port must copy `src/routes/settings/presets.ts` and the `personas.ts` defaults data verbatim):
| id | label | hint | blurb | diff | agg | risk | chat |
|---|---|---|---|---|---|---|---|
| grandpa | Grandpa | Experienced, warm, takes a calculated chance. | Takes a chance. Enjoys the story. | expert | .8 | .8 | .65 |
| counter | The Counter | Precise, patient, plays it by the book. | Counts every card and rarely misplays one. | expert | .3 | .25 | .25 |
| steady | Steady Partner | Encouraging, dependable, never second-guesses out loud. | Solid and supportive — never rattled. | casual | .45 | .4 | .45 |
| bold | Bold Bidder | Orders it up often, plays for the march. | Bids first, asks questions never. | casual | .85 | .8 | .6 |
| chatty | Chatterbox | Warm, talkative, keeps the table lively. | Has something to say about every hand. | casual | .55 | .5 | .85 |
| quiet-pro | Quiet Pro | Expert-level play, almost never talks. | Says little. Plays like it matters. | expert | .5 | .45 | .1 |
| wildcard | Wildcard | Unpredictable and a little reckless. | You never quite know what this one will do. | rookie | .7 | .85 | .65 |
Prompts (verbatim copy needed): grandpa `You are a seasoned kitchen-table player with a mischievous streak. Take plausible close calls and well-supported loners, then play sound partnership euchre. Never deliberately misplay, waste secured winners, coach, or hint at your cards. Your humour is warm and understated, like an old friend.`; counter `You are careful and exact. You track what has been played and almost never make a mistake. You speak rarely, and when you do it is short and dry.`; steady `You are calm and encouraging. You play sound, sensible euchre and never criticize a partner’s call out loud. When something goes well you say so briefly.`; bold `You bid aggressively and love going for a march. You are cheerful even when a bid goes wrong, and a little smug when it doesn’t.`; chatty `You are warm and talkative, with a running commentary on the game. You are good-natured about losing and generous in victory.`; quiet-pro `You are focused and nearly silent. You play at a high level and let the cards do the talking. On the rare occasion you speak, it is one short sentence.`; wildcard `You are unpredictable and take chances other players wouldn’t. You are delighted by a wild outcome, win or lose.`
Settings API: `playerProfile.getSettings()` / `updateSettings(patch)` where patch = `{ housePrompt, table:{pace,banter,showRationale,sound,reduceMotion}, personas:[{seat,patch:{name,blurb,prompt,difficulty,aggression,risk,chattiness}} x3] }`; the actor validates, clamps and sanitises every field (this UI trusts none of it). Seats 1,2,3 = `AI_SEATS`.

---------------------------------------------------------------------------------------------------

## 19. Assets

Directory `static/art/` (all paths as served: `/art/...`):
| File | Size | Use |
|---|---|---|
| `cards/{R}{S}.png` x 24 | 512 x 716 px each (ratio 5:7 = 0.715), RGBA PNG, 19-234 KB | the card faces; R in `9 T J Q K A`, S in `S H D C` (e.g. `9C.png`, `TH.png`, `JS.png`, `AD.png`) |
| `cards/back.png` | 512 x 716 | burgundy card back (original vector art) |
| `cards/{R}{S}.svg` x 24 + `back.svg` | viewBox `-120 -168 240 336` (2.5in x 3.5in) | vector sources (face SVGs 1.2-63 KB; jacks/queens/kings 32-63 KB, pip cards ~2 KB) |
| `cards/LICENSE.txt` | | Creative Commons CC0 1.0 Universal legal text |
| `euchre-hero.webp` | 1200 x 1000 | home hero art |
| `card-stock.glb` (7.2 KB), `dealer.glb` (194 KB), `table.glb` (795 KB) | | 3D models for the unmounted 3D scene (omit) |
Attribution (README): traditional faces are Adrian Kennard's CC0 artwork (https://www.me.uk/cards/makeadeck.cgi), packaged by letele/playing-cards; the back is original. Preload strategy: `warmCardArt()` creates an `Image`, `decoding='async'`, `src='/art/cards/{id}.png'` for every one of the 24 cards on mount and `decode()`s it (errors ignored) so no card face decodes at the moment it is animated. Native: load all 25 PNGs into GPU textures at startup (about 5 MB decoded at 512x716 RGBA is fine; or rasterize the SVGs). Display sizes: hand cards up to ~150px wide (126px on short windows), played cards <=145px (108px short), up-card/deck height <=190px (width = 5:7); rendered with 7px radius and the physical-stack shadow in 1.4. Cards are never recolored; there is NO four-color mode in the shipped UI (EuchreTable3D `fourColor` prop defaults false and is unused).
Card name strings: `RANK_NAME` `9 Nine, T Ten, J Jack, Q Queen, K King, A Ace`; `SUIT_NAME` `S Spades, H Hearts, D Diamonds, C Clubs`; `cardName = "{Rank} of {suit lowercase}"`; `cardNameLower` fully lowercase; `SUIT_NOUN` `spade heart diamond club`. Glyphs `S ♠, H ♥, D ♦, C ♣`.

---------------------------------------------------------------------------------------------------

## 20. Responsive behavior summary and Rune adaptation notes

### 20.1 Breakpoints used by the web UI
- Window width: `<=380`, `<=420`, `<=480`, `<=600`, `<=700` (phone), `<=760` (home), `<=850` (header container), `<=900`, `<=1050`, `<1200` (settings overlay), `>=1500` (home).
- Height: `<=500` (landscape phone), `<=650`, `<=750`, `<=850` (laptop).
The native desktop app can treat width as a continuous layout input: use the `>=701w & >=501h` desktop table layout as the primary design; implement compact variants (stacked seats row, 3-col hand, bottom-sheet-like settings overlay) for windows narrower than 700 or shorter than 500; settings panel inline if width >= 1200, overlay otherwise. Minimum window size recommendation: 800 x 600 desktop; the 320px min width is a web constraint. <!-- critic: 07 section 11 verified layout at a 720 x 520 `min_size`; pick one value in the desktop lane (720 x 520 is the verified floor) -->
Table layout is vertically bounded: no game state may push the hand off-screen (rows are minmax(0,1fr) with container-size sizing; card art scales to the available cell).

### 20.2 Known inconsistencies and dead code (decide explicitly in the port)
1. `TableStatusBar`, `TrickView`, `HandA11y`, `DiscardPanel` exist but are NOT imported by the live play page. Dealer discard uses card-select + `Discard {card} →`. Their strings (`Choose one card to tuck away.`, etc.) are unused. Do not port unless an alternate compact discard UI is desired.
2. `EuchreTable3D`/`TableScene` unused (README still advertises a 3D table; keep README claims out of the new docs).
3. Settings `table.pace`, `table.sound`, `table.reduceMotion`, `table.showRationale` are saved but only partially consumed: sound is a separate local toggle (default off) in the table panel; motion follows OS prefers-reduced-motion; pace/rationale are backend/profile concerns. In the port, wire `reduceMotion` to the animation switch and `sound` as the initial value of the sound toggle (flag as a decision).
4. `docs/04-FRONTEND-UX.md` (older plan) describes magic-link login, a 3D camera, a four-color deck by default, `HandA11y` as the primary hand, replay transport controls and a 6-source "Director"; none of that matches shipped code. The shipped behavior is in this document. `docs/FEEDBACK-2026-10-05.md` records fixes: human turns wait indefinitely; never blindly resend an uncertain move; AI bid pauses longer and passes remain visible by seat; completed trick highlights winner then pauses then gathers; "Wait for me after each trick" persists across reload; settings panel can remain open (the doc's "pinned/moved with arrow keys" panel was replaced by the rail described in 5.9).
5. Seat-label naming differs between the live table (`Left opponent/Your partner/Right opponent`) and `/games/[matchId]` + `TrickView` (`You/West/North/East`). Suggest unifying on the live-table names; keep `West/North/East` in settings persona headings (`West`, `North · your partner`, `East`).
6. Apostrophes: instruction strings use typographic ’; illegal-card and rules strings use straight '. Preserve per string.
7. Spelling: American (`color` etc.) except the tour slide 3 which says `colour`/`shares its colour` and the grandpa prompt `humour` - keep verbatim or normalize to American per the 2026-10-05 note "American spelling".

### 20.3 Rune mapping hints
- Everything is composed from: text, rounded-rect fills/borders, images (card PNGs), simple vector glyph text (♠♥♦♣ in the serif/sans font), SVG-ish score cards, linear/radial gradients, shadows, scrollable column (settings panel, rules, review), modal dialog with scrim, checkbox, select, range slider, text input, textarea with counter, disclosure (details/summary), toast/banner overlay. All card artwork is bitmap; no per-card vector drawing is required.
- Animations are property tweens on transform (translate, scale, rotate, rotateY squash as horizontal scale to 0 over the last 40% for the "edge-on" flip) + opacity with cubic-bezier easings; implement a tiny tween helper that takes keyframes `(offset, translate, scale, rotate, opacity)` with `cubic-bezier(.22,.7,.25,1)` (play), `(.4,0,.2,1)` (collect), `(.22,1,.36,1)` (panel).
- Needed layout anchors computed at runtime: rect of each hand card artwork (play origin), `seat anchor` centre per seat, `taken-stack` rect per seat, up-card rect, dealer seat anchor. Expose them from the layout pass.
- Keyboard map: Tab order = header (score brand link, How to play, Settings) -> Continue (when present) -> rack cards (arrow/Home/End roving) -> Play/Discard -> bid buttons -> go-alone. Esc closes the settings panel/dialogs. Suggested extra desktop shortcuts (not in source, optional): `P` Play, `Enter` on selected card plays it.
- Data flow equivalence (ViewModel): subscribe to actor event `sync {v, view, steps}` and `chat {msgId, seat, kind, text, final}` <!-- critic: SyncEvent carries `v` (protocol/index.ts:221-296; 02 §3.1). MoveAck failure is `{ok:false, code, legal}` with `code` required (protocol/index.ts:373-379); reviewTrick returns `{queued: true}` (euchre-table/index.ts:1798-1827). -->; actions `snapshot()`, `submitMove({moveId, turnId, clientMoveId})` -> `{ok:true,v} | {ok:false,code?}`, `reviewTrick({action:'set',enabled} | {action:'continue',turnId})` -> `{queued: boolean}`, `getHandReview(handNo)`; connection parameters `{ token }`. Keep these names (the TS web client will be retargeted to the Rust actors).

### 20.4 Accessibility labels checklist (all must exist on the native widgets)
`Your cards`, `Select {card}. ...`, `Bidding`, `Match score`, `{n} points, counted with a six and a four`, `The table`, `Up card: {card}`, `{Seat} played {card}[ — winning card]`, `Named trump`, `Leads this trick`, `Dealer`, `{Suit} led`, `Your hand and controls`, `Table settings` (dialog/complementary), `Close settings`, `Close settings panel`, `Players at this table`, `How to play`, `Close rules`, `Search the rules`, `Review a hand` / `Close trick review`, `Euchre home`, `Main navigation`, `Mute table sound` / `Unmute table sound`, loading `status` region, error `alert` regions, plus the screen-reader announcement stream in section 10. Min target 44x44. Focus ring: 3px `#87ac49` (cream `#f5d98e` on felt, `#295744` on rack), offset 4-6.
