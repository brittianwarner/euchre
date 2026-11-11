//! Design tokens from 06-ui-spec §1 (plus the felt and wood rail of 06 §16.1 and 07 §9).
//!
//! Every colour, size, radius, spacing step and shadow the screens use lives here under a
//! name. Colours are written as the sRGB hex the web app uses and converted once into Rune's
//! linear [`Color`] in [`PAL`]. Screens never spell a hex value or a magic number of their own.

use std::sync::LazyLock;

use rune::color::hex;
use rune::tok::{Family, TextStyle};
use rune::{Color, El, Shadow};

pub mod felt;

pub use felt::FELT;

/// The serif family (`--font-serif`), registered from the bundled Source Serif 4 faces.
pub const SERIF: Family = Family::Named("Source Serif 4");
/// The sans slot name (`--font-sans`), registered from the bundled DM Sans faces.
pub const SANS_FAMILY: &str = "DM Sans";
/// The mono slot name. The web app shows no monospace text; it only fills Rune's mono slot.
pub const MONO_FAMILY: &str = "IBM Plex Mono";

/// Converts one sRGB hex token to Rune's linear colour. Tokens are literals checked by the
/// `every_token_parses` test, so the fallback never shows.
pub fn color(token: &str) -> Color {
    hex(token).unwrap_or(Color::BLACK)
}

/// The sRGB hex values of every 06 §1.2 colour token the desktop uses.
pub mod hexes {
    /// `--club-cream`: page background, dialogs, `/new`.
    pub const CREAM: &str = "#f4f3e9";
    /// `--club-ink`: body text on light surfaces.
    pub const INK: &str = "#233e32";
    /// `--club-forest`: the home hero panel.
    pub const FOREST: &str = "#153f36";
    /// `--club-lime`: highlights, the active nav pill, the hero button.
    pub const LIME: &str = "#d4ed9b";
    /// The hero button hover.
    pub const LIME_HOVER: &str = "#e3f6be";
    /// Text on the lime hero button.
    pub const LIME_TEXT: &str = "#143e32";
    /// Card white.
    pub const CARD_WHITE: &str = "#fffdf6";
    /// Soft paper.
    pub const PAPER: &str = "#fffdf5";
    /// Hairlines on light surfaces.
    pub const HAIRLINE: &str = "#dce1d4";
    /// The header divider and score-card hairline.
    pub const HAIRLINE_SOFT: &str = "#d3d9c9";
    /// The settings-button border.
    pub const HAIRLINE_STRONG: &str = "#cad3c1";
    /// The strong green button (`/new` submit, loading reconnect).
    pub const GREEN_BUTTON: &str = "#285641";
    /// Its hover.
    pub const GREEN_BUTTON_HOVER: &str = "#1a4431";
    /// Secondary text on light surfaces.
    pub const TEXT_SECONDARY: &str = "#58695b";
    /// Muted text on light surfaces (home intro paragraph two).
    pub const TEXT_MUTED: &str = "#72766a";
    /// Panel hover on light surfaces.
    pub const PANEL_HOVER: &str = "#e9eddf";
    /// Disabled control background.
    pub const DISABLED_BG: &str = "#dce3d3";
    /// Disabled control text.
    pub const DISABLED_TEXT: &str = "#52624b";
    /// Disabled control border.
    pub const DISABLED_BORDER: &str = "#bdc9b2";
    /// Focus ring (`:focus-visible`).
    pub const FOCUS: &str = "#87ac49";
    /// Focus ring on the felt.
    pub const FOCUS_FELT: &str = "#f5d98e";
    /// Focus ring inside the light rack.
    pub const FOCUS_RACK: &str = "#295744";
    /// Hero text.
    pub const HERO_TEXT: &str = "#f5f1e5";
    /// Hero eyebrow.
    pub const HERO_EYEBROW: &str = "#d5e7bb";
    /// Hero lede.
    pub const HERO_LEDE: &str = "#d2dcd4";
    /// Hero note.
    pub const HERO_NOTE: &str = "#a8bcb0";
    /// Hero bottom strip text.
    pub const HERO_MUTED: &str = "#9db3a4";
    /// Hero image caption.
    pub const HERO_CAPTION: &str = "#a1b9a6";
    /// Red suits in the hero strip.
    pub const HERO_SUIT_RED: &str = "#a8b893";
    /// Hero bottom-strip divider (alpha).
    pub const HERO_DIVIDER: &str = "#c6d7b92b";
    /// Hero dot grain.
    pub const HERO_GRAIN: &str = "#bdd4b1";
    /// The eyebrow dot halo (`0 0 0 4px #d4ed9b15`).
    pub const LIME_HALO: &str = "#d4ed9b15";
    /// Red suit accent on light surfaces.
    pub const SUIT_RED: &str = "#b44835";
    /// Hearts on the score cards.
    pub const SCORE_HEART: &str = "#aa342e";
    /// Rule-card background.
    pub const RULE_CARD: &str = "#e9ecdf";
    /// Rule-card number.
    pub const RULE_NUMBER: &str = "#6b7664";
    /// The table area background (`.table-layout`).
    pub const TABLE_BG: &str = "#17452f";
    /// Text on the table area.
    pub const TABLE_TEXT: &str = "#fff8e8";
    /// Text on the felt.
    pub const FELT_TEXT: &str = "#fffdf3";
    /// Light-green labels on the felt.
    pub const FELT_LABEL: &str = "#d9e6cb";
    /// Felt texture centre (06 §16).
    pub const FELT_CENTER: &str = "#245a49";
    /// Felt texture edge (06 §16).
    pub const FELT_EDGE: &str = "#10382c";
    /// Felt rim stroke (the "marked" ring, `rgba(218,230,176,.22)`).
    pub const FELT_RIM: &str = "#dae6b038";
    /// Felt watermark (`rgba(223,234,197,.16)`).
    pub const FELT_WATERMARK: &str = "#dfeac529";
    /// Wood rail, top of the gradient.
    pub const RAIL_TOP: &str = "#5a3a22";
    /// Wood rail, bottom of the gradient.
    pub const RAIL_BOTTOM: &str = "#3b2415";
    /// Brass line on the rail's inner edge.
    pub const BRASS: &str = "#b4a57a";
    /// The dark under-shadow below the rail.
    pub const RAIL_SHADOW: &str = "#153c2b";
    /// Primary-on-felt button background.
    pub const FELT_BUTTON: &str = "#f5e8c3";
    /// Primary-on-felt button border.
    pub const FELT_BUTTON_BORDER: &str = "#ead8aa";
    /// Primary-on-felt button hover.
    pub const FELT_BUTTON_HOVER: &str = "#fff0c7";
    /// Active-seat gold on the felt.
    pub const SEAT_ACTIVE: &str = "#e6d08c";
    /// Active seat wash (`#ffffff0a`).
    pub const SEAT_ACTIVE_BG: &str = "#ffffff0a";
    /// Notice box background.
    pub const NOTICE_BG: &str = "#fff1d9";
    /// Notice box border.
    pub const NOTICE_BORDER: &str = "#d5b68c";
    /// Global nav pill background (alpha).
    pub const NAV_BG: &str = "#15352eee";
    /// Global nav pill border (alpha).
    pub const NAV_BORDER: &str = "#aec29338";
    /// Global nav link.
    pub const NAV_LINK: &str = "#d9e3d4";
    /// Global nav hover (alpha).
    pub const NAV_HOVER: &str = "#d4ed9b1a";
    /// Home header link.
    pub const HOME_LINK: &str = "#42523e";
    /// Home header link hover.
    pub const HOME_LINK_HOVER: &str = "#e5e9d8";
    /// Home brand caption divider.
    pub const BRAND_CAPTION_BORDER: &str = "#a8b2a0";
    /// Dark scrapbook page background.
    pub const DARK_BG: &str = "#0b0906";
    /// Dark scrapbook text.
    pub const DARK_TEXT: &str = "#f2e8d5";
    /// Dark scrapbook muted text.
    pub const DARK_MUTED: &str = "#93876f";
    /// Dark scrapbook soft text.
    pub const DARK_SOFT: &str = "#c9b89a";
    /// Dark scrapbook accent gold.
    pub const DARK_GOLD: &str = "#e8c27a";
    /// Dark scrapbook card (`rgba(15,20,14,.6)`).
    pub const DARK_CARD: &str = "#0f140e99";
    /// Dark scrapbook green button.
    pub const DARK_BUTTON: &str = "#3d6b38";
    /// Dark scrapbook button text.
    pub const DARK_BUTTON_TEXT: &str = "#f4f7e8";
    /// Dark scrapbook error text.
    pub const DARK_ERROR: &str = "#e0a06e";
    /// Card shadow, light line.
    pub const SHADOW_CARD_LIGHT: &str = "#ffffff";
    /// Card shadow, near edge.
    pub const SHADOW_CARD_EDGE: &str = "#c4c7b9";
    /// Card shadow, soft drop.
    pub const SHADOW_CARD_DROP: &str = "#16382c24";
    /// Toast shadow.
    pub const SHADOW_TOAST: &str = "#10251d30";
}

/// Every colour token as a linear [`Color`], converted once.
pub struct Palette {
    /// See [`hexes::CREAM`].
    pub cream: Color,
    /// See [`hexes::INK`].
    pub ink: Color,
    /// See [`hexes::FOREST`].
    pub forest: Color,
    /// See [`hexes::LIME`].
    pub lime: Color,
    /// See [`hexes::LIME_HOVER`].
    pub lime_hover: Color,
    /// See [`hexes::LIME_TEXT`].
    pub lime_text: Color,
    /// See [`hexes::CARD_WHITE`].
    pub card_white: Color,
    /// See [`hexes::PAPER`].
    pub paper: Color,
    /// See [`hexes::HAIRLINE`].
    pub hairline: Color,
    /// See [`hexes::HAIRLINE_SOFT`].
    pub hairline_soft: Color,
    /// See [`hexes::HAIRLINE_STRONG`].
    pub hairline_strong: Color,
    /// See [`hexes::GREEN_BUTTON`].
    pub green_button: Color,
    /// See [`hexes::GREEN_BUTTON_HOVER`].
    pub green_button_hover: Color,
    /// See [`hexes::TEXT_SECONDARY`].
    pub text_secondary: Color,
    /// See [`hexes::TEXT_MUTED`].
    pub text_muted: Color,
    /// See [`hexes::PANEL_HOVER`].
    pub panel_hover: Color,
    /// See [`hexes::DISABLED_BG`].
    pub disabled_bg: Color,
    /// See [`hexes::DISABLED_TEXT`].
    pub disabled_text: Color,
    /// See [`hexes::DISABLED_BORDER`].
    pub disabled_border: Color,
    /// See [`hexes::FOCUS`].
    pub focus: Color,
    /// See [`hexes::FOCUS_FELT`].
    pub focus_felt: Color,
    /// See [`hexes::FOCUS_RACK`].
    pub focus_rack: Color,
    /// See [`hexes::HERO_TEXT`].
    pub hero_text: Color,
    /// See [`hexes::HERO_EYEBROW`].
    pub hero_eyebrow: Color,
    /// See [`hexes::HERO_LEDE`].
    pub hero_lede: Color,
    /// See [`hexes::HERO_NOTE`].
    pub hero_note: Color,
    /// See [`hexes::HERO_MUTED`].
    pub hero_muted: Color,
    /// See [`hexes::HERO_CAPTION`].
    pub hero_caption: Color,
    /// See [`hexes::HERO_SUIT_RED`].
    pub hero_suit_red: Color,
    /// See [`hexes::HERO_DIVIDER`].
    pub hero_divider: Color,
    /// See [`hexes::HERO_GRAIN`].
    pub hero_grain: Color,
    /// See [`hexes::LIME_HALO`].
    pub lime_halo: Color,
    /// See [`hexes::SUIT_RED`].
    pub suit_red: Color,
    /// See [`hexes::SCORE_HEART`].
    pub score_heart: Color,
    /// See [`hexes::RULE_CARD`].
    pub rule_card: Color,
    /// See [`hexes::RULE_NUMBER`].
    pub rule_number: Color,
    /// See [`hexes::TABLE_BG`].
    pub table_bg: Color,
    /// See [`hexes::TABLE_TEXT`].
    pub table_text: Color,
    /// See [`hexes::FELT_TEXT`].
    pub felt_text: Color,
    /// See [`hexes::FELT_LABEL`].
    pub felt_label: Color,
    /// See [`hexes::FELT_CENTER`].
    pub felt_center: Color,
    /// See [`hexes::FELT_EDGE`].
    pub felt_edge: Color,
    /// See [`hexes::FELT_RIM`].
    pub felt_rim: Color,
    /// See [`hexes::FELT_WATERMARK`].
    pub felt_watermark: Color,
    /// See [`hexes::RAIL_TOP`].
    pub rail_top: Color,
    /// See [`hexes::RAIL_BOTTOM`].
    pub rail_bottom: Color,
    /// See [`hexes::BRASS`].
    pub brass: Color,
    /// See [`hexes::RAIL_SHADOW`].
    pub rail_shadow: Color,
    /// See [`hexes::FELT_BUTTON`].
    pub felt_button: Color,
    /// See [`hexes::FELT_BUTTON_BORDER`].
    pub felt_button_border: Color,
    /// See [`hexes::FELT_BUTTON_HOVER`].
    pub felt_button_hover: Color,
    /// See [`hexes::SEAT_ACTIVE`].
    pub seat_active: Color,
    /// See [`hexes::SEAT_ACTIVE_BG`].
    pub seat_active_bg: Color,
    /// See [`hexes::NOTICE_BG`].
    pub notice_bg: Color,
    /// See [`hexes::NOTICE_BORDER`].
    pub notice_border: Color,
    /// See [`hexes::NAV_BG`].
    pub nav_bg: Color,
    /// See [`hexes::NAV_BORDER`].
    pub nav_border: Color,
    /// See [`hexes::NAV_LINK`].
    pub nav_link: Color,
    /// See [`hexes::NAV_HOVER`].
    pub nav_hover: Color,
    /// See [`hexes::HOME_LINK`].
    pub home_link: Color,
    /// See [`hexes::HOME_LINK_HOVER`].
    pub home_link_hover: Color,
    /// See [`hexes::BRAND_CAPTION_BORDER`].
    pub brand_caption_border: Color,
    /// See [`hexes::DARK_BG`].
    pub dark_bg: Color,
    /// See [`hexes::DARK_TEXT`].
    pub dark_text: Color,
    /// See [`hexes::DARK_MUTED`].
    pub dark_muted: Color,
    /// See [`hexes::DARK_SOFT`].
    pub dark_soft: Color,
    /// See [`hexes::DARK_GOLD`].
    pub dark_gold: Color,
    /// See [`hexes::DARK_CARD`].
    pub dark_card: Color,
    /// See [`hexes::DARK_BUTTON`].
    pub dark_button: Color,
    /// See [`hexes::DARK_BUTTON_TEXT`].
    pub dark_button_text: Color,
    /// See [`hexes::DARK_ERROR`].
    pub dark_error: Color,
    /// See [`hexes::SHADOW_CARD_LIGHT`].
    pub shadow_card_light: Color,
    /// See [`hexes::SHADOW_CARD_EDGE`].
    pub shadow_card_edge: Color,
    /// See [`hexes::SHADOW_CARD_DROP`].
    pub shadow_card_drop: Color,
    /// See [`hexes::SHADOW_TOAST`].
    pub shadow_toast: Color,
}

/// The palette, converted from [`hexes`] on first use.
pub static PAL: LazyLock<Palette> = LazyLock::new(|| {
    use hexes::*;
    Palette {
        cream: color(CREAM),
        ink: color(INK),
        forest: color(FOREST),
        lime: color(LIME),
        lime_hover: color(LIME_HOVER),
        lime_text: color(LIME_TEXT),
        card_white: color(CARD_WHITE),
        paper: color(PAPER),
        hairline: color(HAIRLINE),
        hairline_soft: color(HAIRLINE_SOFT),
        hairline_strong: color(HAIRLINE_STRONG),
        green_button: color(GREEN_BUTTON),
        green_button_hover: color(GREEN_BUTTON_HOVER),
        text_secondary: color(TEXT_SECONDARY),
        text_muted: color(TEXT_MUTED),
        panel_hover: color(PANEL_HOVER),
        disabled_bg: color(DISABLED_BG),
        disabled_text: color(DISABLED_TEXT),
        disabled_border: color(DISABLED_BORDER),
        focus: color(FOCUS),
        focus_felt: color(FOCUS_FELT),
        focus_rack: color(FOCUS_RACK),
        hero_text: color(HERO_TEXT),
        hero_eyebrow: color(HERO_EYEBROW),
        hero_lede: color(HERO_LEDE),
        hero_note: color(HERO_NOTE),
        hero_muted: color(HERO_MUTED),
        hero_caption: color(HERO_CAPTION),
        hero_suit_red: color(HERO_SUIT_RED),
        hero_divider: color(HERO_DIVIDER),
        hero_grain: color(HERO_GRAIN),
        lime_halo: color(LIME_HALO),
        suit_red: color(SUIT_RED),
        score_heart: color(SCORE_HEART),
        rule_card: color(RULE_CARD),
        rule_number: color(RULE_NUMBER),
        table_bg: color(TABLE_BG),
        table_text: color(TABLE_TEXT),
        felt_text: color(FELT_TEXT),
        felt_label: color(FELT_LABEL),
        felt_center: color(FELT_CENTER),
        felt_edge: color(FELT_EDGE),
        felt_rim: color(FELT_RIM),
        felt_watermark: color(FELT_WATERMARK),
        rail_top: color(RAIL_TOP),
        rail_bottom: color(RAIL_BOTTOM),
        brass: color(BRASS),
        rail_shadow: color(RAIL_SHADOW),
        felt_button: color(FELT_BUTTON),
        felt_button_border: color(FELT_BUTTON_BORDER),
        felt_button_hover: color(FELT_BUTTON_HOVER),
        seat_active: color(SEAT_ACTIVE),
        seat_active_bg: color(SEAT_ACTIVE_BG),
        notice_bg: color(NOTICE_BG),
        notice_border: color(NOTICE_BORDER),
        nav_bg: color(NAV_BG),
        nav_border: color(NAV_BORDER),
        nav_link: color(NAV_LINK),
        nav_hover: color(NAV_HOVER),
        home_link: color(HOME_LINK),
        home_link_hover: color(HOME_LINK_HOVER),
        brand_caption_border: color(BRAND_CAPTION_BORDER),
        dark_bg: color(DARK_BG),
        dark_text: color(DARK_TEXT),
        dark_muted: color(DARK_MUTED),
        dark_soft: color(DARK_SOFT),
        dark_gold: color(DARK_GOLD),
        dark_card: color(DARK_CARD),
        dark_button: color(DARK_BUTTON),
        dark_button_text: color(DARK_BUTTON_TEXT),
        dark_error: color(DARK_ERROR),
        shadow_card_light: color(SHADOW_CARD_LIGHT),
        shadow_card_edge: color(SHADOW_CARD_EDGE),
        shadow_card_drop: color(SHADOW_CARD_DROP),
        shadow_toast: color(SHADOW_TOAST),
    }
});

/// Radii (06 §1.4).
pub mod radius {
    /// Card art.
    pub const CARD: f32 = 7.0;
    /// Buttons.
    pub const BUTTON: f32 = 8.0;
    /// The hero's lime button.
    pub const HERO_BUTTON: f32 = 7.0;
    /// Home header links.
    pub const HOME_LINK: f32 = 6.0;
    /// Rule cards.
    pub const RULE_CARD: f32 = 10.0;
    /// The home hero.
    pub const HERO: f32 = 18.0;
    /// Dialogs and the `/new` panel.
    pub const PANEL: f32 = 16.0;
    /// The felt on the desktop table.
    pub const FELT: f32 = 18.0;
    /// The wood rail around the felt.
    pub const RAIL: f32 = 30.0;
    /// Pills (`999px`).
    pub const PILL: f32 = 999.0;
    /// Checkbox box.
    pub const CHECKBOX: f32 = 5.0;
}

/// Spacing and sizes (06 §1.4 and the per-screen paddings).
pub mod space {
    /// The 4 pt step.
    pub const XS: f32 = 4.0;
    /// The 8 pt step.
    pub const S: f32 = 8.0;
    /// 12 pt.
    pub const SM: f32 = 12.0;
    /// 16 pt.
    pub const M: f32 = 16.0;
    /// 24 pt.
    pub const L: f32 = 24.0;
    /// 32 pt.
    pub const XL: f32 = 32.0;
    /// 48 pt.
    pub const XXL: f32 = 48.0;
    /// Minimum tap target.
    pub const TAP: f32 = 44.0;
    /// Primary action height (small).
    pub const PRIMARY_MIN: f32 = 48.0;
    /// Primary action height (large: `/new` submit).
    pub const PRIMARY: f32 = 56.0;
    /// Home main padding top (the header height).
    pub const HOME_HEADER_H: f32 = 104.0;
    /// Home main side padding.
    pub const HOME_GUTTER: f32 = 28.0;
    /// Home header side padding.
    pub const HOME_HEADER_PX: f32 = 50.0;
    /// Home content max width.
    pub const HOME_MAX_W: f32 = 1460.0;
    /// Hero minimum height.
    pub const HERO_MIN_H: f32 = 620.0;
    /// Hero padding top.
    pub const HERO_PT: f32 = 70.0;
    /// Hero padding bottom.
    pub const HERO_PB: f32 = 96.0;
    /// Hero side padding as a fraction of its width (`5.3%`).
    pub const HERO_PX_FRAC: f32 = 0.053;
    /// Gap between the hero button's label and arrow.
    pub const HERO_BUTTON_GAP: f32 = 48.0;
    /// Hero button padding (vertical).
    pub const HERO_BUTTON_PY: f32 = 17.0;
    /// Hero button padding (horizontal).
    pub const HERO_BUTTON_PX: f32 = 23.0;
    /// The hero bottom strip's distance from the hero bottom.
    pub const HERO_STRIP_BOTTOM: f32 = 26.0;
    /// Bower stamp diameter.
    pub const STAMP: f32 = 100.0;
    /// Bower stamp ring width.
    pub const STAMP_RING: f32 = 6.0;
    /// Eyebrow dot diameter.
    pub const EYEBROW_DOT: f32 = 6.0;
    /// Eyebrow dot halo width.
    pub const EYEBROW_HALO: f32 = 4.0;
    /// Intro section max width.
    pub const INTRO_MAX_W: f32 = 1120.0;
    /// Intro column gap.
    pub const INTRO_GAP: f32 = 100.0;
    /// Intro vertical padding.
    pub const INTRO_PY: f32 = 85.0;
    /// Rule card padding.
    pub const RULE_CARD_PAD: f32 = 28.0;
    /// `/new` panel max width.
    pub const NEW_MAX_W: f32 = 640.0;
    /// `/new` panel vertical padding.
    pub const NEW_PY: f32 = 48.0;
    /// `/new` panel horizontal padding.
    pub const NEW_PX: f32 = 24.0;
    /// Checkbox box size.
    pub const CHECKBOX: f32 = 24.0;
    /// Nav pill inset from the window corner (`.6rem`).
    pub const NAV_INSET: f32 = 9.6;
    /// Nav pill padding.
    pub const NAV_PAD: f32 = 5.0;
    /// Nav pill gap.
    pub const NAV_GAP: f32 = 3.0;
    /// Nav link horizontal padding.
    pub const NAV_LINK_PX: f32 = 17.0;
    /// The wood rail's width around the felt.
    pub const RAIL: f32 = 14.0;
    /// The brass line on the rail's inner edge.
    pub const BRASS_LINE: f32 = 1.0;
    /// The rim stroke inset inside the felt.
    pub const FELT_RIM_INSET: f32 = 12.0;
    /// Toast stack inset from the window edge.
    pub const TOAST_INSET: f32 = 24.0;
    /// Toast max width.
    pub const TOAST_MAX_W: f32 = 420.0;
}

/// Card art geometry: the faces are 512x716 px.
pub mod card {
    /// Width / height of the card art.
    pub const ASPECT: f32 = 512.0 / 716.0;
    /// A hand card's width at full size.
    pub const HAND_W: f32 = 112.0;
    /// The gap between hand cards (06 §5.6).
    pub const HAND_GAP: f32 = 18.0;
}

/// The window (D25).
pub mod window {
    /// Default size in points.
    pub const SIZE: (f32, f32) = (1280.0, 820.0);
    /// Minimum size in points.
    pub const MIN: (f32, f32) = (720.0, 520.0);
    /// At or below this width the home page uses its single-column layout (06 §3, `<=760`).
    pub const NARROW_W: f32 = 760.0;
}

/// Home layout measures (06 §2.1, §3) that only the Home screen uses.
pub mod home {
    use std::f32::consts::PI;

    /// Page side padding at <=760 (`86px 14px 0`).
    pub const GUTTER_NARROW: f32 = 14.0;
    /// Header height at <=760.
    pub const HEADER_H_NARROW: f32 = 86.0;
    /// Header side inset inside the page gutter: 50px header padding less the 28px gutter.
    pub const HEADER_INSET: f32 = 22.0;
    /// Header side inset at <=760: 27px less the 14px gutter.
    pub const HEADER_INSET_NARROW: f32 = 13.0;
    /// `THE CARD CLUB` caption max width.
    pub const BRAND_CAPTION_W: f32 = 48.0;
    /// Hero grid: the copy column's share (`1.1fr / 1fr`).
    pub const HERO_COPY_FR: f32 = 1.1;
    /// Hero lede measure.
    pub const LEDE_MAX_W: f32 = 440.0;
    /// Hero minimum height in the single-column layout.
    pub const HERO_MIN_H_NARROW: f32 = 540.0;
    /// The art column height at <=760 (`card-scene` 310px).
    pub const ART_H_NARROW: f32 = 310.0;
    /// The art width as a share of its column (135%).
    pub const ART_SCALE: f32 = 1.35;
    /// The hero art's width over height (1200x1000).
    pub const ART_ASPECT: f32 = 1.2;
    /// Art column size before the first layout.
    pub const ART_FALLBACK: [f32; 2] = [560.0, 450.0];
    /// Mask ellipse radii as a share of the image. The web mask (`ellipse 64% 59%`, black 48%
    /// to transparent 95%) still shows a third of the picture at its edge; the desktop vignette
    /// reaches the forest exactly at the edge so no seam shows, with the same solid centre.
    pub const MASK_R: [f32; 2] = [0.5, 0.5];
    /// Mask stop where the picture starts to fade (the web's `black 48%` at this radius).
    pub const MASK_SOLID: f32 = 0.6;
    /// Mask stop where the picture is gone: the image edge.
    pub const MASK_CLEAR: f32 = 1.0;
    /// Caption inset from the art column's right and bottom.
    pub const CAPTION_INSET: [f32; 2] = [8.0, 4.0];
    /// Stamp inset from the art column's right and bottom.
    pub const STAMP_INSET: [f32; 2] = [36.0, 40.0];
    /// The stamp's turn (14deg).
    pub const STAMP_TURN: f32 = 14.0 * PI / 180.0;
    /// The disclosure `+` turn when open (45deg).
    pub const PLUS_TURN: f32 = PI / 4.0;
    /// Air above a rule card's title.
    pub const RULE_TITLE_GAP: f32 = 40.0;
    /// Bold weight inside body copy.
    pub const STRONG: f32 = 650.0;
}

/// Table layout measures (06 §5, §14, §16.1) that only the Table screen uses.
pub mod table {
    use std::f32::consts::PI;

    /// Hand area height: `clamp(280px, 42dvh, 420px)`.
    pub fn hand_h(height: f32) -> f32 {
        (height * 0.42).clamp(280.0, 420.0)
    }
    /// Table layout padding (`8px 24px`).
    pub const PAD: [f32; 2] = [24.0, 8.0];
    /// Felt radial stops: inner and outer radius as a share of the felt's longer side.
    pub const FELT_STOPS: [f32; 2] = [0.15, 0.72];
    /// The trick: slot centre (x, y as a share of the trick box) and tilt, in seat order
    /// left opponent, partner, right opponent, you (06 §5.4.3).
    pub const SLOTS: [(f32, f32, f32); 4] = [
        (0.28, 0.50, -7.0 * PI / 180.0),
        (0.43, 0.35, 3.0 * PI / 180.0),
        (0.72, 0.50, 7.0 * PI / 180.0),
        (0.57, 0.68, -3.0 * PI / 180.0),
    ];
    /// A played card's width as a share of the trick box, and its cap (06 §5.4.3).
    pub const PLAYED_W: (f32, f32) = (0.22, 145.0);
    /// Seat block width (06 §5.4.2).
    pub const SEAT_W: f32 = 180.0;
    /// Up-card height cap (06 §5.4.4).
    pub const UP_CARD_H: f32 = 190.0;
    /// Played-card winner outline width and offset.
    pub const WINNER_OUTLINE: [f32; 2] = [3.0, 5.0];
    /// Selected hand card lift.
    pub const SELECTED_LIFT: f32 = -8.0;
    /// Rack label block reserved under each card.
    pub const LABEL_H: f32 = 46.0;
    /// Rack max width (06 §5.6).
    pub const RACK_MAX_W: f32 = 850.0;
    /// Hand area max width (06 §5.1).
    pub const HAND_MAX_W: f32 = 1100.0;
    /// Felt button minimum width (06 §5.5).
    pub const BUTTON_MIN_W: f32 = 230.0;
    /// Felt button gap between label and arrow.
    pub const BUTTON_GAP: f32 = 18.0;
    /// Loading screen gap and padding (06 §14.1).
    pub const LOADING_GAP: f32 = 20.0;
    /// Loading copy measure.
    pub const LOADING_MAX_W: f32 = 520.0;
}

/// One CSS box-shadow layer: `(x, y, blur, spread, sRGB hex)`.
pub type ShadowLayer = (f32, f32, f32, f32, &'static str);

/// Elevation tokens (06 §1.4).
pub mod shadow {
    use super::ShadowLayer;
    /// Hand card artwork: `0 1px 0 #fff, 0 3px 0 #c4c7b9, 0 7px 13px #16382c24`.
    pub const HAND_CARD: [ShadowLayer; 3] = [
        (0.0, 1.0, 0.0, 0.0, super::hexes::SHADOW_CARD_LIGHT),
        (0.0, 3.0, 0.0, 0.0, super::hexes::SHADOW_CARD_EDGE),
        (0.0, 7.0, 13.0, 0.0, super::hexes::SHADOW_CARD_DROP),
    ];
    /// The settings panel / toast elevation: `-16px 0 48px #10251d30` turned downward.
    pub const TOAST: [ShadowLayer; 1] = [(0.0, 12.0, 40.0, 0.0, super::hexes::SHADOW_TOAST)];
    /// The rail's dark under-shadow: `0 3px 0 #153c2b`.
    pub const RAIL: [ShadowLayer; 2] = [
        (0.0, 3.0, 0.0, 0.0, super::hexes::RAIL_SHADOW),
        (0.0, 18.0, 36.0, 0.0, "#0000004d"),
    ];
}

/// Adds every layer of a shadow token to an element.
pub fn shadows(mut e: El, layers: &[ShadowLayer]) -> El {
    for &(x, y, blur, spread, token) in layers {
        e = e.box_shadow(Shadow {
            x,
            y,
            blur,
            spread,
            color: color(token),
            inset: false,
        });
    }
    e
}

/// Motion tokens (09 §3).
pub mod motion;

/// Type scale (06 §1.3). Sizes that clamp on the viewport are functions of the width.
pub mod ty {
    use super::{SERIF, TextStyle};

    /// Letter spacing in points for a CSS `em` value at `size`.
    pub const fn em(size: f32, em: f32) -> f32 {
        size * em
    }

    /// CSS `clamp(min, vw * width, max)`.
    pub fn clamp_vw(width: f32, min: f32, vw: f32, max: f32) -> f32 {
        (width * vw / 100.0).clamp(min, max)
    }

    /// Home h1: serif 500, `clamp(52px, 6.2vw, 94px)`, line height 1.02, -0.055em.
    pub fn home_h1(width: f32) -> TextStyle {
        let size = clamp_vw(width, 52.0, 6.2, 94.0);
        TextStyle::new(size, 500.0)
            .family(SERIF)
            .lh(size * 1.02)
            .tracking(em(size, -0.055))
    }

    /// Section h2 (home): serif 500, `clamp(34px, 3.4vw, 48px)`, line height 1.12, -0.045em.
    pub fn section_h2(width: f32) -> TextStyle {
        let size = clamp_vw(width, 34.0, 3.4, 48.0);
        TextStyle::new(size, 500.0)
            .family(SERIF)
            .lh(size * 1.12)
            .tracking(em(size, -0.045))
    }

    /// Rule card h3: serif 500 26/1.2, -0.025em.
    pub const RULE_H3: TextStyle = TextStyle::new(26.0, 500.0)
        .family(SERIF)
        .lh(26.0 * 1.2)
        .tracking(em(26.0, -0.025));
    /// Eyebrow: sans 600 10px, 0.18em, uppercase (callers uppercase the text).
    pub const EYEBROW: TextStyle = TextStyle::new(10.0, 600.0).tracking(em(10.0, 0.18));
    /// Brand wordmark `euchre`: serif 600 38px, -0.07em.
    pub const BRAND: TextStyle = TextStyle::new(38.0, 600.0)
        .family(SERIF)
        .tracking(em(38.0, -0.07));
    /// The club after the wordmark.
    pub const BRAND_CLUB: TextStyle = TextStyle::new(23.0, 600.0).family(SERIF);
    /// Footer wordmark: serif 600 32px.
    pub const BRAND_FOOTER: TextStyle = TextStyle::new(32.0, 600.0)
        .family(SERIF)
        .tracking(em(32.0, -0.07));
    /// Footer wordmark club.
    pub const BRAND_FOOTER_CLUB: TextStyle = TextStyle::new(20.0, 600.0).family(SERIF);
    /// The `THE CARD CLUB` caption: 8px/600, 0.18em.
    pub const BRAND_CAPTION: TextStyle =
        TextStyle::new(8.0, 600.0).lh(10.0).tracking(em(8.0, 0.18));
    /// `/new` h1: serif 500 38px, line height 1.2.
    pub const NEW_H1: TextStyle = TextStyle::new(38.0, 500.0).family(SERIF).lh(38.0 * 1.2);
    /// `/new` lede: 18px/1.5.
    pub const NEW_LEDE: TextStyle = TextStyle::new(18.0, 400.0).lh(27.0);
    /// `/new` checkbox label: 20px.
    pub const CHECK_LABEL: TextStyle = TextStyle::new(20.0, 500.0).lh(26.0);
    /// `/new` checkbox detail (`small`).
    pub const CHECK_DETAIL: TextStyle = TextStyle::new(15.0, 400.0).lh(21.0);
    /// `/new` submit: 20px/600.
    pub const SUBMIT: TextStyle = TextStyle::new(20.0, 600.0);
    /// Body text on light pages.
    pub const BODY: TextStyle = TextStyle::new(16.0, 400.0).lh(24.0);
    /// Small body text.
    pub const SMALL: TextStyle = TextStyle::new(13.0, 400.0).lh(20.0);
    /// Hero lede: 15px/1.75.
    pub const HERO_LEDE: TextStyle = TextStyle::new(15.0, 400.0).lh(15.0 * 1.75);
    /// Hero button: 14px/650.
    pub const HERO_BUTTON: TextStyle = TextStyle::new(14.0, 650.0);
    /// Hero secondary link: 12px.
    pub const HERO_LINK: TextStyle = TextStyle::new(12.0, 500.0);
    /// Hero note: 10px.
    pub const HERO_NOTE: TextStyle = TextStyle::new(10.0, 400.0);
    /// Hero caption: 8px, 0.22em.
    pub const HERO_CAPTION: TextStyle = TextStyle::new(8.0, 500.0).tracking(em(8.0, 0.22));
    /// Hero bottom strip: 8px, 0.16em.
    pub const HERO_STRIP: TextStyle = TextStyle::new(8.0, 500.0).tracking(em(8.0, 0.16));
    /// Hero bottom-strip suits: 18px, 7px spacing.
    pub const HERO_SUITS: TextStyle = TextStyle::new(18.0, 400.0).tracking(7.0);
    /// Stamp small lines: 7px/700, 0.11em.
    pub const STAMP_LINE: TextStyle = TextStyle::new(7.0, 700.0).tracking(em(7.0, 0.11));
    /// Stamp `J`: serif 36px.
    pub const STAMP_J: TextStyle = TextStyle::new(36.0, 500.0).family(SERIF).lh(38.0);
    /// Stamp heart: 25px.
    pub const STAMP_HEART: f32 = 25.0;
    /// Intro paragraph one.
    pub const INTRO_P: TextStyle = TextStyle::new(16.0, 400.0).lh(26.0);
    /// Intro paragraph two (muted, 13px).
    pub const INTRO_P_MUTED: TextStyle = TextStyle::new(13.0, 400.0).lh(21.0);
    /// Rule card number: 10px.
    pub const RULE_NUMBER: TextStyle = TextStyle::new(10.0, 500.0);
    /// Rule card suit glyph: 36px.
    pub const RULE_SUIT: TextStyle = TextStyle::new(36.0, 400.0).lh(36.0);
    /// Rule card body.
    pub const RULE_BODY: TextStyle = TextStyle::new(15.0, 400.0).lh(23.0);
    /// Disclosure summary.
    pub const DISCLOSURE: TextStyle = TextStyle::new(17.0, 600.0);
    /// Footer tagline: 11px.
    pub const FOOTER_TAG: TextStyle = TextStyle::new(11.0, 400.0);
    /// Footer link.
    pub const FOOTER_LINK: TextStyle = TextStyle::new(14.0, 600.0);
    /// Nav links: 12px/550.
    pub const NAV: TextStyle = TextStyle::new(12.0, 550.0);
    /// Nav club glyph: 20px.
    pub const NAV_CLUB: TextStyle = TextStyle::new(20.0, 500.0);
    /// Buttons on the felt: 17px/600.
    pub const FELT_BUTTON: TextStyle = TextStyle::new(17.0, 600.0);
    /// Loading club glyph: 54px.
    pub const LOADING_CLUB: TextStyle = TextStyle::new(54.0, 400.0);
    /// Loading h1 (hand heading scale): serif 500 28px.
    pub const LOADING_H1: TextStyle = TextStyle::new(28.0, 500.0).family(SERIF);
    /// Loading body: 19px.
    pub const LOADING_P: TextStyle = TextStyle::new(19.0, 400.0);
    /// Loading actions: 18px/600.
    pub const LOADING_BUTTON: TextStyle = TextStyle::new(18.0, 600.0);
    /// Hand heading h1: serif 500 28px.
    pub const HAND_H1: TextStyle = TextStyle::new(28.0, 500.0).family(SERIF);
    /// Instruction line: 17px/1.3.
    pub const INSTRUCTION: TextStyle = TextStyle::new(17.0, 400.0).lh(17.0 * 1.3);
    /// Card label: 15px/600.
    pub const CARD_LABEL: TextStyle = TextStyle::new(15.0, 600.0);
    /// Seat name: 17px/600.
    pub const SEAT_NAME: TextStyle = TextStyle::new(17.0, 600.0);
    /// Seat detail: 14px.
    pub const SEAT_DETAIL: TextStyle = TextStyle::new(14.0, 400.0);
    /// Dark scrapbook page heading.
    pub const DARK_H1: TextStyle = TextStyle::new(38.0, 500.0).family(SERIF);
    /// Dark scrapbook body.
    pub const DARK_BODY: TextStyle = TextStyle::new(17.0, 400.0).family(SERIF).lh(26.0);
    /// Toast title.
    pub const TOAST: TextStyle = TextStyle::new(15.0, 600.0);
    /// Notice text: 17px.
    pub const NOTICE: TextStyle = TextStyle::new(17.0, 400.0).lh(23.0);
    /// Scrapbook section heading (a card's h2).
    pub const DARK_H2: TextStyle = TextStyle::new(24.0, 500.0).family(SERIF).lh(30.0);
    /// A stat tile's number (06 §17.1: 1.35rem/600).
    pub const STAT_VALUE: TextStyle = TextStyle::new(22.0, 600.0);
    /// A stat tile's label (.72rem).
    pub const STAT_LABEL: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);
    /// Small scrapbook text: dates, counts, hints.
    pub const DARK_SMALL: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);
    /// A form label on the scrapbook pages.
    pub const DARK_LABEL: TextStyle = TextStyle::new(15.0, 600.0).lh(20.0);
    /// What a player types into a scrapbook field.
    pub const DARK_INPUT: TextStyle = TextStyle::new(16.0, 400.0).family(SERIF).lh(24.0);
    /// A pill: outcome chips, segmented choices, presets.
    pub const PILL: TextStyle = TextStyle::new(13.0, 600.0);
    /// The big score line on a match page.
    pub const DARK_SCORE: TextStyle = TextStyle::new(22.0, 500.0).family(SERIF).lh(30.0);
}

#[cfg(test)]
mod tests {
    use super::hexes::*;

    #[test]
    fn every_token_parses() {
        let all = [
            CREAM,
            INK,
            FOREST,
            LIME,
            LIME_HOVER,
            LIME_TEXT,
            CARD_WHITE,
            PAPER,
            HAIRLINE,
            HAIRLINE_SOFT,
            HAIRLINE_STRONG,
            GREEN_BUTTON,
            GREEN_BUTTON_HOVER,
            TEXT_SECONDARY,
            TEXT_MUTED,
            PANEL_HOVER,
            DISABLED_BG,
            DISABLED_TEXT,
            DISABLED_BORDER,
            FOCUS,
            FOCUS_FELT,
            FOCUS_RACK,
            HERO_TEXT,
            HERO_EYEBROW,
            HERO_LEDE,
            HERO_NOTE,
            HERO_MUTED,
            HERO_CAPTION,
            HERO_SUIT_RED,
            HERO_DIVIDER,
            HERO_GRAIN,
            LIME_HALO,
            SUIT_RED,
            SCORE_HEART,
            RULE_CARD,
            RULE_NUMBER,
            TABLE_BG,
            TABLE_TEXT,
            FELT_TEXT,
            FELT_LABEL,
            FELT_CENTER,
            FELT_EDGE,
            FELT_RIM,
            FELT_WATERMARK,
            RAIL_TOP,
            RAIL_BOTTOM,
            BRASS,
            RAIL_SHADOW,
            FELT_BUTTON,
            FELT_BUTTON_BORDER,
            FELT_BUTTON_HOVER,
            SEAT_ACTIVE,
            SEAT_ACTIVE_BG,
            NOTICE_BG,
            NOTICE_BORDER,
            NAV_BG,
            NAV_BORDER,
            NAV_LINK,
            NAV_HOVER,
            HOME_LINK,
            HOME_LINK_HOVER,
            BRAND_CAPTION_BORDER,
            DARK_BG,
            DARK_TEXT,
            DARK_MUTED,
            DARK_SOFT,
            DARK_GOLD,
            DARK_CARD,
            DARK_BUTTON,
            DARK_BUTTON_TEXT,
            DARK_ERROR,
            SHADOW_CARD_LIGHT,
            SHADOW_CARD_EDGE,
            SHADOW_CARD_DROP,
            SHADOW_TOAST,
        ];
        for token in all {
            assert!(rune::color::hex(token).is_some(), "{token} does not parse");
        }
        for (_, _, _, _, token) in super::shadow::HAND_CARD
            .iter()
            .chain(super::shadow::TOAST.iter())
            .chain(super::shadow::RAIL.iter())
        {
            assert!(rune::color::hex(token).is_some(), "{token} does not parse");
        }
    }

    #[test]
    fn home_type_clamps_like_css() {
        assert_eq!(super::ty::home_h1(400.0).size, 52.0);
        assert_eq!(super::ty::home_h1(4000.0).size, 94.0);
        assert!((super::ty::section_h2(1280.0).size - 43.52).abs() < 0.01);
    }
}
