//! Motion tokens (09 §3): durations, easings, springs, beats, staggers, the turn halo, the card
//! shadow ladder and the gold accent. Every lane reads these; only Lane A changes them.
//!
//! `theme::felt::timing` stays frozen beside them: those constants are the 06 §6 tempo contract
//! and `present.rs` keeps using them.
//!
//! Units: durations are seconds. The ones read against the poll clock (`AppState.clock`, an
//! `f64`) are `f64`; the ones used in pose math, presence delays or springs are `f32`.

use rune::{Beat, Color, Cycle, Easing, El, Shadow, Spring, SpringPair, Transition};

use super::color;

// Kept from the old inline module, values unchanged.

/// Shared-element card flight (07 §5).
pub const FLIGHT: Spring = Spring::new(0.45, 0.12);
/// The 700 ms card play (06 §6.2).
pub const PLAY: Spring = Spring::new(0.7, 0.1);
/// Route changes and small UI settles.
pub const UI: Spring = Spring::new(0.35, 0.0);
/// How long a toast stays up, in seconds.
pub const TOAST_SECS: f64 = 4.0;

// 3.1 Durations (seconds).

/// Arc travel; 0.46 read rushed (S1).
pub const PLAY_TRAVEL: f32 = 0.55;
/// Rotation wobble and squash after touchdown (S1).
pub const PLAY_SETTLE: f32 = 0.40;
/// Travel + settle; the flyer hands off here (S1).
pub const PLAY_END: f32 = 0.95;
/// Touchdown squash window (S1).
pub const SQUASH_T: f32 = 0.16;
/// Gap between cards in one deal packet (S2, d1).
pub const DEAL_IN_PACKET: f32 = 0.055;
/// Gap between deal packets, 3-2 / 2-3 (d1; the mechanism is S2).
pub const DEAL_PACKET_GAP: f32 = 0.110;
/// The up card's width-squeeze turn (S3-A).
pub const FLIP: f32 = 0.36;
/// The up card's gold glint after its turn (09 §4.2: 0 → 0.6 → 0).
pub const FLIP_GLINT: f32 = 0.40;
/// The Brisk two-commit flip: squeeze, then open (S3-B).
pub const FLIP_BRISK: (f32, f32) = (0.18, 0.18);
/// The winner ring waits this long after the 4th card lands (d2).
pub const RING_DELAY: f64 = 0.05;
/// The gather starts this long before the sweep (d3).
pub const GATHER_LEAD: f64 = 0.30;
/// One sweep card's flight at [`SWEEP`] (used by `Collect::finish_at`; S4).
pub const SWEEP_SECS: f64 = 0.60;
/// One sweep card per commit; equals `timing::COLLECT_STAGGER` (06 §6.4, S4).
pub const SWEEP_STAGGER: f64 = 0.045;
/// The illegal-card shake (S5).
pub const SHAKE: f64 = 0.36;
/// One point per step in a score roll (S6).
pub const SCORE_STEP: f64 = 0.14;
/// How long a euchre or march stamp stays on screen (d2).
pub const STAMP_HOLD: f64 = 1.10;
/// A canvas spark burst's life (S8).
pub const SPARKS: f32 = 0.90;
/// One confetti quad's life, shortest and longest; it fades over its last 0.5 s (S8).
pub const CONFETTI_LIFE: (f32, f32) = (2.6, 3.6);
/// How long a confetti burst runs from its banner's first poll (09 §4.9: `t0 + 3.7`).
pub const CONFETTI_SECS: f64 = 3.7;
/// The match's confetti waits this long after its panel starts in (`PANEL_IN` lands at about
/// 0.35 s, then 0.2 s of stillness), so it rises from behind a panel at rest (09 §4.10).
pub const CONFETTI_DELAY: f32 = 0.55;
/// The match-end panel rolls its final score this long after it starts in.
pub const MATCH_ROLL_AT: f32 = 0.35;
/// Fast-forward: the rest of a live shot runs inside this (d1).
pub const FF_SECS: f64 = 0.14;
/// The longest a route transition takes to read as settled (09 §3.1). The nav ignores presses
/// this long after a route change, so the second click of a double-click does not land on
/// whatever link the new screen put under the pointer.
pub const ROUTE_SETTLE: f64 = 0.5;

// 3.1a The play toss (09 §4.3; Lane A wave 1, `choreo::play_pose`).

/// An opponent's arc: the flyer peaks half of this above the chord (09 §4.3).
pub const PLAY_ARC_OPP: f32 = 110.0;
/// Your arc, from the rack (09 §4.3).
pub const PLAY_ARC_YOU: f32 = 40.0;
/// A short throw arcs at most this share of its chord, so a near seat does not loop.
pub const PLAY_ARC_SHARE: f32 = 0.45;
/// Air drag on the ground track: `1 − e^(−kτ)`. Higher leaves faster and lands softer.
pub const PLAY_DRAG: f32 = 2.2;
/// The share of the chord left at touchdown, skidded out by `PLAY_END`.
pub const PLAY_SKID: f32 = 0.03;
/// The touchdown squash, as a scale dip (S1).
pub const PLAY_SQUASH: f32 = 0.035;
/// The flyer's swell at the top of its arc (S1).
pub const PLAY_LIFT_SCALE: f32 = 0.14;

// 3.2 Easings.

/// The card play curve (06 §6.2; S1).
pub const PLAY_EASE: Easing = Easing::cubic(0.22, 0.7, 0.25, 1.0);
/// The up-card turn.
pub const FLIP_EASE: Easing = Easing::EASE_IN_OUT;
/// Pops and rings.
pub const POP_EASE: Easing = Easing::EASE_OUT;
/// Settles that overshoot a little: cubic(.34, 1.45, .64, 1).
pub const SETTLE_EASE: Easing = Easing::OVERSHOOT;

// 3.3 Springs: `Spring::new(period_s, bounce)`; settling takes 1.2-1.5x the period.

/// Play rotation: `.sample(0, 0, 1, t)` (S1).
pub const ROT_SETTLE: Spring = Spring::new(0.42, 0.30);
/// The deal's presence offset, entering (S2).
pub const DEAL_CARD: Spring = Spring::new(0.42, 0.12);
/// The deal's presence offset, leaving (S2).
pub const DEAL_CARD_OUT: Spring = Spring::new(0.20, 0.0);
/// Rack plate lift, scale and shadow, on `.motion` (S5).
pub const HOVER: Spring = Spring::new(0.26, 0.32);
/// A Brisk or Skim matched play (07 §5.1 range).
pub const PLAY_BRISK: Spring = Spring::new(0.42, 0.10);
/// The up card to the dealer (was 0.85/0.05; d2).
pub const PICKUP: Spring = Spring::new(0.55, 0.15);
/// When the pickup reads as landed: `PICKUP` is within 1.5% of rest by 0.38 s.
pub const PICKUP_LAND: f32 = 0.40;
/// The picked-up card turns face down on the dealer's fan, a quick squeeze (09 §4.6).
pub const PICKUP_TURN: f32 = FLIP * 0.5;
/// The share of `PICKUP_LAND` at the end of the flight in which the picked-up card shrinks
/// from the up card's size to a back's (mechanism A): until then it flies at full size.
pub const PICKUP_SHRINK: f32 = 0.3;
/// The trick cards converge (S4).
pub const GATHER: Spring = Spring::new(0.30, 0.12);
/// Matched into the minis (was `COLLECT` 0.78/0; 0.5 VERIFIED, 0.45 inside the hold; S4).
pub const SWEEP: Spring = Spring::new(0.45, 0.0);
/// The turn marker between seats (S7).
pub const MARKER: Spring = Spring::new(0.50, 0.20);
/// The "+2" chip, entering (S6).
pub const CHIP_IN: Spring = Spring::new(0.40, 0.30);
/// The "+2" chip, leaving (S6).
pub const CHIP_OUT: Spring = Spring::new(0.30, 0.0);
/// `RollingText::spring` for the score (S6).
pub const ROLL: Spring = Spring::new(0.42, 0.18);
/// A screen entering: the new sheet's wipe across the window (S9; 09 §4.12). The bounce
/// gives the wipe a quick start and has it across by about 0.3 s; past the far edge an
/// overshoot shows nothing, so the sheet never wobbles.
pub const ROUTE_IN: Spring = Spring::new(0.45, 0.2);
/// A screen leaving: it stays under the new sheet and dissolves slowly, so the wipe passes
/// over the old page rather than over an empty background.
pub const ROUTE_OUT: Spring = Spring::new(0.8, 0.0);
/// The table leaving: quickly, because its raised layers (the match-end panel, confetti, the
/// banner) paint above every sheet and would otherwise hang over the new page.
pub const ROUTE_OUT_TABLE: Spring = Spring::new(0.12, 0.0);
/// A page that must go at once (the table's loading page when the table arrives): every
/// removal fades, so this one fades in a blink.
pub const BLINK: Spring = Spring::new(0.06, 0.0);

/// The exit for something that must go at once: every removal fades, so this one fades in a
/// [`BLINK`]. For a parent's `child_exit` or an element's own transition.
pub fn blink_out() -> Transition {
    Transition::opacity().springs(SpringPair::new(UI, BLINK))
}
/// The table's build-in (09 §4.12): what mounts within this long of the route to the table
/// enters seat by seat, then the rack. Later mounts keep their own entrances.
pub const BUILD_IN_WINDOW: f64 = 1.0;
/// The build-in waits until the wipe is about this far along (seconds after the route).
pub const BUILD_IN_LEAD: f64 = 0.12;
/// One seat to the next in the build-in.
pub const BUILD_IN_STAGGER: f32 = 0.04;
/// The rack follows the seats this long after the build-in starts.
pub const BUILD_IN_RACK: f32 = 0.2;
/// The build-in's rise and fade, in and out.
pub const BUILD_IN: SpringPair = SpringPair::new(Spring::new(0.45, 0.1), Spring::new(0.2, 0.0));
/// Felt buttons' key physics (09 §5.4).
pub const KEY_PRESS: Spring = Spring::new(0.2, 0.3);
/// The Play button's entrance and exit.
pub const BUTTON_IN: SpringPair = SpringPair::new(Spring::new(0.3, 0.0), Spring::new(0.2, 0.0));
/// How far a new line or button rises as it enters, points (09 §4.8).
pub const RISE: f32 = 8.0;
/// A seat plate lighting up for its turn.
pub const SEAT_PLATE: Spring = Spring::new(0.3, 0.0);
/// A name or title fading in once it may show (the up card's name, the result title).
pub const NAME_FADE: Spring = Spring::new(0.25, 0.0);
/// The felt cooling after a lost match (09 §4.10 fallback).
pub const FELT_COOL: Spring = Spring::new(0.8, 0.0);
/// The trump line's old words fading out, quickly, before the new line enters.
pub const TRUMP_OUT: SpringPair =
    SpringPair::new(Spring::new(TRUMP_FADE, 0.0), Spring::new(TRUMP_FADE, 0.0));
/// A status line's words swapping (the felt's result line, 09 §4.8): the old line fades out
/// in about 0.1 s, then the new one rises in ([`LINE_SWAP_GAP`] later).
pub const LINE_SWAP: SpringPair = SpringPair::new(Spring::new(0.30, 0.0), Spring::new(0.10, 0.0));
/// The new line waits this long before it rises in: the old line's 0.10 s exit is at 4% by
/// 0.08 s, so the two never print over each other.
pub const LINE_SWAP_GAP: f32 = 0.09;
/// Dialog scrim opacity, entering (S10).
pub const SCRIM_IN: Spring = Spring::new(0.30, 0.0);
/// Dialog scrim opacity, leaving (S10).
pub const SCRIM_OUT: Spring = Spring::new(0.22, 0.0);
/// The dialog panel, entering (S10).
pub const PANEL_IN: Spring = Spring::new(0.45, 0.30);
/// The dialog panel, leaving (S10).
pub const PANEL_OUT: Spring = Spring::new(0.20, 0.0);
/// The Home to Table matched hero (S9).
pub const HERO: Spring = Spring::new(0.60, 0.12);
/// The settings rail slide (06 §6.5: 320 ms).
pub const RAIL: Spring = Spring::new(0.32, 0.0);

// 3.4 Beats (opacity and uniform scale only; beats are not clamped).

/// The winner card pops (09 §3 had a 1.09 peak; 1.13 reads at table distance).
pub const WIN_POP: &[Beat] = &[
    Beat::new(0.0).scale(1.0),
    Beat::new(0.12).scale(1.13).easing(Easing::EASE_OUT),
    Beat::new(0.30).scale(1.0),
];
/// The winner ring enters.
pub const RING_IN: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(1.18),
    Beat::new(0.22)
        .opacity(1.0)
        .scale(1.0)
        .easing(Easing::EASE_OUT),
];
/// A ring that pulses out and fades.
pub const PULSE: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(0.9),
    Beat::new(0.12)
        .opacity(0.9)
        .scale(1.12)
        .easing(Easing::EASE_OUT),
    Beat::new(0.5).opacity(0.0).scale(1.35),
];
/// A number pops.
pub const NUM_POP: &[Beat] = &[
    Beat::new(0.0).scale(1.0),
    Beat::new(0.10).scale(1.18).easing(Easing::EASE_OUT),
    Beat::new(0.32).scale(1.0).easing(Easing::OVERSHOOT),
];
/// The turn ping.
pub const PING: &[Beat] = &[
    Beat::new(0.0).opacity(0.7).scale(1.0),
    Beat::new(0.9)
        .opacity(0.0)
        .scale(1.45)
        .easing(Easing::EASE_OUT),
];
/// A small knock: the deck at a deal, the stack when a trick lands, the maker chip.
pub const THUMP: &[Beat] = &[
    Beat::new(0.0).scale(1.0),
    Beat::new(0.08).scale(1.10).easing(Easing::EASE_OUT),
    Beat::new(0.22).scale(1.0),
];
/// The trump badge arrives: 0.6 to 1.3 to 1.0 (d2).
pub const TRUMP_IN: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(0.6),
    Beat::new(0.18)
        .opacity(1.0)
        .scale(1.3)
        .easing(Easing::EASE_OUT),
    Beat::new(0.42).scale(1.0).easing(Easing::OVERSHOOT),
];
/// The words before a trump call fade out this fast, and the call's `TRUMP_IN` and `FLASH`
/// start after it (09 §4.6).
pub const TRUMP_FADE: f32 = 0.12;
/// The gold flash behind the trump line as trump is called.
pub const FLASH: &[Beat] = &[
    Beat::new(0.0).opacity(0.0),
    Beat::new(0.10).opacity(1.0).easing(Easing::EASE_OUT),
    Beat::new(0.60).opacity(0.0),
];
/// A euchre or march stamp lands (inside the Transition scale clamp of [0.9, 1.1]).
pub const STAMP: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(1.1),
    Beat::new(0.16)
        .opacity(1.0)
        .scale(0.97)
        .easing(Easing::EASE_OUT),
    Beat::new(0.34).scale(1.0).easing(Easing::OVERSHOOT),
];

/// How long `beats` play: the last beat's time.
pub fn beats_secs(beats: &[Beat]) -> f64 {
    beats.last().map_or(0.0, |b| b.at)
}

// 3.5 Staggers and the cycle.

/// Sweep slot order after the taker's card, which leads: left, partner, right, you
/// (present.rs `Collect::new`).
pub const SWEEP_ORDER: [usize; 4] = [0, 1, 2, 3];

/// The turn halo's breathing opacity (your turn only; 30 fps; reduced holds `still`).
pub static TURN_HALO: Cycle = Cycle {
    period: 1.6,
    phase: 0.0,
    stops: &[(0.0, 0.35), (0.5, 1.0), (1.0, 0.35)],
    still: 0.8,
    fps: 30.0,
};

/// The deal's packets, in dealing order: 3-2-3-2, then 2-3-2-3.
const DEAL_PACKETS: [usize; 8] = [3, 2, 3, 2, 2, 3, 2, 3];

/// The `enter_delay` of the deal's `j`th card (0-based, global deal order): a
/// [`DEAL_IN_PACKET`] step within a packet and a [`DEAL_PACKET_GAP`] step between packets.
/// Indexes past the twentieth card keep stepping within the last packet.
pub fn deal_delay(j: usize) -> f32 {
    let mut first = 0;
    let mut packet = 0;
    for (p, &n) in DEAL_PACKETS.iter().enumerate() {
        packet = p;
        if j < first + n {
            break;
        }
        if p + 1 < DEAL_PACKETS.len() {
            first += n;
        }
    }
    (j - packet) as f32 * DEAL_IN_PACKET + packet as f32 * DEAL_PACKET_GAP
}

// 3.6 Elevation: the lamp is upper-left, so shadows fall right and down.

/// Every card emits exactly three shadows (edge, drop, glow) so their geometry springs (S5/S11).
/// `lift` is the elevation, 0 at rest to 1 mid-flight; `glow` is the gold glow, 0 to 1.
///
/// | Level | `lift` | Where |
/// |---|---|---|
/// | E0 | 0.0 | trick card at rest, stack minis |
/// | E1 | 0.15 | rack card at rest |
/// | E2 | 0.45 | rack hover |
/// | E3 | 0.65 | rack selected |
/// | E4 | `sin(pi * u)` | flyer mid-flight |
pub fn card_shadows(e: El, lift: f32, glow: f32) -> El {
    let l = lift.clamp(0.0, 1.0);
    e.box_shadow(Shadow {
        y: 1.0,
        blur: 0.0,
        color: color(EDGE).alpha(1.0 - l),
        ..Default::default()
    })
    .box_shadow(Shadow {
        x: 1.0 + 3.0 * l,
        y: 4.0 + 16.0 * l,
        blur: 10.0 + 26.0 * l,
        color: Color::BLACK.alpha(0.28 + 0.2 * l),
        ..Default::default()
    })
    .box_shadow(Shadow {
        y: 0.0,
        blur: 28.0,
        spread: 2.0,
        color: color(GOLD).alpha(0.75 * glow),
        ..Default::default()
    })
}

/// E5, separate from the card ladder: dialogs, banners and stamps (S10): y 24, blur 60, black
/// at alpha 0.45.
pub fn e5(e: El) -> El {
    e.box_shadow(Shadow {
        y: 24.0,
        blur: 60.0,
        color: Color::BLACK.alpha(0.45),
        ..Default::default()
    })
}

// 3.8 Colours (one accent: gold).

/// The only accent: winner ring, turn halo, glow, selected outline (d2).
pub const GOLD: &str = "#f4d27a";
/// The ring border on light surfaces, the stamp text edge.
pub const GOLD_DEEP: &str = "#c9a24a";
/// The card edge shadow; the existing `SHADOW_CARD_EDGE`.
pub const EDGE: &str = "#c4c7b9";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse() {
        for token in [GOLD, GOLD_DEEP, EDGE] {
            assert!(rune::color::hex(token).is_some(), "{token} parses");
        }
        assert_eq!(EDGE, super::super::hexes::SHADOW_CARD_EDGE);
    }

    #[test]
    fn deal_delay_follows_the_packets() {
        assert_eq!(deal_delay(0), 0.0);
        assert!((deal_delay(1) - DEAL_IN_PACKET).abs() < 1e-6);
        // The fourth card opens the second packet: two in-packet steps and one packet gap.
        assert!((deal_delay(3) - (2.0 * DEAL_IN_PACKET + DEAL_PACKET_GAP)).abs() < 1e-6);
        // The last card lands near 1.95 s with its 0.5 s spring (09 §4.1).
        assert!((deal_delay(19) - 1.43).abs() < 1e-4);
        assert!((1..20).all(|j| deal_delay(j) > deal_delay(j - 1)));
    }

    #[test]
    fn the_spec_numbers_hold_together() {
        assert!((PLAY_TRAVEL + PLAY_SETTLE - PLAY_END).abs() < 1e-6);
        assert_eq!(SWEEP_STAGGER, crate::theme::felt::timing::COLLECT_STAGGER);
        assert_eq!(DEAL_PACKETS.iter().sum::<usize>(), 20);
    }
}
