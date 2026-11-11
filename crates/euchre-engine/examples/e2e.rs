//! Independent end-to-end harness for the euchre engine: a port of the TS
//! `scripts/e2e-play.ts`.
//!
//! Deliberately NOT written against the unit tests' assumptions: it drives the
//! public API the way a Rivet actor will, and re-derives every invariant from
//! the rules of euchre rather than from the implementation.
//!
//! What counts as SECRET here is precise, and follows the spec:
//!   - another seat's hand                                  (never projected)
//!   - kitty[1..3], the three buried cards                  (never projected)
//!   - the dealer's discard                                 (never projected, V12)
//!
//! The up-card (kitty[0]) is PUBLIC for the whole hand (V14) and stays public
//! even after the dealer takes it into hand, so it is excluded from the secret
//! set. `kitty` is the immutable dealt packet, so the up-card legitimately
//! appears both there and in the dealer's hand: representation, not duplication.
//!
//! ```text
//! cargo run -p euchre-engine --example e2e                 # narrate one game, then fuzz 300
//! cargo run -p euchre-engine --example e2e -- 1000 quiet   # fuzz only
//! ```
//!
//! `tests/e2e.rs` includes this file as a module and calls [`run`] with a
//! small game count, comparing the output with the TS script's.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::time::Instant;

use euchre_engine::{
    Card, DECK, GameInit, GamePhase, GameState, GameStatus, HandResult, HandState, LegalMove, Move,
    Rank, Seat, Suit, Trick, advance, apply, assert_invariants, assert_legal_non_empty,
    create_game, effective_suit, legal_moves, mulberry32, plain_rank, project, trump_rank,
};

/// `println!` into the harness's output sink. Writing to a `String` or to
/// stdout cannot fail in a way the harness could act on, so the result is
/// dropped.
macro_rules! say {
    ($out:expr) => {{
        let _ = writeln!($out);
    }};
    ($out:expr, $($arg:tt)*) => {{
        let _ = writeln!($out, $($arg)*);
    }};
}

/// Streams the harness output to stdout (what `main` uses).
struct Stdout;

impl fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        print!("{s}");
        Ok(())
    }
}

/// The fuzz harness's random source: `mulberry32` as a boxed closure.
type Rng = Box<dyn FnMut() -> f64>;

const SEATS: [Seat; 4] = Seat::ALL;

/// Every trick, each exactly once.
///
/// During `trick_resolve` the engine deliberately leaves `hand.trick` pointing
/// at the trick it has *already* pushed to `trick_log`, so the UI can render the
/// completed trick before the collection animation. Naively concatenating the
/// two double-counts four cards, so dedupe on the trick index.
fn all_tricks(h: &HandState) -> Vec<&Trick> {
    let mut out: Vec<&Trick> = h.trick_log.iter().collect();
    let cur = &h.trick;
    if cur.plays.is_empty() || h.trick_log.iter().any(|t| t.index == cur.index) {
        return out;
    }
    out.push(cur);
    out
}

/// Distinct failure messages, with identical repeats collapsed.
#[derive(Default)]
struct Failures {
    items: Vec<String>,
    seen: HashSet<String>,
}

impl Failures {
    fn check(&mut self, cond: bool, msg: impl FnOnce() -> String) {
        if cond {
            return;
        }
        let msg = msg();
        // Collapse identical repeats so one systemic bug doesn't drown the rest.
        let key: String = strip_seed(&msg).chars().take(120).collect();
        if self.seen.insert(key) {
            self.items.push(msg);
        }
    }
}

/// `msg.replace(/seed=\S+/, '')`: drops the first `seed=<non-space>` run.
fn strip_seed(msg: &str) -> String {
    let Some(start) = msg.find("seed=") else {
        return msg.to_owned();
    };
    let rest = &msg[start + 5..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    if end == 0 {
        return msg.to_owned();
    }
    format!("{}{}", &msg[..start], &rest[end..])
}

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| panic!("serialize: {e}"))
}

/* ------------------------------------------------------------------ security */

/// THE security invariant. Serialise what a seat is allowed to see and assert no
/// secret card id appears anywhere in it, at any depth, under any key name.
/// Substring matching is intentional: it catches a leak through a field nobody
/// thought to audit.
fn check_redaction(s: &GameState, f: &mut Failures, tag: &str) {
    let h = &s.hand;
    let up_card = h.up_card; // public by design: excluded from every secret set

    for seat in SEATS {
        let blob = json(&project(s, seat));

        // Insertion-ordered like a JS Map; re-setting a key keeps its slot.
        let mut secrets: Vec<(Card, String)> = Vec::new();
        let mut set = |c: Card, where_: String| {
            if let Some(slot) = secrets.iter_mut().find(|(k, _)| *k == c) {
                slot.1 = where_;
            } else {
                secrets.push((c, where_));
            }
        };
        for other in SEATS {
            if other == seat {
                continue;
            }
            for &c in &h.hands[other.index()] {
                if Some(c) != up_card {
                    set(c, format!("seat {other}'s hand"));
                }
            }
        }
        for &c in &h.kitty[1..] {
            set(c, "buried kitty".to_owned());
        }
        if let Some(d) = h.dealer_discard
            && Some(d) != up_card
        {
            set(d, "dealer's discard".to_owned());
        }
        // A card already played to the table is public knowledge, so it cannot
        // be a secret even if a stale copy lingers in a hand array.
        for t in all_tricks(h) {
            for p in &t.plays {
                secrets.retain(|(k, _)| *k != p.card);
            }
        }

        for (c, where_) in &secrets {
            if blob.contains(&format!("\"{c}\"")) {
                f.check(false, || {
                    format!(
                        "{tag}: seat {seat}'s view leaks {c} ({where_}) in phase {}",
                        h.phase
                    )
                });
            }
        }
    }
}

/// Every one of the 24 cards accounted for exactly once, up-card aliasing aside.
fn check_card_conservation(s: &GameState, f: &mut Failures, tag: &str) {
    let h = &s.hand;
    if h.phase == GamePhase::Lobby || h.phase == GamePhase::Cutting {
        return;
    }

    let in_hands: Vec<Card> = SEATS
        .iter()
        .flat_map(|i| h.hands[i.index()].iter().copied())
        .collect();
    let played: Vec<Card> = all_tricks(h)
        .iter()
        .flat_map(|t| t.plays.iter().map(|p| p.card))
        .collect();
    let discard: Vec<Card> = h.dealer_discard.into_iter().collect();

    // A card is either still held, already played, buried, or discarded.
    let universe: HashSet<Card> = in_hands
        .iter()
        .chain(&played)
        .chain(h.kitty.iter())
        .chain(&discard)
        .copied()
        .collect();
    f.check(universe.len() == 24, || {
        format!(
            "{tag}: {} distinct cards tracked, expected 24",
            universe.len()
        )
    });

    // Nothing may be in two live places at once. The up-card is the one legal
    // alias (immutable kitty[0] + the dealer's hand after a pickup).
    let live: Vec<Card> = in_hands
        .iter()
        .chain(&played)
        .chain(&discard)
        .copied()
        .collect();
    let mut seen = HashSet::new();
    let mut dupes: Vec<Card> = Vec::new();
    for &c in &live {
        if !seen.insert(c) && !dupes.contains(&c) {
            dupes.push(c);
        }
    }
    for d in dupes {
        f.check(false, || {
            format!(
                "{tag}: {d} is in two live places at once (phase {})",
                h.phase
            )
        });
    }

    // Hand sizes: 5 each, except mid-pickup (dealer 6) and as tricks are played.
    if h.phase == GamePhase::TrickPlay || h.phase == GamePhase::TrickResolve {
        let mut played_by_seat = [0usize; 4];
        for t in all_tricks(h) {
            for p in &t.plays {
                played_by_seat[p.seat.index()] += 1;
            }
        }
        for seat in SEATS {
            if Some(seat) == h.sitting_seat {
                continue;
            }
            let expect = 5usize.saturating_sub(played_by_seat[seat.index()]);
            let actual = h.hands[seat.index()].len();
            f.check(actual == expect, || {
                format!("{tag}: seat {seat} holds {actual}, expected {expect}")
            });
        }
    }
}

/// Re-derive follow-suit legality from the rules, independently of `legal.rs`.
fn check_follow_suit(s: &GameState, f: &mut Failures, tag: &str) {
    let h = &s.hand;
    let Some(trump) = h.trump else {
        return;
    };

    // Reconstruct each seat's holdings at the moment of each play, by replaying
    // the hand forward from what they were dealt (still-held + everything played).
    let mut remaining: [HashSet<Card>; 4] = Default::default();
    for seat in SEATS {
        remaining[seat.index()].extend(h.hands[seat.index()].iter().copied());
    }
    for t in all_tricks(h) {
        for p in &t.plays {
            remaining[p.seat.index()].insert(p.card);
        }
    }

    for t in all_tricks(h) {
        let Some(first) = t.plays.first() else {
            continue;
        };
        let led = effective_suit(first.card, Some(trump));
        for (i, p) in t.plays.iter().enumerate() {
            let held = &mut remaining[p.seat.index()];
            if i > 0 && effective_suit(p.card, Some(trump)) != led {
                let could_have_followed =
                    held.iter().any(|&c| effective_suit(c, Some(trump)) == led);
                f.check(!could_have_followed, || {
                    format!(
                        "{tag}: seat {} played {} off-suit but still held the led suit {led}",
                        p.seat, p.card
                    )
                });
            }
            held.remove(&p.card);
        }
    }
}

/// Recompute every trick winner with an independent comparator.
fn check_trick_winners(s: &GameState, f: &mut Failures, tag: &str) {
    let h = &s.hand;
    let Some(trump) = h.trump else {
        return;
    };
    for t in &h.trick_log {
        let (Some(first), Some(winner)) = (t.plays.first(), t.winner_seat) else {
            continue;
        };
        let led = effective_suit(first.card, Some(trump));
        let mut best_seat = first.seat;
        let mut best: i32 = -1;
        for p in &t.plays {
            let eff = effective_suit(p.card, Some(trump));
            let v = if eff == trump {
                100 + i32::from(trump_rank(p.card, trump))
            } else if eff == led {
                50 + i32::from(plain_rank(p.card))
            } else {
                0
            };
            if v > best {
                best = v;
                best_seat = p.seat;
            }
        }
        f.check(winner == best_seat, || {
            let plays: Vec<String> = t
                .plays
                .iter()
                .map(|p| format!("{}:{}", p.seat, p.card))
                .collect();
            format!(
                "{tag}: trick {} winner {winner}, independent calc {best_seat} (led {led}, trump {trump}, {})",
                t.index,
                plays.join(" ")
            )
        });
        // The led suit the engine recorded must match the effective suit of play 0.
        f.check(t.led_suit == Some(led), || {
            format!(
                "{tag}: trick {} ledSuit {}, expected {led}",
                t.index,
                opt(t.led_suit)
            )
        });
    }
}

/// A loner's hand has exactly 3 plays per trick; a normal hand has 4.
fn check_loner_shape(s: &GameState, f: &mut Failures, tag: &str) {
    let h = &s.hand;
    let expect = if h.sitting_seat.is_none() { 4 } else { 3 };
    for t in &h.trick_log {
        f.check(t.plays.len() == expect, || {
            format!(
                "{tag}: trick {} had {} plays, expected {expect} (sitting={})",
                t.index,
                t.plays.len(),
                opt(h.sitting_seat)
            )
        });
        if let Some(sitting) = h.sitting_seat {
            f.check(!t.plays.iter().any(|p| p.seat == sitting), || {
                format!("{tag}: sitting seat {sitting} played in trick {}", t.index)
            });
        }
    }
}

/// JS string form of a nullable value.
fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or_else(|| "null".to_owned(), |x| x.to_string())
}

/* -------------------------------------------------------------------- driver */

/// What one match produced; compared whole for the determinism check.
#[derive(PartialEq, Debug)]
struct GameSummary {
    hands: usize,
    final_score: [u8; 2],
    winner: u8,
    steps: usize,
    phases_seen: BTreeSet<String>,
    move_ids_seen: BTreeSet<String>,
    hand_results: Vec<String>,
    loners: usize,
    euchres: usize,
}

const MAX_STEPS: usize = 6000;

/// Records a newly written `hand.result` (once per hand number).
fn note_result(
    s: &GameState,
    hand_results: &mut Vec<String>,
    euchres: &mut usize,
) -> Option<HandResult> {
    let result = s.hand.result?;
    let prefix = format!("{}:", s.hand.hand_no);
    if hand_results.iter().any(|r| r.starts_with(&prefix)) {
        return None;
    }
    hand_results.push(format!("{prefix}{result}"));
    if result == HandResult::Euchre {
        *euchres += 1;
    }
    Some(result)
}

fn play_game(
    seed: &str,
    rng: &mut Rng,
    f: &mut Failures,
    out: &mut dyn fmt::Write,
    narrate: bool,
) -> GameSummary {
    let mut s = create_game(GameInit {
        game_id: format!("g-{seed}"),
        seed: seed.to_owned(),
        cfg: None,
        internal_token: None,
        first_dealer: None,
    });
    let tag = format!("seed={seed}");

    let mut phases_seen = BTreeSet::new();
    let mut move_ids_seen = BTreeSet::new();
    let mut hand_results: Vec<String> = Vec::new();
    let mut steps = 0;
    let mut hands = 0;
    let mut loners = 0;
    let mut euchres = 0;
    let mut last_score = s.score;
    let mut last_hand_no: Option<u32> = None;
    let mut last_phase: Option<GamePhase> = None;
    let mut last_trick_count = 0;

    while s.status == GameStatus::Active && steps < MAX_STEPS {
        steps += 1;
        phases_seen.insert(s.hand.phase.to_string());

        // `hand.result` is written by advance() during the engine-only
        // hand_score phase, so sampling only after apply() misses every outcome.
        if note_result(&s, &mut hand_results, &mut euchres).is_some() && narrate {
            narrate_hand_end(out, &s);
        }

        let new_hand = Some(s.hand.hand_no) != last_hand_no && s.hand.phase == GamePhase::BidRound1;
        if narrate {
            if new_hand {
                last_hand_no = Some(s.hand.hand_no);
                hands += 1;
                narrate_hand_start(out, &s);
            }
            if last_phase == Some(GamePhase::BidRound1)
                && s.hand.phase != GamePhase::BidRound1
                && s.hand.trump.is_some()
            {
                narrate_trump(out, &s);
            }
            let done = s.hand.trick_log.len();
            if done > last_trick_count {
                if let Some(t) = s.hand.trick_log.last() {
                    say!(out, "        → {} takes it.", seat_name_opt(t.winner_seat));
                }
                last_trick_count = done;
            }
        } else if new_hand {
            last_hand_no = Some(s.hand.hand_no);
            hands += 1;
        }
        last_phase = Some(s.hand.phase);

        let found = SEATS.iter().find_map(|&seat| {
            let m = legal_moves(&s, seat);
            (!m.is_empty()).then_some((seat, m))
        });

        let Some((actor, moves)) = found else {
            let r = advance(&s);
            if r.state == s {
                f.check(false, || {
                    format!(
                        "{tag}: advance() made no progress in phase {}",
                        s.hand.phase
                    )
                });
                break;
            }
            s = r.state;
            continue;
        };

        let also_acting: Vec<String> = SEATS
            .iter()
            .filter(|&&x| x != actor && !legal_moves(&s, x).is_empty())
            .map(ToString::to_string)
            .collect();
        f.check(also_acting.is_empty(), || {
            format!(
                "{tag}: seats {actor},{} can act at once",
                also_acting.join(",")
            )
        });
        f.check(Some(actor) == s.hand.turn_seat, || {
            format!(
                "{tag}: legalMoves offered seat {actor}, turnSeat is {}",
                opt(s.hand.turn_seat)
            )
        });

        let pick = moves[pick_index(rng(), moves.len())].clone();
        move_ids_seen.insert(pick.id.clone());
        if pick.id.ends_with("+alone") {
            loners += 1;
        }

        if rng() < 0.05 {
            probe_illegal(&s, actor, &moves, f, &tag);
        }
        if narrate {
            narrate_move(out, &s, actor, &pick);
        }

        let before = s.clone();
        match apply(&s, actor, pick.mv, None) {
            Ok(r) => s = r.state,
            Err(e) => {
                f.check(false, || {
                    format!("{tag}: legal move {} was rejected: {e}", pick.id)
                });
                break;
            }
        }
        f.check(s != before, || {
            format!("{tag}: apply({}) returned the same state object", pick.id)
        });

        check_card_conservation(&s, f, &tag);
        check_redaction(&s, f, &tag);
        // Exercise the shared validator (spec section 8) alongside our
        // independent checks, so a bug in one is not masked by agreement with
        // the other.
        if let Err(e) = assert_invariants(&s).and_then(|()| assert_legal_non_empty(&s, legal_moves))
        {
            f.check(false, || format!("{tag}: assertInvariants threw — {e}"));
        }

        let d0 = i32::from(s.score[0]) - i32::from(last_score[0]);
        let d1 = i32::from(s.score[1]) - i32::from(last_score[1]);
        f.check(d0 >= 0 && d1 >= 0, || {
            format!(
                "{tag}: score went backwards {},{} -> {},{}",
                last_score[0], last_score[1], s.score[0], s.score[1]
            )
        });
        f.check(
            [0, 1, 2, 4].contains(&d0) && [0, 1, 2, 4].contains(&d1),
            || format!("{tag}: illegal delta {d0}/{d1}"),
        );
        f.check(!(d0 > 0 && d1 > 0), || {
            format!("{tag}: both teams scored on one hand")
        });
        last_score = s.score;

        check_follow_suit(&s, f, &tag);
        check_trick_winners(&s, f, &tag);
        check_loner_shape(&s, f, &tag);
    }

    phases_seen.insert(s.hand.phase.to_string());
    note_result(&s, &mut hand_results, &mut euchres);

    f.check(steps < MAX_STEPS, || {
        format!("{tag}: did not terminate within {MAX_STEPS} steps")
    });
    f.check(s.status == GameStatus::Complete, || {
        format!("{tag}: ended with status {}", s.status)
    });
    let target = s.cfg.game_to;
    let score = format!("{},{}", s.score[0], s.score[1]);
    f.check(s.score[0] >= target || s.score[1] >= target, || {
        format!("{tag}: complete but neither team reached {target} ({score})")
    });
    f.check(!(s.score[0] >= target && s.score[1] >= target), || {
        format!("{tag}: both teams at or past {target} ({score})")
    });
    // legalMoves must close down once the match is over.
    for seat in SEATS {
        f.check(legal_moves(&s, seat).is_empty(), || {
            format!("{tag}: seat {seat} still has moves after game over")
        });
    }

    GameSummary {
        hands,
        final_score: s.score,
        winner: if s.score[0] >= target { 0 } else { 1 },
        steps,
        phases_seen,
        move_ids_seen,
        hand_results,
        loners,
        euchres,
    }
}

/// `Math.floor(r * len)`, clamped into range.
fn pick_index(r: f64, len: usize) -> usize {
    let i = (r * len as f64).floor();
    if i.is_finite() && i >= 0.0 {
        (i as usize).min(len.saturating_sub(1))
    } else {
        0
    }
}

/// The engine must refuse a move outside the legal set, without mutating state.
fn probe_illegal(s: &GameState, seat: Seat, legal: &[LegalMove], f: &mut Failures, tag: &str) {
    let legal_ids: HashSet<&str> = legal.iter().map(|m| m.id.as_str()).collect();
    let Some(&bogus) = DECK
        .iter()
        .find(|c| !legal_ids.contains(format!("play:{c}").as_str()))
    else {
        return;
    };
    let before = json(s);
    let rejected = apply(s, seat, Move::Play { card: bogus }, None).is_err();
    f.check(rejected, || {
        format!(
            "{tag}: illegal play:{bogus} was accepted for seat {seat} in {}",
            s.hand.phase
        )
    });
    f.check(json(s) == before, || {
        format!("{tag}: rejected move mutated state in place")
    });

    // A seat that is not on turn must never be able to act.
    if let Some(&off) = SEATS.iter().find(|&&x| Some(x) != s.hand.turn_seat) {
        f.check(legal_moves(s, off).is_empty(), || {
            format!("{tag}: off-turn seat {off} has legal moves")
        });
    }
}

/* ----------------------------------------------------------------- narration */

fn glyph(s: Suit) -> &'static str {
    match s {
        Suit::Spades => "♠",
        Suit::Hearts => "♥",
        Suit::Diamonds => "♦",
        Suit::Clubs => "♣",
    }
}

fn card(c: Card) -> String {
    let rank = if c.rank() == Rank::Ten {
        "10".to_owned()
    } else {
        c.rank().as_char().to_string()
    };
    format!("{rank}{}", glyph(c.printed_suit()))
}

fn seat_name(s: Seat) -> &'static str {
    ["South(you)", "West", "North", "East"][s.index()]
}

fn seat_name_opt(s: Option<Seat>) -> &'static str {
    s.map_or("undefined", seat_name)
}

fn pad(s: Seat) -> String {
    format!("{:<10}", seat_name(s))
}

fn cards(cs: &[Card]) -> String {
    cs.iter().map(|&c| card(c)).collect::<Vec<_>>().join(" ")
}

fn narrate_hand_start(out: &mut dyn fmt::Write, s: &GameState) {
    let h = &s.hand;
    let up = h.up_card.map_or_else(|| "?".to_owned(), card);
    say!(
        out,
        "\n┌─ Hand {} ── dealer {} ── up-card {up} ── score {}–{} ─┐",
        h.hand_no + 1,
        seat_name(h.dealer_seat),
        s.score[0],
        s.score[1]
    );
    for seat in SEATS {
        say!(out, "│ {} {}", pad(seat), cards(&h.hands[seat.index()]));
    }
}

fn narrate_trump(out: &mut dyn fmt::Write, s: &GameState) {
    let h = &s.hand;
    let alone = if h.alone_seat.is_some() {
        format!("  ALONE — {} sits out", seat_name_opt(h.sitting_seat))
    } else {
        String::new()
    };
    let trump = h.trump.map_or("?", glyph);
    say!(
        out,
        "│ ▸ {trump} is trump, called by {}{alone}",
        seat_name_opt(h.maker_seat)
    );
}

fn narrate_move(out: &mut dyn fmt::Write, s: &GameState, seat: Seat, m: &LegalMove) {
    let h = &s.hand;
    match m.mv {
        Move::Cut { .. } => {}
        Move::Pass => say!(out, "│   {} \"Pass.\"", pad(seat)),
        Move::OrderUp { .. } | Move::Call { .. } => say!(out, "│   {} \"{}\"", pad(seat), m.label),
        Move::Discard { .. } => say!(out, "│   {} picks up, discards face down.", pad(seat)),
        Move::Play { card: c } => {
            let is_lead = h.trick.plays.is_empty();
            let t = h.trick_log.len() + 1;
            let lead = if is_lead {
                format!("T{t} ")
            } else {
                "   ".to_owned()
            };
            let leads = if is_lead { "  (leads)" } else { "" };
            say!(out, "│     {lead}{} {}{leads}", pad(seat), card(c));
        }
    }
}

fn narrate_hand_end(out: &mut dyn fmt::Write, s: &GameState) {
    let h = &s.hand;
    let result = opt(h.result).to_uppercase();
    say!(
        out,
        "│ ═ {result:<10} tricks {}–{}  →  score {}–{}",
        h.tricks_won[0],
        h.tricks_won[1],
        s.score[0],
        s.score[1]
    );
    say!(out, "└{}", "─".repeat(62));
}

/* ---------------------------------------------------------------------- main */

fn rng_from(seed: u32) -> Rng {
    Box::new(mulberry32(seed))
}

/// Runs the harness with the CLI arguments (`[N] [quiet]`), writing the report
/// to `out`; returns `false` when any check failed.
pub fn run(args: &[String], out: &mut dyn fmt::Write) -> bool {
    let n: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(300);
    let quiet = args.iter().any(|a| a == "quiet");
    let mut f = Failures::default();
    let rule = "═".repeat(64);

    if !quiet {
        say!(out, "{rule}");
        say!(
            out,
            "NARRATED GAME — seed \"demo-7\", random-but-legal play"
        );
        say!(out, "{rule}");
        let g = play_game("demo-7", &mut rng_from(4242), &mut f, out, true);
        say!(
            out,
            "\nFINAL {}–{} — team {} wins after {} hands, {} euchres, {} loner calls.",
            g.final_score[0],
            g.final_score[1],
            g.winner,
            g.hands,
            g.euchres,
            g.loners
        );
    }

    say!(out, "\n{rule}");
    say!(
        out,
        "FUZZ — {n} complete games, every invariant checked at every step"
    );
    say!(out, "{rule}");

    let mut phases = BTreeSet::new();
    let mut move_ids = BTreeSet::new();
    let mut results: Vec<(String, usize)> = Vec::new();
    let mut result_slot: HashMap<String, usize> = HashMap::new();
    let (mut hands, mut steps, mut loners, mut euchres) = (0, 0, 0, 0);
    let t0 = Instant::now();

    for i in 0..n {
        // `i * 7919 + 13` as the JS int32 the TS seed coerces to.
        let seed = (i as u32).wrapping_mul(7919).wrapping_add(13);
        let g = play_game(
            &format!("fuzz-{i}"),
            &mut rng_from(seed),
            &mut f,
            out,
            false,
        );
        hands += g.hands;
        steps += g.steps;
        loners += g.loners;
        euchres += g.euchres;
        phases.extend(g.phases_seen);
        move_ids.extend(g.move_ids_seen);
        for r in g.hand_results {
            let k = r.split(':').nth(1).unwrap_or_default().to_owned();
            let slot = *result_slot.entry(k.clone()).or_insert_with(|| {
                results.push((k, 0));
                results.len() - 1
            });
            results[slot].1 += 1;
        }
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0;

    let a = play_game("det", &mut rng_from(99), &mut f, out, false);
    let b = play_game("det", &mut rng_from(99), &mut f, out, false);
    f.check(a == b, || {
        "determinism: identical seed + rng produced a different game".to_owned()
    });

    let per = |x: f64| if n == 0 { f64::NAN } else { x / n as f64 };
    say!(out, "games            {n}");
    say!(
        out,
        "hands            {hands}  (avg {:.1}/game)",
        per(hands as f64)
    );
    say!(out, "engine steps     {steps}");
    say!(out, "loner calls      {loners}");
    say!(out, "euchres          {euchres}");
    say!(out, "elapsed          {ms:.0}ms  ({:.2}ms/game)", per(ms));
    say!(
        out,
        "phases exercised {}",
        phases.into_iter().collect::<Vec<_>>().join(", ")
    );
    say!(out, "distinct moves   {}", move_ids.len());
    say!(
        out,
        "hand results     {}",
        results
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("  ")
    );

    say!(out, "\n{rule}");
    if f.items.is_empty() {
        say!(out, "✓ ALL INVARIANTS HELD");
        for line in [
            "no secret card ever appeared in any seat's projected view",
            "all 24 cards conserved in every reachable state",
            "hand sizes correct for every seat at every trick",
            "follow-suit re-derived from replayed holdings, never violated",
            "trick winners recomputed independently, always agreed",
            "recorded ledSuit always the effective suit of the lead",
            "loner hands had exactly 3 plays/trick, sitting seat never played",
            "scores monotonic, only +1/+2/+4, never both teams in one hand",
            "illegal moves rejected without mutating state",
            "off-turn seats never had legal moves",
            "exactly one seat could act at any moment",
            "every game terminated with exactly one team at the target",
            "identical seed reproduced an identical game",
        ] {
            say!(out, "  · {line}");
        }
    } else {
        say!(out, "✗ {} DISTINCT FAILURES", f.items.len());
        for m in f.items.iter().take(30) {
            say!(out, "  · {m}");
        }
        return false;
    }
    true
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !run(&args, &mut Stdout) {
        std::process::exit(1);
    }
}
