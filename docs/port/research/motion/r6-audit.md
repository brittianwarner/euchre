# R6 - Audit of the desktop app's look and motion

Scope: `apps/desktop` as built today (read-only), the web app's real animations, and a frank visual critique.
Built with `CARGO_TARGET_DIR=<scratch>/audit-target cargo build -p euchre-desktop` (binary name is `euchre`, not `euchre-desktop`).
Rune citations: pinned rev `85cd8c6b` at `/Users/brittianwarner/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/crates/...` unless marked HEAD (`0ea75abe`, scratchpad `rune-src/rune-head`). For every `el/*.rs` API I list below, HEAD has the identical file:line, so nothing here depends on the rev. App paths are relative to `/Users/brittianwarner/goods/euchre-rune/apps/desktop/src`.

Scratch = `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad`.

## 0. Snapshots (kept)

All under `<scratch>/audit/`. Default window is 1280x820 at scale 2 (2560x1640 PNG); `-min` is `--size 720x520` (1440x1040 PNG). Scenes: `scenes.rs:28-37` (home, new, table-deal, table-bid, table-play, table-trick-won, table-hand-scored, game-over).

- `home.png`, `home-min.png`
- `new.png`, `new-min.png`
- `table-deal.png`, `table-deal-min.png`  (this is the round-1 bid with the up card, not a deal moment)
- `table-bid.png`, `table-bid-min.png`  (round 2, up card turned down)
- `table-play.png`, `table-play-min.png`
- `table-trick-won.png`, `table-trick-won-min.png`
- `table-hand-scored.png`, `table-hand-scored-min.png`
- `game-over.png`, `game-over-min.png`
- Motion evidence: `play-strip.png` (9 frames, 4 frames apart at 60 fps, from 0.5 s, after clicking `hand-JS` then `hand-play`), `play-flight-mid.png`, `play-flight-end.png`; script `scripts/play.json`.
- Layout numbers: `bid-tree.json`, `play-tree.json`, `play-min-tree.json`, `new-min-tree.json` (from `--tree`; no warnings in any).

Commands: `$B --scene NAME --snap out.png`, `... --size 720x520 --snap ...`, `--scene table-play --script scripts/play.json --strip out.png --crop 0,60,1280,760 --start 0.5 --frames 36 --every 4 --cols 3 --scale 1`. A scene plus a script works and drives the offline sim: clicking the rack card then `hand-play` flies the card and the sim answers. Use this for every motion check (no GPU window needed).

## 1. Motion that exists today

Tokens: `theme/felt.rs` `timing` (PLAY 0.7, COLLECT_AFTER 2.2, COLLECT 0.78, COLLECT_STAGGER 0.045, DEAL_IN 0.58, DEAL_STAGGER 0.075, DEAL_OFFSET -24, LIFT 0.18, PICKUP 0.85, ILLEGAL 3.2, BANNER 4.5, CONFETTI 2.4, CONFETTI_STAGGER 0.07, THINKING 1.2). `theme.rs:689-698` has FLIGHT 0.45/0.12, PLAY 0.7/0.1, UI 0.35/0, TOAST_SECS 4.

| Moment | How it is built | Duration / curve | Look in the strip |
|---|---|---|---|
| Your card to trick | `matched("hcard-{hand}-{card}")` on rack art (`table/hand.rs:164-168`) to trick slot (`stage.rs:732-733`) with `.motion(PLAY)` = `Spring::new(0.7, 0.1)` (`stage.rs:52`) | 0.7 s spring, bounce 0.1 | straight line, no arc, no scale pop, no overshoot lift. Draws above the result text and over the partner's card while flying (`play-strip.png` rows 2-3) |
| Opponent card to trick | `matched("play-{hand}-{trick}-{seat}")` on the top back of the seat fan (`stage.rs:558-590`, fan backs are 14x20pt, `FAN` const) to the trick card | same spring | grows from a 14x20 back to a 125x163 face along a straight line; face shown throughout (cookbook 15 #17) |
| Up card to dealer | `matched("pickup-{hand}")` to the dealer fan back (`stage.rs:67-74`, `:776-780`), `PICKUP` = `Spring::new(0.85, 0.05)` | 0.85 s | straight line; also round 2 deck tile takes the up card's id so the turned-down card settles into the deck (extra vs web) |
| Trick collect | per card: `.matched(play_id)` on a 24x34 mini in the winner's stack (`stage.rs:620-632`), `COLLECT = Spring::new(0.78, 0)`; `Presentation::poll` moves one card per 45 ms (`present.rs:~395-415`) | 0.78 s + 45 ms stagger, waits 2.2 s in auto mode (`COLLECT_AFTER`) or Continue in review | straight line, shrink, opacity to 0 (`MINI_REST` 0) |
| Deal-in (your five) | `.transition(offset(0,-24) + scale(0.92) + opacity, springs(DEAL, 0.15))` + `.enter_delay(i*0.075)` (`hand.rs:222-230`), only `i < 5` and only when `deal` is set (never first view, never discard) (`present.rs` `after_change`) | 0.58 s in, 0.15 s out, 75 ms stagger | cards drop 24pt and fade in. Opponents' fans, the up card, deck, dealer chip just appear |
| Hover / selection lift | `.offset(0, -4 / -8)` + `.offset_motion(LIFT)` = `Spring::new(0.18, 0)` (`hand.rs:155-168`); selected adds `outline(3,3,focus_felt)` + `SELECTED` shadow | 0.18 s | works |
| Press feedback | `.press_scale(0.98)` on buttons (`hand.rs:349`, `:430`, `stage.rs` continue, `screens/mod.rs:45`, `home.rs:216`) | spring default | works |
| Score numeral | `Transition::offset(0,10) + opacity` on the keyed numeral (`header.rs:111-126`) | default | extra vs web (web has none) |
| Banner | `Transition::offset(0,-12) + opacity`, shadow `BANNER`, 4.5 s (`stage.rs` `banner`, `table/mod.rs:140-165`) | default 0.2 s | works; position is wrong, see 4 |
| Confetti | 24 pieces `LoopSpec::slide` 2.4 s `Easing::EASE_IN`, delay `(i%6)*0.07`, static `rotate`, `.transition(opacity)` (`stage.rs:957-985`) | 2.4 s | falls in a straight line, no spin, loops until `confetti_until` |
| Thinking dot | `LoopSpec::opacity(1.2, 0.0, 0.25, 1.0)` (`stage.rs:491`) | 1.2 s | works |
| Settings rail / scrims / dialogs | `Transition::offset(RAIL_W,0)` or `offset(32,0)+opacity` (`panels.rs:549-553`), dialogs `offset(0,12)+opacity` (`:645-647`) | default | works |
| Reduce motion | `cx.reduced_motion(...)` from prefs or OS (`app.rs:870-872`); lifts zeroed, transitions `none`, confetti dropped | n/a | good |
| Rack slot memory | `RackSlots` keeps a hole for a played card (`present.rs:30-75`) | n/a | good: the other card does not slide to centre |

Instant today (no motion at all):
- Route changes: `navigate` only sets `tx.motion(UI)` (`app.rs:324-337`); no screen enter/exit transition anywhere in `screens/*.rs` (grep for Transition/enter_delay finds only table header/hand/panels/mod). Home -> New -> Table are hard cuts.
- Whole hand-start: opponents' fans, up card, deck, dealer chip, trump label, seat names mount in place. No shuffle, no deal from the dealer, no turn-up of the up card.
- Trump named: no event at all (no flash on the trump line, no glyph pop, no maker chip pop).
- The "your turn / their turn" seat tint (`SEAT_TINT #ffffff0a`, ~4% white) is effectively invisible; the active seat has no glow or pulse beyond a name colour.
- Result line swap ("You win the trick." etc.): text swaps with no transition (`stage.rs:846-912`, `trick-result-text` has no `.transition`).
- Heading and instruction swaps in the hand area ("Your turn." to "Sending your move..." to "Make yourself at home."): hard text swap; in the strip it flips the same frame the press lands.
- Winner ring appears with the 4th card (`stage.rs:742-744`), so when you play the last card the ring and "Right opponent wins the trick." show in frame 1 while your card is still in the rack (`play-strip.png` row 2 col 1). Spec 6.4 wants the ring after the 0.7 s flight.
- Hand score: header numerals slide, nothing else. `Step::HandScored` only makes a banner for lone march and game won (`copy::banner_for`; same as web 06 section 9).
- Score pips (ScoreCards) change opacity only.
- Disabled/enabled bid buttons: opacity step only.
- Buttons have `.hover(color)` but no lift (web `.deal-button:hover` has translateY(-2px), `routes/+page.svelte:200-206`).

## 2. What the web does that the desktop does not

Web files: `apps/web/src/lib/ui/` (TableStage.svelte, CardRack.svelte, GameFeedback.svelte, GlobalNav.svelte), `routes/play/[gameId]/+page.svelte`, `routes/+page.svelte`. The shipped web is modest. `lib/three/*` (Tween, cubicOut) is NOT mounted (06 section 16); ignore except its TEMPO numbers (deal flight 180 ms, stagger 50, play 560, trick sweep 680/stagger 80, winner pulse delay 150, hand-score 600/counter 450, celebration 1400/2400, up-card reveal 260, game won 2400) as a pacing reference.

Parity gaps (web does it, desktop does not):
1. Play-flight keyframes, `TableStage.svelte:99-108`: 0% `translate(dx,dy) scale(s) rotate(R)` opacity 0.4, 82% `translate(0,-8px) scale(1.025) rotate(1deg)`, 100% rest; 700 ms `cubic-bezier(.22,.7,.25,1)`. Others start at 0.75 scale with R = -6 deg. Desktop: one spring, no start opacity, no -6 deg start (matched flights do not carry rotation, cookbook 15 #17), no lift-and-settle.
2. Up-card pickup, `TableStage.svelte:128-141`: 850 ms, smoothstep, arc `-sin(p*PI)*24`, scale to 0.28, rotate 8 deg, opacity min(1, t*5) (opaque 80%, then fades). Desktop: straight line, no rotate, opacity follows the destination.
3. Collect, `TableStage.svelte:157-173`: 780 ms, 45 ms stagger, `cubic-bezier(.4,0,.2,1)`, 60% waypoint `translate(.55dx,.55dy-20px) scale(.7) rotate(-5deg)`, end `scale(.18) rotateY(90deg)` opacity 0. Desktop: straight line spring; no -20 px arc, no rotation, no flip (no 3D transform exists in `el/visuals.rs`: only `rotate`, `scale`, `origin`).
4. Selected card tilt `perspective(1000px) translateY(-8px) rotateX(2deg)` (`CardRack.svelte:237-238`): desktop lifts only (rotateX impossible; skip).
5. Deal-in `rotateY(18deg)` (`CardRack.svelte:61`): desktop has the rest of it.
6. Quip/banter arrive 180 ms ease-out `translateY(4px)` (`TableStage.svelte:429-441`): desktop result row has no transition.
7. Confetti `translateY(100vh) rotate(540deg)` + fade (`GameFeedback.svelte:104-110`): desktop does not spin.
8. Home `.deal-button` hover `translateY(-2px)`, 200 ms (`routes/+page.svelte:200-206`) and nav hover 160 ms (`GlobalNav.svelte:61`): desktop colour only.
9. Settings rail 320 ms `cubic-bezier(.22,1,.36,1)` width + panel `translateX(32px)` and 180 ms opacity (`play/[gameId]/+page.svelte:1174-1200`): desktop approximates with springs; fine.
10. Reconnect dedupe (pre-seen play keys do not fly, `TableStage.svelte:62-65`): desktop equivalent exists (`seen_view`, `RackSlots`, `after_change`), but check that `tx.instant()` is used for resync snapshots (cookbook 6); not verified in `app.rs`.
11. Sound (`lib/ui/sound`): desktop queues `Cue`s (`present.rs` Hooks) but no lane plays them (D14).

Desktop extras vs web: round-2 deck takes the up card, score numeral slide, thinking dot, hole-keeping rack, hover lift on bid buttons is absent on both.

Bottom line: the desktop is at near parity with the web on the motion the web actually ships (every one of the 6.2 to 6.5 items has a Rune equivalent). The gap to "badass" is not parity, it is new choreography: staging of the deal, trump reveal, trick win, hand score and match end, plus easing quality (curves) and the arcs the web has but the straight matched path cannot do.

## 3. Rune hooks to close the gaps (pinned rev; HEAD identical lines)

- Tween with a real cubic-bezier for offset/scale/rotate: `offset_motion(impl Into<Anim>)` (`el/motion.rs:122`), `scale_motion` (`el/visuals.rs:369`), `rotate_motion` (`:389`) accept `Curve::Tween { duration, easing }` (`rune-motion/src/curve.rs:11-12`, `From<Curve> for Anim` `:137`). `Easing::cubic(x1,y1,x2,y2)` (`easing.rs:41`), presets `EASE`, `EASE_IN`, `EASE_OUT`, `ENTER = cubic(.2,.8,.2,1)`, `EXIT` (`easing.rs:27-34`). Use `Curve::Tween{ duration: 0.18, easing: Easing::EASE }` for hover lift (spring has no CSS-like shape), and `cubic(.22,.7,.25,1)` for the 700 ms play to match the web exactly. All re-exported from `rune` (`rune/src/lib.rs:259-277`).
- Sequence (one-shot keyframes) is opacity + uniform scale only (`rune-motion/src/sequence.rs:5-22`, `Beat::new(at).opacity().scale().easing()`); `El::sequence` (`el/motion.rs:259`). No translate/rotate beats. Use it for trump-glyph pop, score pulse, winner ring pulse. Replay by changing the element key (`sequence.rs:24-25` comment).
- Loops (`el/motion.rs:264`): `LoopSpec::opacity/rotation/slide/sequence/cycle` (`looping.rs:26-50`). A node takes one spec. For confetti spin put `LoopSpec::rotation(period)` on a wrapper and the slide on its child.
- Transition constructors: `opacity, scale, scale_anchor, offset, move_edge, push, slide, blur, blur_replace, reveal, draw, asymmetric, combined, springs` (`rune-motion/src/transition.rs:186-261`). `blur_replace` is a ready text-swap transition for heading/result line swaps; `reveal(edge)` and `scale_anchor` suit a banner or hand-result card. `.enter_delay(s)` (`el/enter_delay.rs:36`) staggers.
- Depth: `box_shadow` (`el/visuals.rs:148`), `blur` (`:248`), `backdrop_blur(sigma)` (`el/backdrop.rs:34`) for a frosted header/rail/dialog; multi-stop gradients (cookbook 9).
- Tokens `POP = Spring::new(0.25,0.15)`, `MORPH 0.3/0.16`, `DRIFT 0.85` (`rune-motion/src/tokens.rs:27-33`) give shaped springs without hand-tuning.
- Motion gate: `MotionIntent::OVERSHOOT|KICK|RECOIL` (`el/motion.rs:7-45`) marks deliberate overshoot so `--trace`/`motion_gate` do not flag it. Our `Spring::new(0.7, 0.1)` bounce on a flight is an overshoot; if a motion check is ever added, mark card flights with `.motion_intent(MotionIntent::OVERSHOOT)` (builder named in the doc comment at `el/motion.rs:1`; confirm the method name before use).
- `Tx::launch(source, dest)` exists at the pinned rev (`el/tx.rs:44-45`, `runtime/motion/flights.rs:1-9`) but it flies a text source to a destination; not a card tool. HEAD's 26 newer commits touching "launch" (`9e537376`, `bc0d36fc`) only restructure these phases (`runtime/motion/flights.rs` HEAD) and do not add an arc API; there is still no path/arc API on `matched`.
- Arc workaround (no API change): a flight is straight in screen space, but `offset`/`offset_motion` springs on the destination are separate lanes. Put the destination under a wrapper that has `.offset(0, -h)` springing to `0` over the flight duration with a bouncy spring to fake a lift-and-drop; accept that it reads as a bounce on arrival, not a mid-flight arc. Unverified; needs a strip test.
- Rotation does not ride a matched flight (cookbook 15 #17), so tilt the destination and let `.rotate_motion(Spring)` settle it from a seeded start (set `.rotate(start)` in the commit before, `.rotate(rest)` in the next with `tx.wake_after`), or accept landing already rotated. Unverified.

## 4. Visual critique by screen

All pixel references are in 2560x1640 PNGs viewed at 2000 px wide; points are layout points from `--tree`.

### Home (`home.png`, `home-min.png`)
Good: strongest screen. Serif headline with lime "hand.", fanned cards, the `THE RIGHT J` stamp, quiet cream shell.
- Hierarchy: the dealer "D" token (top right of the hero art) is a flat grey blob; it reads as a render glitch next to real cards. Make it lime or cream with a drop shadow, or delete it.
- Contrast: `HERO_NOTE #a8bcb0` and the footer strip captions at 12-13 px on `#153f36` are marginal. "Good company. Better cards." caption floats at the art's lower right with no anchor.
- Depth: the radial vignette behind the cards is a soft grey-green disc; add a slightly cooler/darker rim so it stops looking like a smudge.
- Empty space: left column ends at y~930/1281, 200 px of dead hero below the CTA before the strip. Move the CTA and lede up or enlarge the art.
- Min size (720x520): the viewport shows the nav, eyebrow and headline only; art, lede and the "Let's play" button are below the fold. A first-time user at minimum size sees no call to action. Needs a compact hero (hide art, pull CTA up) under ~600 pt tall.
- Motion: none on load. Hero cards could fan in (stagger, Transition offset + rotate settle); CTA could lift on hover (web does).

### New game (`new.png`, `new-min.png`)
- Cheap: two flat checkboxes on cream, a 640pt column left-aligned in a 1280 window, a full-width forest button 90 px below the last option. No preview of what a table looks like. The global nav pill hangs in the top-right corner 8 px from the edge and 15 px from the top, floating unrelated to the column.
- Contrast: option descriptions `#58695b` at 17 px fine; checkbox outline `#bdc9b2` is very faint.
- Fix: make each option a selectable card (panel fill, hairline, hover lift), put a mini felt preview or rules summary beside it, keep the button near the options. 
- Min size: `new-panel` is 641.6 tall (`new-min-tree.json`), "Deal me in" at y 541-597 and the footer at 621 are off the 520 viewport. I did not verify the page scrolls; if it does not, the primary action is unreachable at minimum size.

### Table, bidding (`table-deal.png`, `table-bid.png`, `-min`)
Layout numbers at 1280x820 (`play-tree.json`): header 0-64, felt rail 72-460 (388 pt, 47% of the height), hand area 468-812 (344 pt).
- Felt is a 1232x388 letterbox (3.2:1). The player's actual work area is the bottom half, which is a flat `#17452f` slab with a left-aligned 35 px serif heading and a one-line instruction. Two different greens (flat page vs felt panel) make it read as "a panel on a screen", not "a table". Extend the felt (or a darker vignette) under the rack so cards sit on cloth.
- Card sizing: rack art is 98x137 (`card-JS`), but trick cards are 116-126x156-163 and the up card ~110x150. Your own cards, the main interactive objects, are smaller than the ones on the table, inside a 344 pt panel that is mostly empty. Raise `HAND_W` or let the rack grow into the empty 80-100 pt; the rack could be 130+ wide at this size.
- Caption under every rack card ("Ace of diamonds" 16 px) is redundant with the face and adds noise and a second baseline row; keep captions for a11y only (labels already exist) and show text only for state ("Selected", "Must follow suit").
- Wood rail: 14 pt flat brown gradient with a 1 px brass line and a second 1 px `felt-rim` line 12 pt inside; the double hairline looks like two borders fighting. No wood grain, no inner shadow, no highlight on the top edge, so it reads as a brown stroke not a rail. Add an inset shadow on the felt edge (felt looks pasted on) and a top highlight on the rail.
- Felt texture: spec 16.1 asks for deterministic speckle grain and a `♣`/euchre watermark; `FELT_WATERMARK` exists in `theme.rs:119` and `felt_watermark` is built (`:273`, `:385`) but is unused (grep finds no use in `screens/`). Grain tile is also not implemented. The felt reads as a smooth gradient = flat/cheap.
- Seats: text blocks, a 28 pt initial circle in a barely different green, "Partner · 5 cards", and a 44x20 fan of red 14x20 backs that look like a toy. No seat plate, no card count badge, no active-turn glow (`SEAT_TINT` 4% white is invisible; only the "You" box in the bid scene shows a flat rounded rectangle). Opponent fans are too small to carry the flight origin (a 14x20 back flies to a 125x163 face).
- Inconsistent card backs: hero uses a red guilloche back, seat fans use red, the deck tile is green with a club, and the taken-trick stacks are green tiles. Pick one back and use it for fan, deck and stacks.
- Taken-trick stack glyph (24x16 green tile with a diagonal stripe, `stage.rs:595-617`) reads as a small monitor/console icon, not a pile of cards; draw 2-3 offset card edges and show the count.
- Bug (bid, round 2, `bid-tree.json`): `bid-copy` is 137.5 pt tall (y 166-303.6) in a trick box 123.6 tall (y 173-296.6) so it overflows the box and overlaps `felt-you` (y 296.6) - visible in `table-bid.png` where "another suit." sits on the "You" box. The eyebrow "A seat at the table" (the loading-screen headline, 06 section 14.1) collides with Ned's "Partner - Passed" line. Copy is also redundant: "Your turn." under the felt repeats the "You - Your turn" box. Shorten or drop `bid-line` and drop the "You" block text when the hand heading already says it.
- Buttons: Pass (white), Spades/Hearts/Diamonds (cream), all same weight; "Make diamonds trump" (round 1) is cream like the rest. There is no primary. Go alone is a hollow dark box with no hover affordance. The control row spans the full 1068 pt but the buttons sit in the middle with the checkbox hanging off the right.
- Header: a hard cream 64 pt band over green. ScoreCards are two 24x34 mini cards per team with unreadable 8.5 pt pips; "How to play" and "Settings v" are heavy outlined cream boxes that out-weigh the scoreboard. "First to 10" is `#596952` on cream, low contrast. At minimum (720 wide) the Settings button starts at x=629 with width 112.5 (`play-min-tree.json`), ends at 741.5, so it is clipped 21.5 pt off the window (`table-*-min.png`).
- Min size: felt 696x236, rack cards 51x72 (`card-JS`), trick cards 60x78. The cards you play with are tiny and the rack sits in 105 pt with ~100 pt of dead green around it. The seat names collapse to "Gr..." / "Gran..." even though the seat has room (also at 1280, see below).

### Table, play (`table-play.png`)
- Trick area works: four tilted cards with layered shadows read well and the "Led" pill is clear. Hierarchy problem: the ring/outline on the winner is the only emphasis; your own card slot is lower and overlapping the right opponent's card (card overlap order follows play order).
- The Ace of spades face shows `www.me.uk/cards/` and a QR code (asset attribution baked into the art; visible in `table-play.png`, `table-play-min.png`, `table-trick-won.png`, `game-over.png`). Re-export that card without the watermark or mask it. It is the single cheapest-looking thing on screen.
- "Gran..." name truncation (`table-trick-won.png`, `table-hand-scored.png`): the right seat packs name + maker chip + lead arrow + dealer chip into `NAME_MAX_W` so the name truncates when 100 pt of space is free. Let the chips wrap under the name or widen the block.
- Empty space: between the left seat (x 180-310) and the first trick card (x 630) and right of the last card there are ~330 pt gaps each side; side seats are small text islands far from the action. Pull the side seats toward the trick or use their space for larger cards.
- "Play selected card" is a permanently visible grey disabled button, 52 pt tall. Disabled state looks dead. Hide until a card is selected or fold selection into double-click/Enter and show a hint.
- Flight (`play-strip.png`): the card leaves its slot, passes over "Right opponent wins the trick." text and over the partner's card (z above everything), lands without a settle. The caption under the card ("Jack of spades / Selected") fades while the card is already moving. See section 1 for the winner ring and heading timing.

### Trick won / review (`table-trick-won.png`)
- Result row with Continue is the best-designed piece: bold gold lead-in, plain follow-up, cream button. But the rack under it still shows full-size cards, a "Review the trick." heading and a dead "Play selected card" button. During review the bottom 344 pt has nothing to do; use it (replay the trick, show points, enlarge Continue) or dim the rack.
- Continue is cream-on-felt like every other button; it should be the one clear primary (lime/blue accent) here.

### Hand scored (`table-hand-scored.png`)
- Weakest screen. The bottom 40% is flat empty green with "Make yourself at home." (an idle placeholder heading, wrong for a scoring moment) and "Hand complete. Adding the points..." The scoring itself is only a header numeral change 0 to 1. There is no hand-result card (who made it, points, tricks), no score-delta pulse, no ScoreCards animation. This is where the most visible win is available.
- The felt still shows the finished trick and the cards-to-zero fans; fine as backdrop but dim it.

### Game over (`game-over.png`)
- The banner "A good game. Well played." is placed at y~1050/1281, inside the hand area under three other pieces of text that say the same thing (hand heading "A good game, well played.", instruction "The other team wins the match."); the felt still says "Hand 12 / Clubs are trump / Left opponent wins the trick." The end of a match needs its own composed state: final score large, team names, Play again as the primary, felt dimmed. Currently the "Play again" button is a small cream button at the far right of the heading row. `BANNER_TOP` is 64 (06 says 104 from the top); at 1280x820 the banner in the snapshot sits low instead, check that `BANNER_TOP` is honoured by the pin (`stage`/`table/mod.rs` `banner_layer` is passed into `hand_area`, so it centres in the hand area, not at 64).
- Win vs loss: no confetti on loss is per spec, but a loss has no tone at all.

## 5. What to build (ordered by visible payoff per effort)

1. Hand-score moment: result card (hand number, "Your team made it", +N points, tricks) with `Transition::scale_anchor`/`reveal`, score ScoreCards pip fill with a `Sequence` pulse, header numeral roll. Replace the "Make yourself at home." heading with a scoring one. Large, cheap, high-visibility.
2. Deal choreography: on a new hand, stagger the dealer's deal to four fans (backs `enter_delay`), then the up card turning over (`Sequence` scale 0.0 to 1.0 on a back to face swap, since no rotateY), dealer chip pop. Dim/lift the seat that is to act.
3. Play-flight polish: Tween with `cubic(.22,.7,.25,1)`, delay the winner ring and result line until the flight lands (state: stamp a `landed_at` in `Presentation::poll`), add a short settle (`Sequence` scale 1.0 to 1.03 to 1.0 on the landed card), and a landing shadow squash. Check `play-strip.png` rows 2-3 again after.
4. Trick win: pulse the winner ring (`Sequence` opacity), glow on the winning seat, collect with a lift (offset spring) before the sweep, the stack shows a count that ticks up.
5. Trump called: glyph pop on the trump line and a one-shot sweep across the felt in the suit colour (`Sequence` opacity on a pinned overlay), maker chip pop (`Spring POP`).
6. Match end: composed end panel (see section 4), confetti with spin (nested `LoopSpec::rotation` + `slide`), loss tone state.
7. Screen transitions: `Transition::push`/`slide` or `opacity` + `offset(0,12)` on the route root so Home/New/Table do not hard-cut; the table entering with the felt scaling in (`scale_anchor`).
8. Type-swap polish: `Transition::blur_replace` or opacity on `trick-result-text`, `hand-h1`, `hand-instruction` so heading/instruction changes do not hard-cut (`play-strip.png`).
9. Look fixes with no motion: bigger rack cards, kill rack captions, unify card backs, felt grain + watermark (pre-generate the 512 tile from 06 section 16), inset shadow + wood rail highlight, seat plates and an active-turn glow, bid hierarchy (one primary), fix bid-area overflow, fix header overflow at 720, remove the Ace-of-spades watermark, compact hero and sticky submit at small heights.

## 6. Verification notes and caveats

- Nothing here was run with a GPU window; snapshots are headless (`--snap`), strips use `--strip`. All numbers come from `--tree`.
- `table-deal` is a bidding frame (the scene fixture), not an animated deal; there is no scene that captures the deal-in. To see it, add a scene or script that sets `present.deal_in` (not done; apps/desktop is read-only for me).
- The overlap at the bid scene and the min-size Settings clip are measured from the tree, not just seen.
- I did not verify whether `new` and `home` scroll at 720x520, whether `tx.instant()` is used on resync snapshots, or any item marked "unverified" above.
- The lifted card in `play-strip.png` draws above the felt result text because flights draw above the window (cookbook 5.1); that is expected, but it covers the result line mid-flight. Moving the result row below or fading it during flights avoids it.
