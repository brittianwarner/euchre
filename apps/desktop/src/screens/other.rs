//! The dark scrapbook screens (06 §17, §18): Your games, one match, and Settings, on the
//! `euchre_profile_model` shapes the profile returns.
//!
//! The desktop knows who is playing from its device enrollment, so the web's identity gate and
//! "Playing as" line have no part here. List rows rise in on a 30 ms stagger (09 §5.7).

use euchre_profile_model::Pace;
use euchre_profile_model::{
    ClientHandReplay, ClientMatchReplay, PERSONA_PRESETS, PersonaPatch, PersonaPatchEntry,
    PersonaPreset, ProfileSettings, ProfileStats, SettingsPatch, TablePatch, default_settings,
};
use euchre_protocol::{
    DIFFICULTIES, Difficulty, GameStatus, HOUSE_PROMPT_MAX_CHARS, HandResult, MatchOutcome,
    PERSONA_BLURB_MAX_CHARS, PERSONA_NAME_MAX_CHARS, PERSONA_PROMPT_MAX_CHARS, Suit,
};
use rune::{
    Color, Cursor, Cx, Drag, El, LocalTime, ScrollSpec, Span, Stagger, TextEdit, Transition, Tx,
    col, edit_access, el, row, text, text_input,
};

use crate::app::{AppState, Route};
use crate::assets::{CARD_LICENSE, CREDITS};
use crate::net::bridge::{Bridge, Intent, SettingsReply};
use crate::net::{RecentTable, RecentTables, sim};
use crate::screens::notice;
use crate::theme::{PAL, motion, radius, space, ty};

/* ------------------------------------------------------------------------ */
/* The typed profile seam                                                    */
/* ------------------------------------------------------------------------ */

/// One match's detail as the match screen reads it: the profile's replay.
pub type MatchDetailData = ClientMatchReplay;
/// A change to the profile settings, as the Settings screen builds it.
pub type SettingsEdit = SettingsPatch;

/// Reads a match detail reply.
pub fn parse_match_detail(replay: ClientMatchReplay) -> Result<MatchDetailData, String> {
    Ok(replay)
}

/// Reads a settings reply (`getSettings`, `updateSettings`) into the screen's form.
pub fn parse_settings(reply: SettingsReply) -> Result<SettingsData, String> {
    Ok(SettingsData::from_reply(reply))
}

/// The wire form of a settings change (`Intent::PatchSettings`).
pub fn settings_patch_value(edit: &SettingsEdit) -> SettingsPatch {
    edit.clone()
}

/// The Settings screen's data: what the profile saved and the form being edited.
#[derive(Clone, Debug)]
pub struct SettingsData {
    /// The settings as the profile last returned them.
    pub saved: ProfileSettings,
    /// The form.
    pub draft: SettingsDraft,
    /// A save is in flight (or failed, when the settings error is set).
    pub saving: bool,
    /// The last reply was a save: show "Saved." until the next change.
    pub just_saved: bool,
}

/// The Settings form (06 §18).
#[derive(Clone, Debug)]
pub struct SettingsDraft {
    /// The house prompt.
    pub house_prompt: TextEdit,
    /// Seats 1, 2, 3.
    pub personas: [PersonaDraft; 3],
    /// Table pacing.
    pub pace: Pace,
    /// Let opponents talk at the table.
    pub banter: bool,
    /// Show each seat's reasoning after a hand.
    pub show_rationale: bool,
}

/// One seat's persona in the form.
#[derive(Clone, Debug)]
pub struct PersonaDraft {
    /// Display name.
    pub name: TextEdit,
    /// One-line blurb.
    pub blurb: TextEdit,
    /// Character prompt.
    pub prompt: TextEdit,
    /// Playing strength.
    pub difficulty: Difficulty,
    /// How eagerly they bid.
    pub aggression: f64,
    /// How much they gamble on a call.
    pub risk: f64,
    /// How much they talk.
    pub chattiness: f64,
}

impl SettingsData {
    /// The form for these saved settings.
    pub fn from_reply(reply: SettingsReply) -> SettingsData {
        let s = &reply.settings;
        let personas = s.personas.clone().map(|p| PersonaDraft {
            name: TextEdit::new(p.name),
            blurb: TextEdit::new(p.blurb),
            prompt: TextEdit::new(p.prompt),
            difficulty: p.difficulty,
            aggression: p.aggression,
            risk: p.risk,
            chattiness: p.chattiness,
        });
        let draft = SettingsDraft {
            house_prompt: TextEdit::new(s.house_prompt.clone()),
            personas,
            pace: s.table.pace,
            banter: s.table.banter,
            show_rationale: s.table.show_rationale,
        };
        SettingsData {
            saved: *reply.settings,
            draft,
            saving: false,
            just_saved: reply.saved,
        }
    }
}

impl SettingsDraft {
    /// The whole form as one patch, as the web form posts it. Sound and reduce motion come from
    /// this device's own switches, so the profile keeps the same choice.
    pub fn patch(&self, sound: bool, reduce_motion: bool) -> SettingsPatch {
        let personas = self
            .personas
            .iter()
            .zip(euchre_profile_model::AI_SEATS)
            .map(|(p, seat)| PersonaPatchEntry {
                seat,
                patch: PersonaPatch {
                    name: Some(p.name.text.trim().to_owned()),
                    blurb: Some(p.blurb.text.trim().to_owned()),
                    prompt: Some(p.prompt.text.clone()),
                    difficulty: Some(p.difficulty.as_str().to_owned()),
                    aggression: Some(p.aggression),
                    risk: Some(p.risk),
                    chattiness: Some(p.chattiness),
                    ..PersonaPatch::default()
                },
            })
            .collect();
        SettingsPatch {
            house_prompt: Some(self.house_prompt.text.clone()),
            table: Some(TablePatch {
                pace: Some(self.pace),
                banter: Some(self.banter),
                show_rationale: Some(self.show_rationale),
                sound: Some(sound),
                reduce_motion: Some(reduce_motion),
            }),
            personas: Some(personas),
        }
    }
}

/* ------------------------------------------------------------------------ */
/* Copy                                                                      */
/* ------------------------------------------------------------------------ */

/// The brand line over every scrapbook page.
pub const BRAND: &str = "EUCHRE";
/// Games heading.
pub const GAMES_H1: &str = "Your games";
/// Games load failure. The web's "Try refreshing the page." has no meaning in a window with no
/// page to refresh, so the notice carries a [`TRY_AGAIN`] link instead.
pub const GAMES_FAILED: &str = "Couldn't open your scrapbook just now.";
/// Loads the screen's data again after a failure.
pub const TRY_AGAIN: &str = "Try again";
/// The stats grid's label.
pub const STATS_LABEL: &str = "Lifetime stats";
/// The pager's label.
pub const PAGER_LABEL: &str = "Older and newer games";
/// Pager: back toward the newest games.
pub const NEWER: &str = "← Newer games";
/// Pager: on to older games.
pub const OLDER: &str = "Older games →";
/// A page with no rows.
pub const EMPTY_PAGE: &str = "No more games on this page.";
/// Empty state heading.
pub const EMPTY_H2: &str = "Nothing here yet";
/// Empty state copy.
pub const EMPTY_P: &str = "Play your first hand and it'll land right here — the date, the final score, whether you sent them home early, all of it. Think of this as the scrapbook on top of the fridge.";
/// Empty state action.
pub const EMPTY_CTA: &str = "Deal a hand";
/// Match detail back link.
pub const BACK_TO_GAMES: &str = "← Back to your games";
/// One match could not be read (the web shows its error page here).
pub const MATCH_FAILED: &str =
    "Couldn't open this game just now. Go back to your games and try it again.";
/// The match totals grid's label.
pub const TOTALS_LABEL: &str = "Match totals";
/// Settings heading.
pub const SETTINGS_H1: &str = "Settings";
/// Settings load failure (see [`GAMES_FAILED`]).
pub const SETTINGS_FAILED: &str = "Couldn't open your settings just now.";
/// The disclaimer box (06 §18, verbatim; the middle sentence is bold).
pub const DISCLAIMER: &str = "What this does and doesn't do: everything below shapes how your three opponents sound and how readily they bid or gamble. It cannot change the rules of euchre, let a seat see cards it shouldn't, or make an illegal move legal — those are enforced by the game itself and no amount of text here reaches them. Treat these prompts the way you'd treat a note passed to an actor, not a rulebook.";
/// The bold part of [`DISCLAIMER`].
pub const DISCLAIMER_BOLD: &str = "It cannot change the rules of euchre, let a seat see cards it shouldn't, or make an illegal move legal";
/// After a save.
pub const SAVED: &str = "Saved. Your next match will use these.";
/// A save the profile refused.
pub const SAVE_FAILED: &str =
    "Couldn’t save that — one of the fields may be too long or an odd value.";
/// The save button.
pub const SAVE: &str = "Save changes";
/// The table section heading.
pub const TABLE_H2: &str = "The table";
/// The device switches' group: they apply at once, unlike the fields above them that wait
/// for Save (desktop-authored).
pub const DEVICE_H3: &str = "On this computer";
/// Under [`DEVICE_H3`].
pub const DEVICE_HINT: &str = "These change right away. No need to save.";
/// The table section hint.
pub const TABLE_HINT: &str = "How the game presents itself. Nothing here touches how anyone plays.";
/// The pace choice.
pub const PACE: &str = "Pace";
/// Banter.
pub const BANTER: &str = "Let opponents talk at the table";
/// Reasoning.
pub const RATIONALE: &str = "Show each seat's reasoning after a hand";
/// The house prompt heading.
pub const HOUSE_H2: &str = "House prompt";
/// The house prompt hint.
pub const HOUSE_HINT: &str =
    "Sets the overall mood of the table — applies quietly to all three opponents at once.";
/// The persona cards' headings, seats 1, 2, 3.
pub const SEAT_HEADINGS: [&str; 3] = ["West", "North · your partner", "East"];
/// Name field.
pub const NAME: &str = "Name";
/// Blurb field.
pub const BLURB: &str = "One-line blurb (shown at the table)";
/// Prompt field.
pub const PROMPT: &str = "Character prompt — untrusted text, style only";
/// Difficulty field.
pub const DIFFICULTY: &str = "Difficulty";
/// The presets line.
pub const PRESETS: &str = "One-tap starting points — fills in the blurb, prompt and dials below, doesn't save until you do:";
/// The difficulty explainers (06 §18), rookie, casual, expert.
pub const DIFFICULTY_NOTES: [&str; 3] = [
    "Plays a little unpredictably. Easygoing, forgiving of mistakes at the table.",
    "Solid, sensible play most of the time — the middle of the road.",
    "Weighs every legal option every time. The strongest seat at the table.",
];

/// The credits section heading.
pub const ABOUT_H2: &str = "About";
/// The card-art license disclosure, closed.
pub const SHOW_LICENSE: &str = "Show the card art license";
/// The card-art license disclosure, open.
pub const HIDE_LICENSE: &str = "Hide the card art license";

/// Max page width (42rem).
const PAGE_MAX_W: f32 = 672.0;
/// The gap between list rows entering (09 §5.7).
const STAGGER_GAP: f32 = 0.03;
/// How far a list row rises as it enters.
const STAGGER_RISE: f32 = 6.0;
/// A dial's step.
const DIAL_STEP: f64 = 0.05;
/// A dial's track height and its knob.
const DIAL_TRACK: (f32, f32) = (6.0, 18.0);
/// A counter turns gold past this share of its limit (the web's `near`).
const NEAR: f64 = 0.9;

/* ------------------------------------------------------------------------ */
/* Shared pieces                                                             */
/* ------------------------------------------------------------------------ */

fn page(key: &str, children: Vec<El>) -> El {
    let body = col()
        .key(format!("{key}-page"))
        .w_full()
        .max_w(PAGE_MAX_W)
        .self_center()
        .px(space::L)
        .pt(space::XXL + space::L)
        .pb(space::XXL + space::M)
        .gap(space::L)
        .children(children);
    col()
        .key(key.to_owned())
        .w_full()
        .h_full()
        .fill(PAL.dark_bg)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(body)
}

fn brand() -> El {
    text(BRAND).font(ty::EYEBROW).color(PAL.dark_muted)
}

fn h1(key: &str, line: impl Into<String>) -> El {
    text(line.into())
        .key(key.to_owned())
        .font(ty::DARK_H1)
        .color(PAL.dark_text)
        .role("heading")
        .level(1)
}

fn h2(line: impl Into<String>) -> El {
    text(line.into())
        .font(ty::DARK_H2)
        .color(PAL.dark_text)
        .role("heading")
        .level(2)
}

fn body(line: impl Into<String>) -> El {
    text(line.into())
        .font(ty::DARK_BODY)
        .color(PAL.dark_soft)
        .wrap()
}

fn small(line: impl Into<String>) -> El {
    text(line.into())
        .font(ty::DARK_SMALL)
        .color(PAL.dark_muted)
        .wrap()
}

fn hairline() -> Color {
    PAL.dark_muted.alpha(0.35)
}

fn dark_button(key: &str, label: &'static str, go: Route) -> El {
    row()
        .key(key.to_owned())
        .min_h(space::PRIMARY_MIN)
        .px(space::L)
        .center()
        .radius(radius::BUTTON)
        .fill(PAL.dark_button)
        .hover(PAL.dark_button.mix(PAL.dark_text, 0.12))
        .hover_motion(motion::HOVER, motion::HOVER)
        .press_scale(0.96)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label(label)
        .on_press_tx(move |s: &mut AppState, tx| s.navigate(go.clone(), tx))
        .child(
            text(label)
                .font(ty::LOADING_BUTTON)
                .color(PAL.dark_button_text),
        )
}

fn link(key: &str, label: &'static str, go: impl Fn(&mut AppState, &mut Tx) + 'static) -> El {
    text(label)
        .key(key.to_owned())
        .font(ty::DARK_BODY)
        .color(PAL.dark_gold)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label(label)
        .min_h(space::TAP)
        .center_y()
        .on_press_tx(go)
}

fn card(key: String) -> El {
    col()
        .key(key)
        .w_full()
        .gap(space::S)
        .padding(space::M + space::XS)
        .radius(radius::BUTTON + 4.0)
        .fill(PAL.dark_card)
        .border(1.0, PAL.dark_muted.alpha(0.18))
}

/// A list row's entrance: it rises 6 pt and fades in, `i` of `n` on a 30 ms stagger.
fn staggered(e: El, i: usize, n: usize) -> El {
    e.transition(Transition::offset(0.0, STAGGER_RISE).combined(Transition::opacity()))
        .enter_delay(Stagger::new(STAGGER_GAP).delay(i, n))
}

/// A grid of number tiles, three to a row.
fn tiles(key: &str, label: &str, items: Vec<(String, String)>) -> El {
    let mut grid = col()
        .key(key.to_owned())
        .w_full()
        .gap(space::S)
        .role("group")
        .label(label.to_owned());
    for (r, chunk) in items.chunks(3).enumerate() {
        let mut line = row().w_full().gap(space::S);
        for (c, (value, name)) in chunk.iter().enumerate() {
            line = line.child(
                col()
                    .key(format!("{key}-{}", r * 3 + c))
                    .grow(1.0)
                    .w(0.0)
                    .gap(space::XS)
                    .px(space::M)
                    .py(space::SM)
                    .radius(radius::BUTTON + 2.0)
                    .fill(PAL.dark_card)
                    .border(1.0, PAL.dark_gold.alpha(0.16))
                    .role("text")
                    .label(format!("{name}: {value}"))
                    .child(
                        text(value.clone())
                            .font(ty::STAT_VALUE)
                            .color(PAL.dark_gold)
                            .wrap()
                            .a11y_hidden(),
                    )
                    .child(
                        text(name.clone())
                            .font(ty::STAT_LABEL)
                            .color(PAL.dark_muted)
                            .wrap()
                            .a11y_hidden(),
                    ),
            );
        }
        for _ in chunk.len()..3 {
            line = line.child(el().grow(1.0).w(0.0));
        }
        grid = grid.child(line);
    }
    grid
}

fn outcome(o: MatchOutcome) -> &'static str {
    match o {
        MatchOutcome::Won => "Won",
        MatchOutcome::Lost => "Lost",
        MatchOutcome::Abandoned => "Left unfinished",
    }
}

/// The outcome chip on a match card: gold for a win, an outline otherwise.
fn chip(o: MatchOutcome) -> El {
    let (fill, ink, edge) = match o {
        MatchOutcome::Won => (PAL.dark_gold, PAL.dark_bg, PAL.dark_gold),
        MatchOutcome::Lost => (Color::TRANSPARENT, PAL.dark_soft, PAL.dark_soft.alpha(0.5)),
        MatchOutcome::Abandoned => (Color::TRANSPARENT, PAL.dark_muted, hairline()),
    };
    row()
        .px(space::SM)
        .py(space::XS - 1.0)
        .radius(radius::PILL)
        .fill(fill)
        .border(1.0, edge)
        .child(text(outcome(o)).font(ty::PILL).color(ink))
}

fn plural(n: i64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn at_the_table(opponents: &[String]) -> Option<El> {
    (!opponents.is_empty()).then(|| small(format!("At the table: {}", opponents.join(" · "))))
}

/* ------------------------------------------------------------------------ */
/* Dates                                                                     */
/* ------------------------------------------------------------------------ */

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn month(t: &LocalTime) -> &'static str {
    MONTHS[(t.month.clamp(1, 12) - 1) as usize]
}

/// Sunday = 0, from the civil date (Howard Hinnant's `days_from_civil`).
fn weekday(t: &LocalTime) -> &'static str {
    let y = i64::from(t.year) - i64::from(t.month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(t.month);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(t.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    WEEKDAYS[(days + 4).rem_euclid(7) as usize]
}

fn clock(t: &LocalTime) -> String {
    let h = match t.hour % 12 {
        0 => 12,
        h => h,
    };
    let half = if t.hour < 12 { "AM" } else { "PM" };
    format!("{h}:{:02} {half}", t.minute)
}

/// `Nov 10, 2026, 2:20 PM` (the games list: month short, day, year, hour, minute).
pub fn short_date(epoch_ms: i64) -> String {
    let t = LocalTime::at(epoch_ms);
    let m = month(&t);
    format!("{} {}, {}, {}", &m[..3], t.day, t.year, clock(&t))
}

/// `Tuesday, November 10, 2026 at 2:20 PM` (a match page: weekday long, date and time).
pub fn long_date(epoch_ms: i64) -> String {
    let t = LocalTime::at(epoch_ms);
    format!(
        "{}, {} {}, {} at {}",
        weekday(&t),
        month(&t),
        t.day,
        t.year,
        clock(&t)
    )
}

/* ------------------------------------------------------------------------ */
/* Your games                                                                */
/* ------------------------------------------------------------------------ */

/// A match's tricks per team. The server records `[0, 0]` today
/// (`docs/port/requests/profile-tricks-won.md`), so when the totals are empty the hand rows'
/// own counts stand in: the tile then agrees with the rows under it.
pub fn match_tricks(replay: &ClientMatchReplay) -> [i64; 2] {
    let [us, them] = replay.stats.tricks;
    if us > 0 || them > 0 {
        return [i64::from(us), i64::from(them)];
    }
    replay.hands.iter().fold([0, 0], |[us, them], hand| {
        [us + hand.tricks_won[0], them + hand.tricks_won[1]]
    })
}

fn stats_grid(s: &ProfileStats) -> El {
    let pct = (s.win_rate * 100.0).round();
    let record = format!("{}–{}", s.won, s.lost);
    let record_label = if s.abandoned > 0 {
        format!("Record (+{} left unfinished)", s.abandoned)
    } else {
        "Record".to_owned()
    };
    tiles(
        "games-stats",
        STATS_LABEL,
        vec![
            (s.matches.to_string(), "Matches played".to_owned()),
            (format!("{pct}%"), "Win rate".to_owned()),
            (record, record_label),
            (
                format!("{}–{}", s.euchres_for, s.euchres_against),
                "Euchres sent–taken".to_owned(),
            ),
            (
                format!("{}/{}", s.loners_made, s.loners_attempted),
                "Loners made".to_owned(),
            ),
            (s.hands_played.to_string(), "Hands played".to_owned()),
        ],
    )
}

fn match_card(m: &euchre_protocol::MatchSummary, i: usize, n: usize) -> El {
    let id = m.match_id.clone();
    let played_at = i64::try_from(m.played_at).unwrap_or(i64::MAX);
    let score = format!("You {} – {} them", m.score[0], m.score[1]);
    let hands = plural(i64::from(m.hands_played), "hand", "hands");
    let date = short_date(played_at);
    let mut c = card(format!("game-{id}"))
        .hover(PAL.dark_card.mix(PAL.dark_gold, 0.08))
        .hover_motion(motion::HOVER, motion::HOVER)
        .press_scale(0.99)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label(format!("{date}, {}, {score}", outcome(m.outcome)))
        .on_press_tx(move |s: &mut AppState, tx| s.navigate(Route::MatchDetail(id.clone()), tx))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(text(date).font(ty::DARK_SMALL).color(PAL.dark_muted))
                .child(chip(m.outcome)),
        )
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(text(score).font(ty::DARK_SCORE).color(PAL.dark_text))
                .child(text(hands).font(ty::DARK_SMALL).color(PAL.dark_soft)),
        );
    if let Some(line) = at_the_table(&m.opponents) {
        c = c.child(line);
    }
    staggered(c, i, n)
}

fn pager(newer: bool, older: Option<String>) -> Option<El> {
    if !newer && older.is_none() {
        return None;
    }
    let mut r = row()
        .key("games-pager")
        .w_full()
        .justify_between()
        .role("group")
        .label(PAGER_LABEL);
    r = r.child(if newer {
        link("games-newer", NEWER, |s: &mut AppState, _| {
            // One page at a time: a press while one loads does nothing.
            let Some(bridge) = s.bridge.clone().filter(|_| !s.games.loading) else {
                return;
            };
            let back = s.games.newer.pop().flatten();
            s.games.shown.clone_from(&back);
            s.games.loading = true;
            bridge.send(Intent::ListGames { cursor: back });
        })
    } else {
        el()
    });
    if let Some(cursor) = older {
        r = r.child(link("games-older", OLDER, move |s: &mut AppState, _| {
            let Some(bridge) = s.bridge.clone().filter(|_| !s.games.loading) else {
                return;
            };
            let here = s.games.shown.replace(cursor.clone());
            s.games.newer.push(here);
            s.games.loading = true;
            bridge.send(Intent::ListGames {
                cursor: Some(cursor.clone()),
            });
        }));
    }
    Some(r)
}

/// The saved-tables band's heading (06 §17.1; "this browser" reads "this device").
pub const SAVED_H2: &str = "Your saved tables";
/// Under [`SAVED_H2`].
pub const SAVED_P: &str = "Resume on this device, exactly where the table is waiting.";
/// When some saved table could not be read just now.
pub const SAVED_UNAVAILABLE: &str = "Some saved tables could not be opened. Their links are kept above; try reconnecting. Completed matches are also listed below.";
/// The band's corner radius and padding (06 §17.1).
const SAVED_RADIUS: f32 = 12.0;
const SAVED_PAD: f32 = 20.0;
/// A saved-table row's height (06 §17.1).
const SAVED_ROW_H: f32 = 48.0;

/// A saved table's link (06 §17.1): resume a live one, open a finished one, or reconnect one
/// whose snapshot was unavailable.
pub fn saved_table_label(table: &RecentTable) -> String {
    let verb = match table.status {
        Some(GameStatus::Active) => "Resume game",
        Some(GameStatus::Complete | GameStatus::Abandoned) => "Open game",
        None => "",
    };
    match (verb, table.hand_no, table.score) {
        ("", _, _) | (_, None, _) | (_, _, None) => {
            let short: String = table.game_id.chars().take(8).collect();
            format!("Reconnect saved table · {short} →")
        }
        (verb, Some(hand), Some([a, b])) => {
            format!("{verb} · Hand {} · {a}–{b} →", hand.saturating_add(1))
        }
    }
}

/// The saved-tables band (06 §17.1, contract §N2.5): the tables this device opened, each a
/// link that opens it; `None` when there are none.
fn saved_tables(recent: &RecentTables) -> Option<El> {
    if recent.tables.is_empty() {
        return None;
    }
    let n = recent.tables.len();
    let mut rows = col()
        .key("saved-tables-list")
        .w_full()
        .role("list")
        .label(SAVED_H2);
    for (i, table) in recent.tables.iter().enumerate() {
        let label = saved_table_label(table);
        let id = table.game_id.clone();
        let row_el = row()
            .key(format!("saved-table-{i}"))
            .w_full()
            .min_h(SAVED_ROW_H)
            .center_y()
            .cursor(Cursor::Pointer)
            .focusable(true)
            .role("link")
            .label(label.clone())
            .on_press_tx(move |s: &mut AppState, tx| s.continue_table(id.clone(), tx))
            .border_sides(
                [if i == 0 { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0],
                PAL.ink.alpha(0.12),
            )
            .child(
                text(label.clone())
                    .font(ty::DARK_BODY)
                    .color(PAL.ink)
                    .spans(vec![
                        Span::new(0, label.len()).underline(PAL.ink.alpha(0.4)),
                    ])
                    .a11y_hidden(),
            );
        rows = rows.child(staggered(row_el, i, n));
    }
    let mut band = col()
        .key("saved-tables")
        .w_full()
        .gap(space::S)
        .padding(SAVED_PAD)
        .radius(SAVED_RADIUS)
        .fill(PAL.cream)
        .role("region")
        .label(SAVED_H2)
        .child(
            text(SAVED_H2)
                .font(ty::DARK_H2)
                .color(PAL.ink)
                .role("heading")
                .level(2),
        )
        .child(
            text(SAVED_P)
                .font(ty::DARK_SMALL)
                .color(PAL.text_secondary)
                .wrap(),
        )
        .child(rows);
    if recent.unavailable {
        band = band.child(
            text(SAVED_UNAVAILABLE)
                .key("saved-tables-unavailable")
                .font(ty::DARK_SMALL)
                .color(PAL.text_secondary)
                .role("status")
                .wrap(),
        );
    }
    Some(band)
}

/// `/games`.
pub fn games(state: &AppState, _cx: &mut Cx<AppState>) -> El {
    let mut items = vec![brand(), h1("games-h1", GAMES_H1)];
    if state.games.error.is_some() {
        items.push(notice("games-error", GAMES_FAILED));
        items.push(link("games-retry", TRY_AGAIN, |s, _| s.reload_route()));
    }
    // The saved tables sit above the list, whatever the list says (contract §N2.5).
    if let Some(band) = state
        .recent_tables
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(saved_tables)
    {
        items.push(band);
    }
    let stats = state.games.stats.clone();
    let first_page = state.games.newer.is_empty();
    let Some(list) = &state.games.page else {
        return page("games", items);
    };
    if list.rows.is_empty() && first_page {
        items.push(
            col()
                .key("games-empty")
                .w_full()
                .gap(space::M)
                .child(h2(EMPTY_H2))
                .child(body(EMPTY_P))
                .child(row().child(dark_button("games-deal", EMPTY_CTA, Route::NewGame))),
        );
        return page("games", items);
    }
    if let Some(s) = stats.filter(|s| s.matches > 0) {
        items.push(stats_grid(&s));
    }
    if list.rows.is_empty() {
        items.push(body(EMPTY_PAGE));
    } else {
        let n = list.rows.len();
        items.push(
            col()
                .key("games-list")
                .w_full()
                .gap(space::SM)
                .role("list")
                .children(
                    list.rows
                        .iter()
                        .enumerate()
                        .map(|(i, m)| match_card(m, i, n)),
                ),
        );
    }
    if let Some(p) = pager(!first_page, list.cursor.clone()) {
        items.push(p);
    }
    page("games", items)
}

/* ------------------------------------------------------------------------ */
/* One match                                                                 */
/* ------------------------------------------------------------------------ */

/// The match page's seat names (06 §17.2 keeps the compass here).
const COMPASS: [&str; 4] = ["You", "West", "North", "East"];

fn seat_name(seat: i64) -> &'static str {
    usize::try_from(seat)
        .ok()
        .and_then(|i| COMPASS.get(i))
        .copied()
        .unwrap_or("Someone")
}

fn suit_line(s: Suit) -> (&'static str, &'static str) {
    match s {
        Suit::Spades => ("♠", "Spades"),
        Suit::Hearts => ("♥", "Hearts"),
        Suit::Diamonds => ("♦", "Diamonds"),
        Suit::Clubs => ("♣", "Clubs"),
    }
}

/// The result line (06 §17.2, verbatim).
pub fn result_line(r: Option<HandResult>) -> &'static str {
    match r {
        Some(HandResult::Point) => "Took the majority — 1 point",
        Some(HandResult::March) => "Swept all five tricks — 2 points",
        Some(HandResult::LonePoint) => "Went alone, took the majority — 1 point",
        Some(HandResult::LoneMarch) => "Went alone, swept all five — 4 points",
        Some(HandResult::Euchre) => "Euchred the makers — 2 points",
        Some(HandResult::ThrowIn) => "Thrown in — no points",
        None => "Unresolved",
    }
}

fn delta_line(delta: Option<[i64; 2]>) -> String {
    match delta {
        Some([us, _]) if us > 0 => format!("(+{us} us)"),
        Some([_, them]) if them > 0 => format!("(+{them} them)"),
        _ => "(no score)".to_owned(),
    }
}

fn hand_row(h: &ClientHandReplay, i: usize, n: usize) -> El {
    let call = match (h.trump, h.maker_seat) {
        (Some(suit), Some(maker)) => {
            let (glyph, name) = suit_line(suit);
            let alone = if h.alone_seat.is_some() {
                " · alone"
            } else {
                ""
            };
            format!("{glyph} {name} called by {}{alone}", seat_name(maker))
        }
        _ => "No trump — thrown in".to_owned(),
    };
    let tricks = format!("{}–{} tricks", h.tricks_won[0], h.tricks_won[1]);
    let ours = h.delta.is_some_and(|[us, _]| us > 0);
    let c = col()
        .key(format!("match-hand-{}", h.hand_no))
        .w_full()
        .gap(space::XS)
        .px(space::M)
        .py(space::SM)
        .radius(radius::BUTTON + 2.0)
        .fill(PAL.dark_card)
        .border_sides(
            [0.0, 0.0, 0.0, 3.0],
            if ours { PAL.dark_gold } else { hairline() },
        )
        .role("listitem")
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    text(format!("Hand {}", h.hand_no + 1))
                        .font(ty::DARK_LABEL)
                        .color(PAL.dark_text),
                )
                .child(
                    text(format!("Dealer: {}", seat_name(h.dealer_seat)))
                        .font(ty::DARK_SMALL)
                        .color(PAL.dark_muted),
                ),
        )
        .child(text(call).font(ty::DARK_INPUT).color(PAL.dark_soft))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .gap(space::M)
                .child(text(tricks).font(ty::DARK_SMALL).color(PAL.dark_muted))
                .child(
                    text(format!("{} {}", result_line(h.result), delta_line(h.delta)))
                        .font(ty::DARK_SMALL)
                        .color(if ours { PAL.dark_gold } else { PAL.dark_soft }),
                ),
        );
    staggered(c, i, n)
}

fn match_heading(o: MatchOutcome) -> &'static str {
    match o {
        MatchOutcome::Won => "You won",
        MatchOutcome::Lost => "They won",
        MatchOutcome::Abandoned => "Left unfinished",
    }
}

/// `/games/[matchId]`.
pub fn match_detail(state: &AppState, match_id: &str, _cx: &mut Cx<AppState>) -> El {
    let back = link("match-back", BACK_TO_GAMES, |s: &mut AppState, tx| {
        s.navigate(Route::Games, tx);
    });
    let mut items = vec![row().child(back)];
    let detail = state
        .match_detail
        .as_ref()
        .filter(|(id, _)| id == match_id)
        .map(|(_, r)| r);
    let replay = match detail {
        Some(Ok(r)) => r,
        Some(Err(_)) => {
            items.push(notice("match-error", MATCH_FAILED));
            return page("match", items);
        }
        None => {
            // Still loading: the list row, when there is one, gives the heading at once.
            let row_for = state
                .games
                .page
                .as_ref()
                .and_then(|p| p.rows.iter().find(|m| m.match_id == match_id));
            if let Some(m) = row_for {
                items.push(h1("match-h1", match_heading(m.outcome)));
                items.push(
                    text(format!("You {} – {} them", m.score[0], m.score[1]))
                        .font(ty::DARK_SCORE)
                        .color(PAL.dark_text),
                );
            }
            return page("match", items);
        }
    };
    let s = &replay.stats;
    items.push(
        col()
            .key("match-header")
            .gap(space::S)
            .child(
                text(long_date(replay.played_at))
                    .key("match-date")
                    .font(ty::DARK_SMALL)
                    .color(PAL.dark_muted),
            )
            .child(h1("match-h1", match_heading(replay.outcome)))
            .child(
                text(format!(
                    "You {} – {} them",
                    replay.score[0], replay.score[1]
                ))
                .key("match-score")
                .font(ty::DARK_SCORE)
                .color(PAL.dark_text),
            )
            .children(at_the_table(&replay.opponents)),
    );
    items.push(tiles(
        "match-totals",
        TOTALS_LABEL,
        vec![
            (replay.hands_played.to_string(), "Hands played".to_owned()),
            (
                format!("{}–{}", s.euchres_for, s.euchres_against),
                "Euchres sent–taken".to_owned(),
            ),
            (
                format!("{}/{}", s.loners_made, s.loners_attempted),
                "Loners made".to_owned(),
            ),
            (s.marches.to_string(), "Marches".to_owned()),
            (s.throw_ins.to_string(), "Thrown in".to_owned()),
            {
                let [us, them] = match_tricks(replay);
                (format!("{us}–{them}"), "Tricks won".to_owned())
            },
        ],
    ));
    let n = replay.hands.len();
    if n > 0 {
        items.push(
            col()
                .key("match-hands")
                .w_full()
                .gap(space::S)
                .role("list")
                .children(
                    replay
                        .hands
                        .iter()
                        .enumerate()
                        .map(|(i, h)| hand_row(h, i, n)),
                ),
        );
    }
    page("match", items)
}

/* ------------------------------------------------------------------------ */
/* Settings                                                                  */
/* ------------------------------------------------------------------------ */

/// A text field in the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    House,
    Name(usize),
    Blurb(usize),
    Prompt(usize),
}

impl Field {
    fn max(self) -> usize {
        match self {
            Field::House => HOUSE_PROMPT_MAX_CHARS,
            Field::Name(_) => PERSONA_NAME_MAX_CHARS,
            Field::Blurb(_) => PERSONA_BLURB_MAX_CHARS,
            Field::Prompt(_) => PERSONA_PROMPT_MAX_CHARS,
        }
    }

    fn key(self) -> String {
        match self {
            Field::House => "set-house".to_owned(),
            Field::Name(i) => format!("set-p{i}-name"),
            Field::Blurb(i) => format!("set-p{i}-blurb"),
            Field::Prompt(i) => format!("set-p{i}-prompt"),
        }
    }

    fn multi_line(self) -> bool {
        matches!(self, Field::House | Field::Prompt(_))
    }
}

/// The form in state. Only fields that are drawn call it, and they are drawn only with data,
/// so the fallback never shows.
fn data_mut(s: &mut AppState) -> &mut SettingsData {
    s.settings.value.get_or_insert_with(|| {
        SettingsData::from_reply(SettingsReply {
            settings: Box::new(default_settings(0)),
            saved: false,
        })
    })
}

/// The form changed: the "Saved." line goes.
fn touch(s: &mut AppState) -> &mut SettingsDraft {
    let d = data_mut(s);
    d.just_saved = false;
    &mut d.draft
}

fn edit_mut(d: &mut SettingsDraft, f: Field) -> &mut TextEdit {
    match f {
        Field::House => &mut d.house_prompt,
        Field::Name(i) => &mut d.personas[i.min(2)].name,
        Field::Blurb(i) => &mut d.personas[i.min(2)].blurb,
        Field::Prompt(i) => &mut d.personas[i.min(2)].prompt,
    }
}

/// Keeps a field inside its limit, as the web's `maxlength` does.
fn clamp_len(e: &mut TextEdit, max: usize) {
    if let Some((cut, _)) = e.text.char_indices().nth(max) {
        let caret = e.focus.min(cut);
        let kept = e.text[..cut].to_owned();
        e.replace_text(kept);
        e.set_caret(caret);
    }
}

fn save(s: &mut AppState) {
    let (sound, reduce_motion) = (s.prefs.sound, s.prefs.reduce_motion);
    let Some(d) = s.settings.value.as_mut() else {
        return;
    };
    d.saving = true;
    d.just_saved = false;
    let patch = d.draft.patch(sound, reduce_motion);
    s.settings.error = None;
    s.patch_settings(patch);
}

fn label(line: &str) -> El {
    text(line.to_owned())
        .font(ty::DARK_LABEL)
        .color(PAL.dark_text)
        .wrap()
}

fn counter(n: usize, max: usize) -> El {
    let near = n as f64 > max as f64 * NEAR;
    text(format!("{n} / {max}"))
        .font(ty::DARK_SMALL)
        .color(if near { PAL.dark_gold } else { PAL.dark_muted })
        .a11y_hidden()
}

/// A labelled text field with its counter.
fn field(d: &SettingsDraft, f: Field, title: &str) -> El {
    let edit = edit_mut(&mut d.clone(), f).clone();
    let n = edit.text.chars().count();
    let max = f.max();
    let mut input = text_input(&edit, ty::DARK_INPUT)
        .key(f.key())
        .w_full()
        .color(PAL.dark_text)
        .caret(PAL.dark_gold, PAL.dark_gold.alpha(0.3))
        .label(title.to_owned())
        .on_edit(edit_access(move |s: &mut AppState| {
            edit_mut(&mut data_mut(s).draft, f)
        }))
        .on_input_change(move |s: &mut AppState, _tx: &mut Tx| {
            clamp_len(edit_mut(touch(s), f), max);
        });
    input = if f.multi_line() {
        input.on_submit(move |s: &mut AppState, _tx: &mut Tx| {
            edit_mut(touch(s), f).insert("\n");
        })
    } else {
        input
            .single_line()
            .on_submit(|s: &mut AppState, _tx: &mut Tx| save(s))
    };
    let boxed = if f.multi_line() {
        rune::auto_grow(input, if f == Field::House { 3 } else { 4 }, 10)
    } else {
        input
    };
    col()
        .w_full()
        .gap(space::XS + 2.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .items_end()
                .gap(space::M)
                .child(label(title).grow(1.0).w(0.0))
                .child(counter(n, max)),
        )
        .child(
            col()
                .key(format!("{}-box", f.key()))
                .w_full()
                .px(space::SM)
                .py(space::S)
                .radius(radius::BUTTON)
                .fill(PAL.dark_bg.alpha(0.55))
                .border(1.0, hairline())
                .cursor(Cursor::Text)
                .child(boxed),
        )
}

/// One choice of a segmented row.
fn segment(key: String, title: &str, on: bool, pick: impl Fn(&mut AppState) + 'static) -> El {
    row()
        .key(key)
        .min_h(space::TAP - 8.0)
        .px(space::M)
        .center()
        .radius(radius::PILL)
        .fill(if on {
            PAL.dark_gold
        } else {
            Color::TRANSPARENT
        })
        .border(1.0, if on { PAL.dark_gold } else { hairline() })
        .hover(if on {
            PAL.dark_gold
        } else {
            PAL.dark_gold.alpha(0.12)
        })
        .press_scale(0.96)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("radio")
        .label(title.to_owned())
        .checked(on)
        .on_press(pick)
        .child(
            text(title.to_owned())
                .font(ty::PILL)
                .color(if on { PAL.dark_bg } else { PAL.dark_soft })
                .a11y_hidden(),
        )
}

fn segmented(key: &str, title: &str, items: Vec<El>) -> El {
    col()
        .w_full()
        .gap(space::XS + 2.0)
        .child(label(title))
        .child(
            row()
                .key(key.to_owned())
                .gap(space::S)
                .flex_wrap()
                .role("radiogroup")
                .label(title.to_owned())
                .children(items),
        )
}

fn check(key: &str, title: &'static str, on: bool, flip: impl Fn(&mut AppState) + 'static) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .min_h(space::TAP)
        .gap(space::M)
        .center_y()
        .px(space::S)
        .radius(radius::BUTTON)
        .hover(PAL.dark_gold.alpha(0.06))
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("checkbox")
        .label(title)
        .checked(on)
        .on_press(flip)
        .child(
            row()
                .size(space::CHECKBOX - 4.0, space::CHECKBOX - 4.0)
                .center()
                .radius(radius::CHECKBOX)
                .border(2.0, PAL.dark_gold)
                .fill(if on { PAL.dark_gold } else { PAL.dark_bg })
                .child(
                    text(if on { "✓" } else { "" })
                        .font(ty::SEAT_DETAIL)
                        .color(PAL.dark_bg)
                        .a11y_hidden(),
                ),
        )
        .child(
            text(title)
                .font(ty::DARK_INPUT)
                .color(PAL.dark_text)
                .wrap()
                .grow(1.0)
                .w(0.0)
                .a11y_hidden(),
        )
}

fn dial_value(raw: f64) -> f64 {
    ((raw / DIAL_STEP).round() * DIAL_STEP).clamp(0.0, 1.0)
}

/// Which persona dial.
#[derive(Clone, Copy, Debug)]
enum Dial {
    Aggression,
    Risk,
    Chattiness,
}

impl Dial {
    const ALL: [Dial; 3] = [Dial::Aggression, Dial::Risk, Dial::Chattiness];

    fn title(self) -> &'static str {
        match self {
            Dial::Aggression => "How eagerly they bid",
            Dial::Risk => "How much they'll gamble on a call",
            Dial::Chattiness => "How much they talk",
        }
    }

    fn slot(self, p: &mut PersonaDraft) -> &mut f64 {
        match self {
            Dial::Aggression => &mut p.aggression,
            Dial::Risk => &mut p.risk,
            Dial::Chattiness => &mut p.chattiness,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Dial::Aggression => "aggression",
            Dial::Risk => "risk",
            Dial::Chattiness => "chattiness",
        }
    }
}

/// A 0..1 slider in 0.05 steps: drag or click the track, or use the arrow keys.
fn dial(seat: usize, which: Dial, value: f64) -> El {
    let pct = (value * 100.0).round();
    let title = format!("{} — {pct}%", which.title());
    let (track_h, knob) = DIAL_TRACK;
    let x = value as f32;
    let set = move |s: &mut AppState, v: f64| {
        let d = touch(s);
        *which.slot(&mut d.personas[seat.min(2)]) = dial_value(v);
    };
    let track = row()
        .key(format!("set-p{seat}-{}", which.name()))
        .w_full()
        .h(knob + 6.0)
        .center_y()
        .cursor(Cursor::Pointer)
        .role("slider")
        .label(which.title())
        .range_value(value, 0.0, 1.0)
        .range_step(DIAL_STEP)
        .a11y_value(format!("{pct}%"))
        .on_drag(move |s: &mut AppState, d: &mut Drag, _tx: &mut Tx| {
            let travel = (d.w - knob).max(1.0);
            let v = f64::from(((d.x - knob / 2.0) / travel).clamp(0.0, 1.0));
            set(s, v);
        })
        .on_step(move |s: &mut AppState, delta: i32, _tx: &mut Tx| {
            let d = touch(s);
            let slot = which.slot(&mut d.personas[seat.min(2)]);
            *slot = dial_value(*slot + f64::from(delta) * DIAL_STEP);
        })
        .on_step_limit(move |s: &mut AppState, max: bool, _tx: &mut Tx| {
            set(s, if max { 1.0 } else { 0.0 });
        })
        .children([
            row()
                .grow(x)
                .w(0.0)
                .h(track_h)
                .radius(radius::PILL)
                .fill(PAL.dark_gold),
            el().key(format!("set-p{seat}-{}-knob", which.name()))
                .size(knob, knob)
                .radius(knob / 2.0)
                .fill(PAL.dark_text)
                .border(2.0, PAL.dark_gold),
            row()
                .grow(1.0 - x)
                .w(0.0)
                .h(track_h)
                .radius(radius::PILL)
                .fill(PAL.dark_muted.alpha(0.3)),
        ]);
    col()
        .w_full()
        .gap(space::XS)
        .child(
            text(title)
                .font(ty::DARK_SMALL)
                .color(PAL.dark_soft)
                .a11y_hidden(),
        )
        .child(track)
}

fn apply_preset(p: &mut PersonaDraft, preset: &PersonaPreset) {
    p.blurb.replace_text(preset.blurb);
    p.prompt.replace_text(preset.prompt);
    p.difficulty = preset.difficulty;
    p.aggression = preset.aggression;
    p.risk = preset.risk;
    p.chattiness = preset.chattiness;
}

fn persona_card(d: &SettingsDraft, seat: usize) -> El {
    let p = &d.personas[seat];
    let diff_index = DIFFICULTIES
        .iter()
        .position(|x| *x == p.difficulty)
        .unwrap_or(1);
    let difficulty = segmented(
        &format!("set-p{seat}-difficulty"),
        DIFFICULTY,
        DIFFICULTIES
            .iter()
            .map(|&choice| {
                segment(
                    format!("set-p{seat}-diff-{}", choice.as_str()),
                    choice.as_str(),
                    choice == p.difficulty,
                    move |s: &mut AppState| touch(s).personas[seat].difficulty = choice,
                )
            })
            .collect(),
    );
    let presets = col().w_full().gap(space::S).child(small(PRESETS)).child(
        row()
            .key(format!("set-p{seat}-presets"))
            .gap(space::S)
            .flex_wrap()
            .children(PERSONA_PRESETS.iter().map(|preset| {
                row()
                    .key(format!("set-p{seat}-preset-{}", preset.id))
                    .min_h(space::TAP - 12.0)
                    .px(space::SM)
                    .center()
                    .radius(radius::PILL)
                    .border(1.0, hairline())
                    .hover(PAL.dark_gold.alpha(0.12))
                    .press_scale(0.96)
                    .cursor(Cursor::Pointer)
                    .focusable(true)
                    .role("button")
                    .label(preset.label)
                    .help(preset.hint)
                    .description(preset.hint)
                    .on_press(move |s: &mut AppState| {
                        apply_preset(&mut touch(s).personas[seat], preset);
                    })
                    .child(
                        text(preset.label)
                            .font(ty::PILL)
                            .color(PAL.dark_soft)
                            .a11y_hidden(),
                    )
            })),
    );
    card(format!("set-persona-{seat}"))
        .gap(space::M)
        .child(h2(SEAT_HEADINGS[seat]))
        .child(field(d, Field::Name(seat), NAME))
        .child(field(d, Field::Blurb(seat), BLURB))
        .children(Dial::ALL.iter().map(|&w| {
            let v = match w {
                Dial::Aggression => p.aggression,
                Dial::Risk => p.risk,
                Dial::Chattiness => p.chattiness,
            };
            dial(seat, w, v)
        }))
        .child(difficulty)
        .child(small(DIFFICULTY_NOTES[diff_index]))
        .child(presets)
        .child(field(d, Field::Prompt(seat), PROMPT))
}

fn disclaimer() -> El {
    let start = DISCLAIMER.find(DISCLAIMER_BOLD).unwrap_or(0);
    let end = start + DISCLAIMER_BOLD.len();
    col()
        .key("set-disclaimer")
        .w_full()
        .px(space::M)
        .py(space::SM + 2.0)
        .radius(radius::BUTTON + 2.0)
        .fill(PAL.dark_gold.alpha(0.07))
        .border_sides([0.0, 0.0, 0.0, 3.0], PAL.dark_gold)
        .child(
            text(DISCLAIMER)
                .font(ty::DARK_SMALL)
                .color(PAL.dark_soft)
                .spans(vec![Span::new(start, end).bold().color(PAL.dark_text)])
                .wrap(),
        )
}

fn table_card(state: &AppState) -> El {
    let p = &state.prefs;
    let mut c = card("settings-table".to_owned())
        .gap(space::M)
        .child(h2(TABLE_H2))
        .child(small(TABLE_HINT));
    if let Some(data) = &state.settings.value {
        let d = &data.draft;
        c = c
            .child(segmented(
                "set-pace",
                PACE,
                [
                    (Pace::Brisk, "Brisk"),
                    (Pace::Normal, "Normal"),
                    (Pace::Relaxed, "Relaxed"),
                ]
                .into_iter()
                .map(|(pace, title)| {
                    segment(
                        format!("set-pace-{}", pace.as_str()),
                        title,
                        d.pace == pace,
                        move |s: &mut AppState| touch(s).pace = pace,
                    )
                })
                .collect(),
            ))
            .child(check("set-banter", BANTER, d.banter, |s| {
                let d = touch(s);
                d.banter = !d.banter;
            }))
            .child(check("set-rationale", RATIONALE, d.show_rationale, |s| {
                let d = touch(s);
                d.show_rationale = !d.show_rationale;
            }));
    }
    // This device's switches work whether or not the profile answered, and apply at once, so
    // they sit in their own group, apart from the fields that wait for Save.
    let device = col()
        .key("settings-device")
        .w_full()
        .gap(space::M)
        .pt(space::M)
        .border_sides([1.0, 0.0, 0.0, 0.0], hairline())
        .role("group")
        .label(DEVICE_H3)
        .child(
            col()
                .gap(space::XS)
                .child(
                    text(DEVICE_H3)
                        .font(ty::DARK_BODY)
                        .color(PAL.dark_text)
                        .role("heading")
                        .level(3),
                )
                .child(small(DEVICE_HINT)),
        )
        .child(check("pref-sound", "Sound", p.sound, |s| s.toggle_sound()))
        .child(check(
            "pref-motion",
            "Reduce motion",
            p.reduce_motion,
            |s| {
                s.update_prefs(|p| p.reduce_motion = !p.reduce_motion);
            },
        ))
        .child(check("pref-hints", "Hints", p.hints, |s| {
            s.update_prefs(|p| p.hints = !p.hints)
        }));
    c.child(device)
}

fn save_row(state: &AppState, data: &SettingsData) -> El {
    let failed = data.saving && state.settings.error.is_some();
    let mut c = col().key("set-save-row").w_full().gap(space::M);
    if failed {
        c = c.child(notice("set-save-failed", SAVE_FAILED));
    } else if data.just_saved {
        c = c.child(
            text(SAVED)
                .key("set-saved")
                .font(ty::DARK_BODY)
                .color(PAL.dark_gold)
                .role("status")
                .live_region(rune::LiveRegion::Polite),
        );
    }
    c.child(
        row().child(
            row()
                .key("set-save")
                .min_h(space::PRIMARY_MIN)
                .px(space::L)
                .center()
                .radius(radius::BUTTON)
                .fill(PAL.dark_button)
                .hover(PAL.dark_button.mix(PAL.dark_text, 0.12))
                .hover_motion(motion::HOVER, motion::HOVER)
                .press_scale(0.96)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .role("button")
                .label(SAVE)
                .on_press(save)
                .child(
                    text(SAVE)
                        .font(ty::LOADING_BUTTON)
                        .color(PAL.dark_button_text)
                        .a11y_hidden(),
                ),
        ),
    )
}

/// `/settings`.
pub fn settings(state: &AppState, _cx: &mut Cx<AppState>) -> El {
    let mut items = vec![brand(), h1("settings-h1", SETTINGS_H1)];
    let data = state.settings.value.as_ref();
    let load_failed = state.settings.error.is_some() && data.is_none_or(|d| !d.saving);
    if load_failed {
        items.push(notice("settings-error", SETTINGS_FAILED));
        items.push(link("settings-retry", TRY_AGAIN, |s, _| s.reload_route()));
    }
    items.push(disclaimer());
    let mut cards = vec![table_card(state)];
    if let Some(data) = data {
        let d = &data.draft;
        cards.push(
            card("set-house-card".to_owned())
                .gap(space::M)
                .child(h2(HOUSE_H2))
                .child(small(HOUSE_HINT))
                .child(field(d, Field::House, HOUSE_H2)),
        );
        for seat in 0..3 {
            cards.push(persona_card(d, seat));
        }
        cards.push(save_row(state, data));
    }
    if crate::web_history::IS_WEB {
        cards.push(classic_card());
    }
    cards.push(about(state.license_open));
    let n = cards.len();
    items.extend(
        cards
            .into_iter()
            .enumerate()
            .map(|(i, c)| staggered(c, i, n)),
    );
    page("settings", items)
}

/// The web build's "Classic table" row: switch this browser to the classic site.
fn classic_card() -> El {
    card("settings-classic".to_owned())
        .child(h2("Classic table"))
        .child(small("Play on the classic site in this browser."))
        .child(link(
            "settings-classic-use",
            "Use the classic table",
            |s, _| s.use_classic_table(),
        ))
}

/// The About card: credits for the art and type, and the card-art license on demand
/// (ARCHITECTURE "Assets").
fn about(open: bool) -> El {
    let label = if open { HIDE_LICENSE } else { SHOW_LICENSE };
    let mut c = card("settings-about".to_owned())
        .child(h2(ABOUT_H2))
        .child(
            text(CREDITS)
                .key("settings-credits")
                .font(ty::SEAT_DETAIL)
                .color(PAL.dark_soft)
                .wrap(),
        )
        .child(
            row()
                .key("settings-license-toggle")
                .min_h(space::TAP)
                .center_y()
                .cursor(Cursor::Pointer)
                .focusable(true)
                .role("button")
                .label(label)
                .expanded(open)
                .on_press(|s: &mut AppState| s.license_open = !s.license_open)
                .child(
                    text(label)
                        .font(ty::SEAT_DETAIL)
                        .color(PAL.dark_gold)
                        .spans(vec![Span::new(0, label.len()).underline(PAL.dark_gold)])
                        .a11y_hidden(),
                ),
        );
    if open {
        c = c.child(
            text(CARD_LICENSE)
                .key("settings-license")
                .font(ty::SEAT_DETAIL)
                .color(PAL.dark_muted)
                .wrap(),
        );
    }
    c
}

/* ------------------------------------------------------------------------ */
/* Fixtures for scenes                                                       */
/* ------------------------------------------------------------------------ */

/// Your games with the sim's seeded history: the first page and the lifetime stats. Pure and
/// deterministic (dates are local time of [`sim::FIXTURE_NOW_MS`]); never touches the network.
pub fn fixture_games(state: &mut AppState) {
    let history = sim::seeded_history("fixture", sim::SIM_HISTORY, sim::FIXTURE_NOW_MS);
    state.route = Route::Games;
    state.games.page = sim::history_page(&history, None).ok();
    state.games.loading = false;
    state.games.error = None;
    state.games.stats = Some(sim::history_stats(&history, sim::FIXTURE_NOW_MS));
    state.games.newer.clear();
    state.games.shown = None;
    state.bridge = Some(Bridge::detached());
}

/// Settings with a new profile's defaults loaded.
pub fn fixture_settings(state: &mut AppState) {
    state.route = Route::Settings;
    state.settings.value = Some(SettingsData::from_reply(SettingsReply {
        settings: Box::new(default_settings(
            i64::try_from(sim::FIXTURE_NOW_MS).unwrap_or(0),
        )),
        saved: false,
    }));
    state.settings.loading = false;
    state.settings.error = None;
}

/// One finished match with its hands: the newest win with a loner or a euchre in it, else the
/// newest finished match.
pub fn fixture_match_detail(state: &mut AppState) {
    let history = sim::seeded_history("fixture", sim::SIM_HISTORY, sim::FIXTURE_NOW_MS);
    let lively = |m: &&sim::SimMatch| {
        m.replay.outcome == MatchOutcome::Won
            && (m.replay.stats.loners_attempted > 0 || m.replay.stats.euchres_for > 0)
    };
    let pick = history
        .iter()
        .find(lively)
        .or_else(|| {
            history
                .iter()
                .find(|m| m.replay.outcome != MatchOutcome::Abandoned)
        })
        .or(history.first())
        .cloned();
    state.games.page = sim::history_page(&history, None).ok();
    if let Some(m) = pick {
        let id = m.summary.match_id.clone();
        state.route = Route::MatchDetail(id.clone());
        state.match_detail = Some((id, Ok(m.replay)));
    } else {
        state.route = Route::Games;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_like_the_web() {
        // 2026-11-10 19:20 UTC, read on a UTC clock.
        let t = LocalTime::at_offset(1_794_338_400_000, 0);
        assert_eq!(weekday(&t), "Tuesday");
        assert_eq!(clock(&t), "7:20 PM");
        assert_eq!(month(&t), "November");
        let midnight = LocalTime::at_offset(0, 0);
        assert_eq!(weekday(&midnight), "Thursday");
        assert_eq!(clock(&midnight), "12:00 AM");
    }

    /// A seeded match with three hands or more.
    fn sim_replay() -> ClientMatchReplay {
        sim::seeded_history("fixture", sim::SIM_HISTORY, sim::FIXTURE_NOW_MS)
            .into_iter()
            .map(|m| m.replay)
            .find(|r| r.hands.len() >= 3)
            .expect("a seeded match with three hands")
    }

    #[test]
    fn tricks_won_falls_back_to_the_hand_rows() {
        let mut replay = sim_replay();
        replay.stats.tricks = [0, 0];
        for (hand, tricks) in replay.hands.iter_mut().zip([[4, 1], [0, 5], [2, 3]]) {
            hand.tricks_won = tricks;
        }
        replay.hands.truncate(3);
        assert_eq!(match_tricks(&replay), [6, 9], "the rows' sum");
        replay.stats.tricks = [7, 8];
        assert_eq!(
            match_tricks(&replay),
            [7, 8],
            "the server's totals when it has them"
        );
    }

    #[test]
    fn lines_follow_06() {
        assert_eq!(delta_line(Some([2, 0])), "(+2 us)");
        assert_eq!(delta_line(Some([0, 4])), "(+4 them)");
        assert_eq!(delta_line(Some([0, 0])), "(no score)");
        assert_eq!(delta_line(None), "(no score)");
        assert_eq!(
            result_line(Some(HandResult::Euchre)),
            "Euchred the makers — 2 points"
        );
        assert_eq!(plural(1, "hand", "hands"), "1 hand");
        assert_eq!(plural(7, "hand", "hands"), "7 hands");
        assert!(DISCLAIMER.contains(DISCLAIMER_BOLD));
    }

    #[test]
    fn the_form_round_trips_into_one_patch() {
        let reply = SettingsReply {
            settings: Box::new(default_settings(5)),
            saved: false,
        };
        let mut data = SettingsData::from_reply(reply);
        data.draft.personas[2].name.replace_text("  Aunt Jo ");
        data.draft.pace = Pace::Relaxed;
        let patch = data.draft.patch(false, true);
        let next =
            euchre_profile_model::apply_settings_patch(&data.saved, &patch.to_value(), 9).unwrap();
        assert_eq!(next.personas[2].name, "Aunt Jo");
        assert_eq!(next.table.pace, Pace::Relaxed);
        assert!(!next.table.sound);
        assert!(next.table.reduce_motion);
        assert_eq!(
            next.personas[0].name, data.saved.personas[0].name,
            "other seats stay"
        );
        assert_eq!(
            next.personas[0].version, data.saved.personas[0].version,
            "unchanged"
        );
    }

    #[test]
    fn a_preset_fills_everything_but_the_name() {
        let mut data = SettingsData::from_reply(SettingsReply {
            settings: Box::new(default_settings(0)),
            saved: false,
        });
        let name = data.draft.personas[0].name.text.clone();
        apply_preset(&mut data.draft.personas[0], &PERSONA_PRESETS[6]);
        let p = &data.draft.personas[0];
        assert_eq!(p.name.text, name);
        assert_eq!(p.blurb.text, PERSONA_PRESETS[6].blurb);
        assert_eq!(p.difficulty, PERSONA_PRESETS[6].difficulty);
        assert_eq!(p.risk, PERSONA_PRESETS[6].risk);
    }

    #[test]
    fn fields_keep_inside_their_limits() {
        let mut e = TextEdit::new("abcdef");
        clamp_len(&mut e, 4);
        assert_eq!(e.text, "abcd");
        let mut wide = TextEdit::new("ééé");
        clamp_len(&mut wide, 2);
        assert_eq!(wide.text, "éé");
        assert_eq!(dial_value(0.437), 0.45);
        assert_eq!(dial_value(1.3), 1.0);
    }
}
