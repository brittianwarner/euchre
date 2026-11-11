//! The felt's own tokens (06 §1.2 "Table (felt) surfaces", §5.3, §5.4, §5.8, §9): the gold and
//! cream accents on the green, the score cards, the bid buttons, the outcome banner and the
//! card elevations. Kept apart from [`super::PAL`] so the table lane can grow them without
//! touching the shell's palette.

use std::sync::LazyLock;

use rune::Color;
use rune::tok::TextStyle;

use super::{SERIF, ShadowLayer, color};

/// The sRGB hex values of the felt tokens.
pub mod hexes {
    /// The lead arrow and the old winner outline: the one gold (09 §3.8, `motion::GOLD`).
    pub const WINNER: &str = crate::theme::motion::GOLD;
    /// Active seat name.
    pub const SEAT_NAME_ACTIVE: &str = "#ffe5a4";
    /// Seat initial marker: default, partner, active.
    pub const INITIAL: &str = "#b9d2c1";
    /// Partner's initial marker.
    pub const INITIAL_PARTNER: &str = "#e4cf91";
    /// The active seat's initial marker.
    pub const INITIAL_ACTIVE: &str = "#edd79a";
    /// The trump glyph in the status row.
    pub const TRUMP_GLYPH: &str = "#e0cf97";
    /// The lone marker (`★ … alone`).
    pub const LONE: &str = "#f2d887";
    /// The banter speaker's name.
    pub const QUOTE_SPEAKER: &str = "#e9d79f";
    /// Trick-won result text.
    pub const TRICK_WON: &str = "#ffecb7";
    /// The trick result line.
    pub const RESULT: &str = "#e0e9d7";
    /// Dealer and maker chip fill.
    pub const CHIP_BG: &str = "#f4ead0";
    /// Dealer chip text.
    pub const DEALER_TEXT: &str = "#35482f";
    /// Dealer chip under-edge.
    pub const DEALER_EDGE: &str = "#b4a57a";
    /// Maker chip text.
    pub const MAKER_TEXT: &str = "#243d31";
    /// The led pill fill.
    pub const LED_PILL: &str = "#ffffff0c";
    /// The active seat tint.
    pub const SEAT_TINT: &str = "#ffffff0a";
    /// The hand area's top border.
    pub const HAND_BORDER: &str = "#e2dbbd26";
    /// Card label on the felt.
    pub const CARD_LABEL: &str = "#f7f1df";
    /// Card sub-label on the felt.
    pub const CARD_SUB: &str = "#ddd5bd";
    /// Disabled felt button fill.
    pub const BUTTON_OFF_BG: &str = "#e8eadb";
    /// Disabled felt button text.
    pub const BUTTON_OFF_TEXT: &str = "#6a7466";
    /// Disabled felt button border.
    pub const BUTTON_OFF_BORDER: &str = "#d5ddc7";
    /// Secondary bid button fill (`pass`, `cut:no`).
    pub const BID_SECONDARY: &str = "#fffdf6";
    /// Secondary bid button border.
    pub const BID_SECONDARY_BORDER: &str = "#85957b";
    /// Secondary bid button text.
    pub const BID_SECONDARY_TEXT: &str = "#334d3e";
    /// Secondary bid button hover.
    pub const BID_SECONDARY_HOVER: &str = "#e9eddf";
    /// The taken-stack fill.
    pub const STACK: &str = "#285743";
    /// The taken-stack stripe.
    pub const STACK_STRIPE: &str = "#e8e2ca";
    /// The taken-stack border.
    pub const STACK_BORDER: &str = "#f8f0db";
    /// The taken-stack first offset edge.
    pub const STACK_EDGE_1: &str = "#c6c6b5";
    /// The taken-stack second offset edge.
    pub const STACK_EDGE_2: &str = "#e7e5d8";
    /// Deck tile border.
    pub const DECK_BORDER: &str = "#f4f0df";
    /// Deck tile outline.
    pub const DECK_OUTLINE: &str = "#ffffff70";
    /// Deck tile stripes, light and dark.
    pub const DECK_LIGHT: &str = "#3a6448";
    /// Deck tile dark stripe.
    pub const DECK_DARK: &str = "#2a4f3b";
    /// Deck tile club.
    pub const DECK_CLUB: &str = "#f2e8cd";
    /// Outcome banner fill.
    pub const BANNER: &str = "#fff8df";
    /// Outcome banner text.
    pub const BANNER_TEXT: &str = "#253c2d";
    /// Outcome banner star.
    pub const BANNER_STAR: &str = "#967629";
    /// Outcome banner shadow.
    pub const BANNER_SHADOW: &str = "#00180d44";
    /// Confetti, cycling (09 §3.8: the 06 three plus cream and lamp gold).
    pub const CONFETTI: [&str; 5] = ["#e0c579", "#497d69", "#ba5e46", "#fff8df", "#f5d98e"];
    /// The felt's lit centre (09 §3.8, S11).
    pub const FELT_HI: &str = "#2d6b56";
    /// The rail gradient's top stop (S11).
    pub const RAIL_HI: &str = "#7a5232";
    /// The vignette's outer stop: black at 0.42 (S11).
    pub const VIGNETTE: &str = "#0000006b";
    /// The lamp pool over the trick (d1).
    pub const LAMP: &str = "#f5d98e2e";
    /// A seat plate at rest (d3).
    pub const SEAT_PLATE: &str = "#0f31238c";
    /// The seat plate on its turn (d3).
    pub const SEAT_PLATE_ON: &str = "#1a4a37d9";
    /// The felt cooling after a lost match: `#0b1d16` at 0.18 (09 §4.10 fallback).
    pub const COOL: &str = "#0b1d162e";
    /// Spark streaks: gold, and every third one white (S8).
    pub const SPARK: [&str; 2] = ["#ffd75e", "#ffffff"];
    /// The stamp's ink: a deep gold that reads on cream.
    pub const STAMP_INK: &str = "#8a6a22";
    /// The banter bubble's fill: the felt's darkest green, nearly opaque.
    pub const BUBBLE: &str = "#0b2a1ff0";
    /// The banter bubble's edge.
    pub const BUBBLE_EDGE: &str = "#e9d79f40";
    /// The banter bubble's text.
    pub const BUBBLE_TEXT: &str = "#f3ecd4";
    /// Score card stroke.
    pub const SCORE_STROKE: &str = "#99a58a";
    /// Score card spade pips.
    pub const SCORE_SPADE: &str = "#243e32";
    /// Score divider `—`.
    pub const SCORE_DIVIDER: &str = "#78856d";
    /// `First to 10`.
    pub const FIRST_TO: &str = "#596952";
    /// Played card shadow layers.
    pub const PLAYED_EDGE_1: &str = "#d9d8ca";
    /// Played card second edge.
    pub const PLAYED_EDGE_2: &str = "#abb3a0";
    /// Played card drop.
    pub const PLAYED_DROP: &str = "#082b2350";
    /// Up card / deck first edge.
    pub const UP_EDGE_1: &str = "#c6c9ba";
    /// Up card / deck second edge.
    pub const UP_EDGE_2: &str = "#aeb7a6";
    /// Up card / deck drop.
    pub const UP_DROP: &str = "#0d281b55";
    /// Selected hand card edge.
    pub const SELECTED_EDGE: &str = "#bac2b1";
    /// Selected hand card drop.
    pub const SELECTED_DROP: &str = "#15301d30";
}

/// Every felt token as a linear [`Color`], converted once.
pub struct FeltPalette {
    /// [`hexes::WINNER`].
    pub winner: Color,
    /// [`hexes::SEAT_NAME_ACTIVE`].
    pub seat_name_active: Color,
    /// [`hexes::INITIAL`].
    pub initial: Color,
    /// [`hexes::INITIAL_PARTNER`].
    pub initial_partner: Color,
    /// [`hexes::INITIAL_ACTIVE`].
    pub initial_active: Color,
    /// [`hexes::TRUMP_GLYPH`].
    pub trump_glyph: Color,
    /// [`hexes::LONE`].
    pub lone: Color,
    /// [`hexes::QUOTE_SPEAKER`].
    pub quote_speaker: Color,
    /// [`hexes::TRICK_WON`].
    pub trick_won: Color,
    /// [`hexes::RESULT`].
    pub result: Color,
    /// [`hexes::CHIP_BG`].
    pub chip_bg: Color,
    /// [`hexes::DEALER_TEXT`].
    pub dealer_text: Color,
    /// [`hexes::DEALER_EDGE`].
    pub dealer_edge: Color,
    /// [`hexes::MAKER_TEXT`].
    pub maker_text: Color,
    /// [`hexes::LED_PILL`].
    pub led_pill: Color,
    /// [`hexes::SEAT_TINT`].
    pub seat_tint: Color,
    /// [`hexes::HAND_BORDER`].
    pub hand_border: Color,
    /// [`hexes::CARD_LABEL`].
    pub card_label: Color,
    /// [`hexes::CARD_SUB`].
    pub card_sub: Color,
    /// [`hexes::BUTTON_OFF_BG`].
    pub button_off_bg: Color,
    /// [`hexes::BUTTON_OFF_TEXT`].
    pub button_off_text: Color,
    /// [`hexes::BUTTON_OFF_BORDER`].
    pub button_off_border: Color,
    /// [`hexes::BID_SECONDARY`].
    pub bid_secondary: Color,
    /// [`hexes::BID_SECONDARY_BORDER`].
    pub bid_secondary_border: Color,
    /// [`hexes::BID_SECONDARY_TEXT`].
    pub bid_secondary_text: Color,
    /// [`hexes::BID_SECONDARY_HOVER`].
    pub bid_secondary_hover: Color,
    /// [`hexes::STACK`].
    pub stack: Color,
    /// [`hexes::STACK_STRIPE`].
    pub stack_stripe: Color,
    /// [`hexes::STACK_BORDER`].
    pub stack_border: Color,
    /// [`hexes::DECK_BORDER`].
    pub deck_border: Color,
    /// [`hexes::DECK_OUTLINE`].
    pub deck_outline: Color,
    /// [`hexes::DECK_LIGHT`].
    pub deck_light: Color,
    /// [`hexes::DECK_DARK`].
    pub deck_dark: Color,
    /// [`hexes::DECK_CLUB`].
    pub deck_club: Color,
    /// [`hexes::BANNER`].
    pub banner: Color,
    /// [`hexes::BANNER_TEXT`].
    pub banner_text: Color,
    /// [`hexes::BANNER_STAR`].
    pub banner_star: Color,
    /// [`hexes::CONFETTI`].
    pub confetti: [Color; 5],
    /// [`hexes::FELT_HI`].
    pub felt_hi: Color,
    /// [`hexes::RAIL_HI`].
    pub rail_hi: Color,
    /// [`hexes::VIGNETTE`].
    pub vignette: Color,
    /// [`hexes::LAMP`].
    pub lamp: Color,
    /// [`hexes::SEAT_PLATE`].
    pub seat_plate: Color,
    /// [`hexes::SEAT_PLATE_ON`].
    pub seat_plate_on: Color,
    /// [`hexes::COOL`].
    pub cool: Color,
    /// [`hexes::SPARK`].
    pub spark: [Color; 2],
    /// [`hexes::STAMP_INK`].
    pub stamp_ink: Color,
    /// [`hexes::BUBBLE`].
    pub bubble: Color,
    /// [`hexes::BUBBLE_EDGE`].
    pub bubble_edge: Color,
    /// [`hexes::BUBBLE_TEXT`].
    pub bubble_text: Color,
    /// [`hexes::SCORE_STROKE`].
    pub score_stroke: Color,
    /// [`hexes::SCORE_SPADE`].
    pub score_spade: Color,
    /// [`hexes::SCORE_DIVIDER`].
    pub score_divider: Color,
    /// [`hexes::FIRST_TO`].
    pub first_to: Color,
}

/// The felt palette, converted from [`hexes`] on first use.
pub static FELT: LazyLock<FeltPalette> = LazyLock::new(|| {
    use hexes::*;
    FeltPalette {
        winner: color(WINNER),
        seat_name_active: color(SEAT_NAME_ACTIVE),
        initial: color(INITIAL),
        initial_partner: color(INITIAL_PARTNER),
        initial_active: color(INITIAL_ACTIVE),
        trump_glyph: color(TRUMP_GLYPH),
        lone: color(LONE),
        quote_speaker: color(QUOTE_SPEAKER),
        trick_won: color(TRICK_WON),
        result: color(RESULT),
        chip_bg: color(CHIP_BG),
        dealer_text: color(DEALER_TEXT),
        dealer_edge: color(DEALER_EDGE),
        maker_text: color(MAKER_TEXT),
        led_pill: color(LED_PILL),
        seat_tint: color(SEAT_TINT),
        hand_border: color(HAND_BORDER),
        card_label: color(CARD_LABEL),
        card_sub: color(CARD_SUB),
        button_off_bg: color(BUTTON_OFF_BG),
        button_off_text: color(BUTTON_OFF_TEXT),
        button_off_border: color(BUTTON_OFF_BORDER),
        bid_secondary: color(BID_SECONDARY),
        bid_secondary_border: color(BID_SECONDARY_BORDER),
        bid_secondary_text: color(BID_SECONDARY_TEXT),
        bid_secondary_hover: color(BID_SECONDARY_HOVER),
        stack: color(STACK),
        stack_stripe: color(STACK_STRIPE),
        stack_border: color(STACK_BORDER),
        deck_border: color(DECK_BORDER),
        deck_outline: color(DECK_OUTLINE),
        deck_light: color(DECK_LIGHT),
        deck_dark: color(DECK_DARK),
        deck_club: color(DECK_CLUB),
        banner: color(BANNER),
        banner_text: color(BANNER_TEXT),
        banner_star: color(BANNER_STAR),
        confetti: CONFETTI.map(color),
        felt_hi: color(FELT_HI),
        rail_hi: color(RAIL_HI),
        vignette: color(VIGNETTE),
        lamp: color(LAMP),
        seat_plate: color(SEAT_PLATE),
        seat_plate_on: color(SEAT_PLATE_ON),
        cool: color(COOL),
        spark: SPARK.map(color),
        stamp_ink: color(STAMP_INK),
        bubble: color(BUBBLE),
        bubble_edge: color(BUBBLE_EDGE),
        bubble_text: color(BUBBLE_TEXT),
        score_stroke: color(SCORE_STROKE),
        score_spade: color(SCORE_SPADE),
        score_divider: color(SCORE_DIVIDER),
        first_to: color(FIRST_TO),
    }
});

/// Card elevations on the felt (06 §1.4).
pub mod shadow {
    use super::ShadowLayer;
    use super::hexes::*;
    /// A played card: `0 2px 0 #d9d8ca, 0 4px 0 #abb3a0, 0 9px 15px #082b2350`.
    pub const PLAYED: [ShadowLayer; 3] = [
        (0.0, 2.0, 0.0, 0.0, PLAYED_EDGE_1),
        (0.0, 4.0, 0.0, 0.0, PLAYED_EDGE_2),
        (0.0, 9.0, 15.0, 0.0, PLAYED_DROP),
    ];
    /// The up card and deck: `0 3px #c6c9ba, 0 6px #aeb7a6, 0 12px 20px #0d281b55`.
    pub const UP: [ShadowLayer; 3] = [
        (0.0, 3.0, 0.0, 0.0, UP_EDGE_1),
        (0.0, 6.0, 0.0, 0.0, UP_EDGE_2),
        (0.0, 12.0, 20.0, 0.0, UP_DROP),
    ];
    /// A selected hand card: `0 4px 0 #bac2b1, 0 15px 20px #15301d30`.
    pub const SELECTED: [ShadowLayer; 2] = [
        (0.0, 4.0, 0.0, 0.0, SELECTED_EDGE),
        (0.0, 15.0, 20.0, 0.0, SELECTED_DROP),
    ];
    /// The taken stack: `2px 2px #c6c6b5, 4px 4px #e7e5d8`.
    pub const STACK: [ShadowLayer; 2] = [
        (2.0, 2.0, 0.0, 0.0, STACK_EDGE_1),
        (4.0, 4.0, 0.0, 0.0, STACK_EDGE_2),
    ];
    /// The dealer chip's under-edge: `0 2px 0 #b4a57a`.
    pub const DEALER: [ShadowLayer; 1] = [(0.0, 2.0, 0.0, 0.0, DEALER_EDGE)];
    /// The outcome banner: `0 8px 32px #00180d44`.
    pub const BANNER: [ShadowLayer; 1] = [(0.0, 8.0, 32.0, 0.0, BANNER_SHADOW)];
}

/// Measures the felt pieces use (06 §5.3, §5.4, §5.6, §5.8, §9).
pub mod size {
    /// Score cards: the drawing's viewBox and its shown size (06 §5.3).
    pub const SCORE_VIEWBOX: [f32; 2] = [78.0, 50.0];
    /// Score cards shown size.
    pub const SCORE_CARDS: [f32; 2] = [72.0, 46.0];
    /// Header minimum height.
    pub const HEADER_H: f32 = 64.0;
    /// Header height in the compact (short window) layout.
    pub const HEADER_H_COMPACT: f32 = 52.0;
    /// Window width below which `First to 10` hides.
    pub const FIRST_TO_MIN_W: f32 = 1050.0;
    /// Window height below which the table goes compact.
    pub const COMPACT_H: f32 = 640.0;
    /// Seat initial marker.
    pub const INITIAL: f32 = 28.0;
    /// Seat name max width (09 §5.2: 130 to 150).
    pub const NAME_MAX_W: f32 = 150.0;
    /// A seat block narrower than this drops its initial and puts its badges on their own line.
    pub const SEAT_TIGHT_W: f32 = 160.0;
    /// The least a seat name keeps before it truncates.
    pub const NAME_MIN_W: f32 = 40.0;
    /// Dealer and maker chips.
    pub const CHIP: f32 = 20.0;
    /// An opponent's hand token (the back a played card flies from).
    pub const TOKEN: [f32; 2] = [20.0, 28.0];
    /// The taken-stack glyph.
    pub const STACK: [f32; 2] = [24.0, 16.0];
    /// The collected minis that fly into the stack.
    pub const MINI: [f32; 2] = [24.0, 34.0];
    /// Opacity of the taken stack once the seat has a trick.
    pub const STACK_SHOWN: f32 = 0.8;
    /// Opacity of a mini at rest (the collect fades the card as it lands).
    pub const MINI_REST: f32 = 0.0;
    /// Side seat width cap and share of the felt (the cap fits "Grandpa" beside three marks).
    pub const SIDE_SEAT: (f32, f32) = (224.0, 0.18);
    /// Played card height as a share of the trick box, less a small margin.
    pub const PLAYED_H_SHARE: (f32, f32) = (0.80, 8.0);
    /// The same in a short window (below [`COMPACT_H`]): the trick gives way before the rack.
    pub const PLAYED_H_SHARE_COMPACT: (f32, f32) = (0.68, 8.0);
    /// Played card width cap in a window at most 850 tall, and above it.
    pub const PLAYED_CAP: (f32, f32) = (108.0, 145.0);
    /// Felt content padding (sides, top, bottom), clear of the rim: normal and compact.
    pub const CONTENT_PAD: [(f32, f32, f32); 2] = [(28.0, 24.0, 20.0), (16.0, 8.0, 6.0)];
    /// The partner's seat lays out in one row (name beside fan) at this width, in a wide felt.
    pub const PARTNER_WIDE_W: f32 = 280.0;
    /// How far the trump call's gold flash reaches past its line, sideways (half that
    /// vertically).
    pub const TRUMP_FLASH_PAD: f32 = 12.0;
    /// Gap between a seat's badges, and between its fan and stack.
    pub const SEAT_GAP: f32 = 6.0;
    /// Hand area height in the compact layout: share of the window and its clamp.
    pub const HAND_COMPACT: (f32, f32, f32) = (0.40, 190.0, 400.0);
    /// A compact window at least this tall keeps the card's sub line (`Selected ✓`).
    pub const SUB_LINE_MIN_H: f32 = 501.0;
    /// The card sub line's reserved height.
    pub const SUB_LINE_H: f32 = 17.0;
    /// The controls row under the rack: normal and compact.
    pub const CONTROLS_H: (f32, f32) = (56.0, 48.0);
    /// The trick result row, tall enough for Continue: normal and compact.
    pub const RESULT_H: (f32, f32) = (48.0, 42.0);
    /// Deck tile border width.
    pub const DECK_BORDER: f32 = 6.0;
    /// Outcome banner top offset inside the hand area (over the rack, clear of the felt).
    pub const BANNER_TOP: f32 = 64.0;
    /// Outcome banner radius.
    pub const BANNER_RADIUS: f32 = 14.0;
    /// Where the match-end panel's centre sits, as a share of the window's height (09 §4.10).
    pub const MATCH_END_AT: f32 = 0.38;
    /// The match-end panel's width, inside the window's gutters (09 §4.10).
    pub const MATCH_END_W: f32 = 448.0;
    /// About how tall the match-end panel lays out: where its top edge is before it is laid out.
    pub const MATCH_END_H: f32 = 220.0;
    /// The match-end panel's radius (the dialog sheets' 18).
    pub const MATCH_END_RADIUS: f32 = 18.0;
    /// Confetti piece size (the renderer-owned fallback fall).
    pub const CONFETTI: [f32; 2] = [8.0, 14.0];
    /// Confetti count (the renderer-owned fallback fall).
    pub const CONFETTI_COUNT: usize = 24;
    /// Canvas confetti quads, two cannons (09 §2.7 cap).
    pub const CONFETTI_QUADS: u32 = 120;
    /// Canvas spark streaks per burst (09 §2.7 cap).
    pub const SPARKS: u32 = 28;
    /// An upward spark's start offset and share of the travel: the burst stays flat on top,
    /// under the partner's plate.
    pub const SPARK_RISE: (f32, f32) = (20.0, 0.25);
    /// The rail (09 §5.1): corner radius, its brass inlay's inset.
    pub const RAIL_RADIUS: f32 = 32.0;
    /// The rail's brass inlay inset from its outer edge.
    pub const RAIL_INLAY: f32 = 3.0;
    /// The felt's corner radius inside the rail.
    pub const FELT_RADIUS: f32 = 20.0;
    /// The lamp pool, vignette and felt radial radii as shares of the felt `[w, h]`.
    pub const FELT_REACH: [f32; 2] = [0.48, 0.66];
    /// The lamp pool's reach.
    pub const LAMP_REACH: [f32; 2] = [0.34, 0.60];
    /// The vignette's reach.
    pub const VIGNETTE_REACH: [f32; 2] = [0.54, 0.62];
    /// The grain image's size (stretched over the felt) and its strongest alpha.
    pub const GRAIN: (u32, u32, f32) = (1024, 448, 18.0);
    /// The club watermark under the trick.
    pub const WATERMARK: f32 = 220.0;
    /// The watermark's opacity while the bid copy sits over it (16% x 0.3, about 5%).
    pub const WATERMARK_UNDER_COPY: f32 = 0.3;
    /// A seat plate's radius and padding (vertical, horizontal; 09 §5.2).
    pub const PLATE_RADIUS: f32 = 14.0;
    /// A seat plate's padding: vertical, horizontal.
    pub const PLATE_PAD: (f32, f32) = (8.0, 12.0);
    /// A seat plate's padding in a short window, where the felt is tight.
    pub const PLATE_PAD_COMPACT: (f32, f32) = (2.0, 8.0);
    /// A seat plate's padding in a narrow felt, where the partner's fan stacks under its name.
    pub const PLATE_PAD_NARROW: (f32, f32) = (4.0, 12.0);
    /// An opponent's fan: back width, height, step (09 §5.2).
    pub const FAN: [f32; 3] = [30.0, 42.0, 9.0];
    /// A fan's tilt per back about its middle, radians (09 §5.2).
    pub const FAN_TILT: f32 = 0.08;
    /// The winner ring: border width, inset outside the card (09 §4.4).
    pub const RING: [f32; 2] = [3.0, 6.0];
    /// The currently-winning outline: width, alpha of the gold (09 §5.3).
    pub const LEADING: [f32; 2] = [1.5, 0.45];
    /// The gather: offset toward the winner, rotation step (09 §4.4).
    pub const GATHER: [f32; 2] = [18.0, 0.05];
    /// The euchre stamp: text size, border, radius, padding.
    pub const STAMP: [f32; 4] = [44.0, 2.0, 12.0, 22.0];
    /// The stamp's turn, radians: it lands a little askew, like a hand stamp (09 §4.8).
    pub const STAMP_TURN: f32 = -0.07;
    /// The banter bubble's widest and its corner radius.
    pub const BUBBLE: [f32; 2] = [220.0, 12.0];
    /// Hover lift on a rack card.
    pub const HOVER_LIFT: f32 = -4.0;
    /// Disabled bid button opacity.
    pub const BID_OFF: f32 = 0.55;
    /// Disabled Continue opacity.
    pub const CONTINUE_OFF: f32 = 0.65;
}

/// Timings (06 §6, in seconds unless named ms).
pub mod timing {
    /// A played card's flight.
    pub const PLAY: f32 = 0.7;
    /// Card play + read pause before an automatic collect (06 §6.4).
    pub const COLLECT_AFTER: f64 = 2.2;
    /// Each collected card's flight.
    pub const COLLECT: f32 = 0.78;
    /// Delay between collected cards.
    pub const COLLECT_STAGGER: f64 = 0.045;
    /// Deal-in per card and its stagger.
    pub const DEAL_IN: f32 = 0.58;
    /// Deal-in stagger.
    pub const DEAL_STAGGER: f32 = 0.075;
    /// Deal-in start offset.
    pub const DEAL_OFFSET: f32 = -24.0;
    /// The rack's hover and selection lift.
    pub const LIFT: f32 = 0.18;
    /// The up card's pickup flight.
    pub const PICKUP: f32 = 0.85;
    /// An illegal-card message.
    pub const ILLEGAL: f64 = 3.2;
    /// An outcome banner.
    pub const BANNER: f64 = 4.5;
    /// Confetti fall.
    pub const CONFETTI: f32 = 2.4;
    /// Confetti stagger step (`(i % 6) * 70ms`).
    pub const CONFETTI_STAGGER: f32 = 0.07;
    /// The thinking dot's period.
    pub const THINKING: f64 = 1.2;
}

/// Felt type styles (06 §1.3).
pub mod ty {
    use super::{SERIF, TextStyle};
    /// Trump heading: serif 500 26px.
    pub const TRUMP: TextStyle = TextStyle::new(26.0, 500.0).family(SERIF).lh(31.0);
    /// Trump heading, compact.
    pub const TRUMP_COMPACT: TextStyle = TextStyle::new(21.0, 500.0).family(SERIF).lh(25.0);
    /// Eyebrow on the felt: 14px, 0.055em.
    pub const EYEBROW: TextStyle = TextStyle::new(14.0, 400.0).tracking(14.0 * 0.055);
    /// The led pill: 18px; `Led` 13px.
    pub const LED: TextStyle = TextStyle::new(18.0, 500.0);
    /// Tricks line: 18px, the numbers 25px.
    pub const TRICKS: TextStyle = TextStyle::new(18.0, 400.0).features("'tnum' 1");
    /// Seat initial letter: 16/600.
    pub const INITIAL: TextStyle = TextStyle::new(16.0, 600.0);
    /// Chip letter.
    pub const CHIP: TextStyle = TextStyle::new(12.0, 700.0);
    /// Bid-area eyebrow.
    pub const BID_EYEBROW: TextStyle = TextStyle::new(14.0, 500.0).tracking(14.0 * 0.055);
    /// Bid-area heading: serif 500 23px.
    pub const BID_H2: TextStyle = TextStyle::new(23.0, 500.0).family(SERIF).lh(29.0);
    /// Bid-area paragraph: 17px.
    pub const BID_P: TextStyle = TextStyle::new(17.0, 400.0).lh(23.0);
    /// Trick result line: 16px.
    pub const RESULT: TextStyle = TextStyle::new(16.0, 400.0).lh(22.0);
    /// Trick result line, won: 18px.
    pub const RESULT_WON: TextStyle = TextStyle::new(18.0, 600.0).lh(24.0);
    /// Card label line 2: 13px.
    pub const CARD_SUB: TextStyle = TextStyle::new(13.0, 400.0);
    /// Card label line 2 when selected (bold).
    pub const CARD_SUB_ON: TextStyle = TextStyle::new(13.0, 700.0);
    /// Bid buttons: 19/600.
    pub const BID: TextStyle = TextStyle::new(19.0, 600.0);
    /// Bid buttons, compact.
    pub const BID_COMPACT: TextStyle = TextStyle::new(16.0, 600.0);
    /// Go alone label: 17px.
    pub const ALONE: TextStyle = TextStyle::new(17.0, 500.0);
    /// Continue button: 650.
    pub const CONTINUE: TextStyle = TextStyle::new(17.0, 650.0);
    /// Score numerals: sans 600 36px, tabular.
    pub const SCORE: TextStyle = TextStyle::new(36.0, 600.0).features("'tnum' 1");
    /// Score numerals, compact.
    pub const SCORE_COMPACT: TextStyle = TextStyle::new(28.0, 600.0).features("'tnum' 1");
    /// Score labels.
    pub const SCORE_LABEL: TextStyle = TextStyle::new(14.0, 500.0);
    /// `First to 10`: 15px.
    pub const FIRST_TO: TextStyle = TextStyle::new(15.0, 400.0);
    /// Score card rank numeral: 9px in the viewBox.
    pub const SCORE_RANK: TextStyle = TextStyle::new(8.5, 600.0);
    /// Score card pip: 13px in the viewBox.
    pub const SCORE_PIP: TextStyle = TextStyle::new(12.0, 400.0).lh(12.0);
    /// Outcome banner: serif 600 24px.
    pub const BANNER: TextStyle = TextStyle::new(24.0, 600.0).family(SERIF);
    /// The deck tile's club: 48px.
    pub const DECK_CLUB: TextStyle = TextStyle::new(48.0, 400.0);
    /// Hand heading h1, compact.
    pub const HAND_H1_COMPACT: TextStyle = TextStyle::new(25.0, 500.0).family(SERIF);
    /// Instruction line, compact.
    pub const INSTRUCTION_COMPACT: TextStyle = TextStyle::new(16.0, 400.0).lh(20.0);
    /// Trick-slot caption (`Sitting out`): 15px.
    pub const SLOT_CAPTION: TextStyle = TextStyle::new(15.0, 400.0);
    /// The euchre stamp: serif 600 44px (09 §4.8).
    pub const STAMP: TextStyle = TextStyle::new(44.0, 600.0).family(SERIF).lh(50.0);
    /// The banter bubble: 14px, two lines.
    pub const BUBBLE: TextStyle = TextStyle::new(14.0, 400.0).lh(19.0);
}

#[cfg(test)]
mod tests {
    use super::hexes::*;

    #[test]
    fn every_felt_token_parses() {
        let mut all = vec![
            WINNER,
            SEAT_NAME_ACTIVE,
            INITIAL,
            INITIAL_PARTNER,
            INITIAL_ACTIVE,
            TRUMP_GLYPH,
            LONE,
            QUOTE_SPEAKER,
            TRICK_WON,
            RESULT,
            CHIP_BG,
            DEALER_TEXT,
            DEALER_EDGE,
            MAKER_TEXT,
            LED_PILL,
            SEAT_TINT,
            HAND_BORDER,
            CARD_LABEL,
            CARD_SUB,
            BUTTON_OFF_BG,
            BUTTON_OFF_TEXT,
            BUTTON_OFF_BORDER,
            BID_SECONDARY,
            BID_SECONDARY_BORDER,
            BID_SECONDARY_TEXT,
            BID_SECONDARY_HOVER,
            STACK,
            STACK_STRIPE,
            STACK_BORDER,
            STACK_EDGE_1,
            STACK_EDGE_2,
            DECK_BORDER,
            DECK_OUTLINE,
            DECK_LIGHT,
            DECK_DARK,
            DECK_CLUB,
            BANNER,
            BANNER_TEXT,
            BANNER_STAR,
            BANNER_SHADOW,
            SCORE_STROKE,
            SCORE_SPADE,
            SCORE_DIVIDER,
            FIRST_TO,
            PLAYED_EDGE_1,
            PLAYED_EDGE_2,
            PLAYED_DROP,
            UP_EDGE_1,
            UP_EDGE_2,
            UP_DROP,
            SELECTED_EDGE,
            SELECTED_DROP,
            FELT_HI,
            RAIL_HI,
            VIGNETTE,
            LAMP,
            SEAT_PLATE,
            SEAT_PLATE_ON,
            COOL,
            STAMP_INK,
            BUBBLE,
            BUBBLE_EDGE,
            BUBBLE_TEXT,
        ];
        all.extend(CONFETTI);
        all.extend(SPARK);
        for token in all {
            assert!(rune::color::hex(token).is_some(), "{token} does not parse");
        }
    }
}
