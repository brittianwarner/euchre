//! Settings: defaults, patch validation, repair, and the views built from them
//! (`player-profile/settings.ts`, `server/personas.ts` `personasFromSettings`).
//!
//! Persona prompts are untrusted text: sanitised and clamped on the way in,
//! stored as data, returned verbatim. A caller may never set a persona's `id`
//! or `version`; the version is bumped by the server on every accepted change.

use euchre_protocol::{
    AnthropicModelId, DIAL_MAX, DIAL_MIN, Difficulty, HOUSE_PROMPT_MAX_CHARS, MODEL_BID,
    MODEL_PLAY, PERSONA_BLURB_MAX_CHARS, PERSONA_NAME_MAX_CHARS, PERSONA_PROMPT_MAX_CHARS,
    PersonaAssignment, PersonaConfig, PersonaView, ProtocolErrorCode, SchemaV1, Seat,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ProfileError, ProfileResult};
use crate::guards::{
    SanitizeOptions, clamp_number, js_clamp, require_boolean, require_one_of, sanitize_text,
    sanitize_value,
};
use crate::js::{as_number, is_record, trunc_i64};

/// `AI_SEATS`: the seats a profile's personas bind, index `i` -> seat `i + 1`.
pub const AI_SEATS: [u8; 3] = [1, 2, 3];

/// `MAX_PERSONA_PATCHES`: one patch entry per AI seat.
pub const MAX_PERSONA_PATCHES: usize = AI_SEATS.len();

/// `ANTHROPIC_MODEL_IDS`, in TS order (`[MODEL_PLAY, MODEL_BID]`).
pub const ANTHROPIC_MODEL_IDS: [(AnthropicModelId, &str); 2] = [
    (MODEL_PLAY, MODEL_PLAY.as_str()),
    (MODEL_BID, MODEL_BID.as_str()),
];

/// `DIFFICULTIES` as validator pairs, in TS order.
pub const DIFFICULTY_CHOICES: [(Difficulty, &str); 3] = [
    (Difficulty::Rookie, "rookie"),
    (Difficulty::Casual, "casual"),
    (Difficulty::Expert, "expert"),
];

/// Table pacing (`TablePrefs['pace']`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Pace {
    /// `"brisk"`.
    #[serde(rename = "brisk")]
    Brisk,
    /// `"normal"`.
    #[default]
    #[serde(rename = "normal")]
    Normal,
    /// `"relaxed"`.
    #[serde(rename = "relaxed")]
    Relaxed,
}

impl Pace {
    /// The wire string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Pace::Brisk => "brisk",
            Pace::Normal => "normal",
            Pace::Relaxed => "relaxed",
        }
    }
}

/// `PACES`, in TS order.
pub const PACES: [(Pace, &str); 3] = [
    (Pace::Brisk, "brisk"),
    (Pace::Normal, "normal"),
    (Pace::Relaxed, "relaxed"),
];

/// `TablePrefs`: five presentation flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePrefs {
    /// How quickly the table plays.
    pub pace: Pace,
    /// Whether AI seats talk.
    pub banter: bool,
    /// Whether AI rationale is shown.
    pub show_rationale: bool,
    /// Whether sound plays.
    pub sound: bool,
    /// Whether motion is reduced.
    pub reduce_motion: bool,
}

/// `DEFAULT_TABLE_PREFS`.
pub const DEFAULT_TABLE_PREFS: TablePrefs = TablePrefs {
    pace: Pace::Normal,
    banter: true,
    show_rationale: true,
    sound: true,
    reduce_motion: false,
};

impl Default for TablePrefs {
    fn default() -> TablePrefs {
        DEFAULT_TABLE_PREFS
    }
}

/// `ProfileSettings`: the bounded, `c.state`-resident settings object.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSettings {
    /// Always `1`.
    pub schema: SchemaV1,
    /// Bumped on every accepted patch.
    pub version: i64,
    /// Epoch ms of the last accepted patch.
    pub updated_at: i64,
    /// The shared house prompt, mirrored onto every persona.
    pub house_prompt: String,
    /// Seats 1, 2, 3 in order.
    pub personas: [PersonaConfig; 3],
    /// Presentation flags.
    pub table: TablePrefs,
}

/// The house prompt a new profile starts with (settings `DEFAULT_HOUSE_PROMPT`;
/// not the server-personas text `euchre_protocol::DEFAULT_HOUSE_PROMPT`).
pub const DEFAULT_SETTINGS_HOUSE_PROMPT: &str = "This is a friendly Thursday-night euchre game in somebody\u{2019}s kitchen. Keep the table talk short, dry and good-natured. No coaching, no gloating that lasts.";

/// One seat's seed (`PersonaSeed`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PersonaSeed {
    /// Server-assigned id.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// One-line description.
    pub blurb: &'static str,
    /// Character prompt.
    pub prompt: &'static str,
    /// Playing strength.
    pub difficulty: Difficulty,
    /// Bidding dial.
    pub aggression: f64,
    /// Risk dial.
    pub risk: f64,
    /// Talk dial.
    pub chattiness: f64,
    /// Sampling temperature.
    pub temperature: f64,
}

/// `DEFAULT_PERSONAS`: seats 1 (Marge), 2 (Dutch, the partner), 3 (Verna).
pub const DEFAULT_PERSONAS: [PersonaSeed; 3] = [
    PersonaSeed {
        id: "marge",
        name: "Marge",
        blurb: "Counts every card and lets you know it.",
        prompt: "You have played in this kitchen for thirty years. You are precise, patient, and faintly amused by anybody who bids on three small trump. You speak in short sentences.",
        difficulty: Difficulty::Expert,
        aggression: 0.35,
        risk: 0.25,
        chattiness: 0.4,
        temperature: 0.5,
    },
    PersonaSeed {
        id: "dutch",
        name: "Dutch",
        blurb: "Your partner. Steady, and slightly deaf.",
        prompt: "You are the player\u{2019}s partner. You are steady, encouraging and never second-guess a call out loud. When you are pleased you say so in four words or fewer.",
        difficulty: Difficulty::Casual,
        aggression: 0.5,
        risk: 0.45,
        chattiness: 0.5,
        temperature: 0.6,
    },
    PersonaSeed {
        id: "verna",
        name: "Verna",
        blurb: "Orders it up on anything red.",
        prompt: "You bid too often and enjoy it. You are cheerful about being euchred and insufferable about a march. You never explain your reasoning.",
        difficulty: Difficulty::Casual,
        aggression: 0.8,
        risk: 0.75,
        chattiness: 0.75,
        temperature: 0.7,
    },
];

/// `seedToPersona(seed, housePrompt)`: version 1, the default models.
pub fn seed_to_persona(seed: &PersonaSeed, house_prompt: &str) -> PersonaConfig {
    PersonaConfig {
        id: seed.id.to_owned(),
        version: 1,
        name: seed.name.to_owned(),
        blurb: seed.blurb.to_owned(),
        prompt: seed.prompt.to_owned(),
        house_prompt: house_prompt.to_owned(),
        difficulty: seed.difficulty,
        aggression: seed.aggression,
        risk: seed.risk,
        chattiness: seed.chattiness,
        temperature: seed.temperature,
        model_id: MODEL_PLAY,
        bid_model_id: MODEL_BID,
    }
}

/// `defaultSettings(now)`: a brand-new profile's settings.
pub fn default_settings(now: i64) -> ProfileSettings {
    let [a, b, c] = &DEFAULT_PERSONAS;
    ProfileSettings {
        schema: SchemaV1,
        version: 1,
        updated_at: now,
        house_prompt: DEFAULT_SETTINGS_HOUSE_PROMPT.to_owned(),
        personas: [
            seed_to_persona(a, DEFAULT_SETTINGS_HOUSE_PROMPT),
            seed_to_persona(b, DEFAULT_SETTINGS_HOUSE_PROMPT),
            seed_to_persona(c, DEFAULT_SETTINGS_HOUSE_PROMPT),
        ],
        table: DEFAULT_TABLE_PREFS,
    }
}

fn seat(n: u8) -> Seat {
    Seat::wrapping(n)
}

/// `personaView(persona, seat)`: the client-safe face; `degraded` is always `false`.
pub fn persona_view(persona: &PersonaConfig, seat: Seat) -> PersonaView {
    PersonaView {
        id: persona.id.clone(),
        version: persona.version,
        name: persona.name.clone(),
        blurb: persona.blurb.clone(),
        seat,
        difficulty: persona.difficulty,
        chattiness: persona.chattiness,
        degraded: false,
    }
}

/// The `getPersonas` action body: one view per AI seat, in seat order.
pub fn persona_views(settings: &ProfileSettings) -> Vec<PersonaView> {
    settings
        .personas
        .iter()
        .zip(AI_SEATS)
        .map(|(p, s)| persona_view(p, seat(s)))
        .collect()
}

/// `personasFromSettings(settings)`: seat `i + 1`, the persona with the
/// settings house prompt, an empty dossier.
pub fn personas_from_settings(settings: &ProfileSettings) -> Vec<PersonaAssignment> {
    settings
        .personas
        .iter()
        .zip(AI_SEATS)
        .map(|(p, s)| PersonaAssignment {
            seat: seat(s),
            persona: PersonaConfig {
                house_prompt: settings.house_prompt.clone(),
                ..p.clone()
            },
            dossier: String::new(),
        })
        .collect()
}

/* -------------------------------------------------------------------------- */
/* Patch validation                                                            */
/* -------------------------------------------------------------------------- */

const DIALS: [&str; 4] = ["aggression", "risk", "chattiness", "temperature"];
const MODELS: [&str; 2] = ["modelId", "bidModelId"];
const FLAGS: [&str; 4] = ["banter", "showRationale", "sound", "reduceMotion"];

fn seat_index(raw: Option<&Value>) -> ProfileResult<usize> {
    // `AI_SEATS.findIndex((seat) => seat === rawSeat)`: strict number equality.
    if let Some(v) = as_number(raw)
        && let Some(i) = AI_SEATS.iter().position(|s| f64::from(*s) == v)
    {
        return Ok(i);
    }
    Err(ProfileError::invalid_persona(
        "seat must be one of: 1, 2, 3",
    ))
}

fn apply_persona_patch(
    current: &PersonaConfig,
    raw: Option<&Value>,
    seat: u8,
) -> ProfileResult<PersonaConfig> {
    let Some(Value::Object(patch)) = raw else {
        return Err(ProfileError::invalid_persona(format!(
            "patch for seat {seat} must be an object"
        )));
    };
    let field = |name: &str| format!("personas[{seat}].{name}");
    let mut next = current.clone();
    let mut changed = false;

    if patch.contains_key("name") {
        let f = field("name");
        let name = sanitize_value(
            patch.get("name"),
            SanitizeOptions::new(false, PERSONA_NAME_MAX_CHARS, &f),
        )?;
        if name.is_empty() {
            return Err(ProfileError::invalid_persona(format!(
                "{f} is empty after sanitisation"
            )));
        }
        changed |= next.name != name;
        next.name = name;
    }
    if patch.contains_key("blurb") {
        let f = field("blurb");
        let blurb = sanitize_value(
            patch.get("blurb"),
            SanitizeOptions::new(false, PERSONA_BLURB_MAX_CHARS, &f),
        )?;
        changed |= next.blurb != blurb;
        next.blurb = blurb;
    }
    if patch.contains_key("prompt") {
        let f = field("prompt");
        let prompt = sanitize_value(
            patch.get("prompt"),
            SanitizeOptions::new(true, PERSONA_PROMPT_MAX_CHARS, &f),
        )?;
        changed |= next.prompt != prompt;
        next.prompt = prompt;
    }
    if patch.contains_key("difficulty") {
        let d = require_one_of(
            patch.get("difficulty"),
            &DIFFICULTY_CHOICES,
            &field("difficulty"),
            ProtocolErrorCode::InvalidPersona,
        )?;
        changed |= next.difficulty != d;
        next.difficulty = d;
    }
    for dial in DIALS {
        if patch.contains_key(dial) {
            let v = clamp_number(
                patch.get(dial),
                DIAL_MIN,
                DIAL_MAX,
                &field(dial),
                ProtocolErrorCode::InvalidPersona,
            )?;
            let slot = match dial {
                "aggression" => &mut next.aggression,
                "risk" => &mut next.risk,
                "chattiness" => &mut next.chattiness,
                _ => &mut next.temperature,
            };
            // `!==` on numbers: `0 !== -0` is false, NaN cannot occur.
            changed |= *slot != v;
            *slot = v;
        }
    }
    for model in MODELS {
        if patch.contains_key(model) {
            let m = require_one_of(
                patch.get(model),
                &ANTHROPIC_MODEL_IDS,
                &field(model),
                ProtocolErrorCode::InvalidPersona,
            )?;
            let slot = if model == "modelId" {
                &mut next.model_id
            } else {
                &mut next.bid_model_id
            };
            changed |= *slot != m;
            *slot = m;
        }
    }

    if changed {
        next.version = current.version.saturating_add(1);
    }
    Ok(next)
}

fn apply_table_patch(current: TablePrefs, raw: Option<&Value>) -> ProfileResult<TablePrefs> {
    let Some(Value::Object(patch)) = raw else {
        return Err(ProfileError::invalid_persona("table must be an object"));
    };
    let mut next = current;
    if patch.contains_key("pace") {
        next.pace = require_one_of(
            patch.get("pace"),
            &PACES,
            "table.pace",
            ProtocolErrorCode::InvalidPersona,
        )?;
    }
    for flag in FLAGS {
        if patch.contains_key(flag) {
            let v = require_boolean(patch.get(flag), &format!("table.{flag}"))?;
            match flag {
                "banter" => next.banter = v,
                "showRationale" => next.show_rationale = v,
                "sound" => next.sound = v,
                _ => next.reduce_motion = v,
            }
        }
    }
    Ok(next)
}

/// `applySettingsPatch(current, rawPatch, now)`: validates, clamps and applies a
/// patch, returning **new** settings (`current` is never touched).
///
/// Errors are `invalid_persona` with the TS messages ("settings patch must be
/// an object", "personas must be an array", "at most 3 persona patches per
/// call", "each persona patch must be an object", "seat must be one of: 1, 2,
/// 3", "duplicate seat in persona patches", and the per-field ones).
pub fn apply_settings_patch(
    current: &ProfileSettings,
    raw_patch: &Value,
    now: i64,
) -> ProfileResult<ProfileSettings> {
    let Value::Object(patch) = raw_patch else {
        return Err(ProfileError::invalid_persona(
            "settings patch must be an object",
        ));
    };
    let mut next = current.clone();

    if patch.contains_key("housePrompt") {
        next.house_prompt = sanitize_value(
            patch.get("housePrompt"),
            SanitizeOptions::new(true, HOUSE_PROMPT_MAX_CHARS, "housePrompt"),
        )?;
    }

    if patch.contains_key("table") {
        next.table = apply_table_patch(next.table, patch.get("table"))?;
    }

    if patch.contains_key("personas") {
        let Some(Value::Array(entries)) = patch.get("personas") else {
            return Err(ProfileError::invalid_persona("personas must be an array"));
        };
        if entries.len() > MAX_PERSONA_PATCHES {
            return Err(ProfileError::invalid_persona(format!(
                "at most {MAX_PERSONA_PATCHES} persona patches per call"
            )));
        }
        let mut seen = [false; 3];
        for entry in entries {
            let Value::Object(map) = entry else {
                return Err(ProfileError::invalid_persona(
                    "each persona patch must be an object",
                ));
            };
            let index = seat_index(map.get("seat"))?;
            if seen[index] {
                return Err(ProfileError::invalid_persona(
                    "duplicate seat in persona patches",
                ));
            }
            seen[index] = true;
            next.personas[index] =
                apply_persona_patch(&next.personas[index], map.get("patch"), AI_SEATS[index])?;
        }
    }

    for persona in &mut next.personas {
        if persona.house_prompt != next.house_prompt {
            persona.house_prompt = next.house_prompt.clone();
            persona.version = persona.version.saturating_add(1);
        }
    }

    next.version = current.version.saturating_add(1);
    next.updated_at = now;
    Ok(next)
}

/* -------------------------------------------------------------------------- */
/* Repair                                                                      */
/* -------------------------------------------------------------------------- */

fn safe_text(value: Option<&Value>, max_chars: usize, fallback: &str, multiline: bool) -> String {
    match value {
        Some(Value::String(s)) => {
            sanitize_text(s, SanitizeOptions::new(multiline, max_chars, "stored"))
        }
        _ => fallback.to_owned(),
    }
}

fn safe_id(value: Option<&Value>, fallback: &str) -> String {
    match value {
        Some(Value::String(s))
            if (1..=48).contains(&s.len())
                && s.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                }) =>
        {
            s.clone()
        }
        _ => fallback.to_owned(),
    }
}

fn safe_int(value: Option<&Value>, fallback: i64) -> i64 {
    match as_number(value) {
        Some(v) if v.is_finite() => trunc_i64(v),
        _ => fallback,
    }
}

fn safe_dial(value: Option<&Value>, fallback: f64) -> f64 {
    match as_number(value) {
        Some(v) if v.is_finite() => js_clamp(v, DIAL_MIN, DIAL_MAX),
        _ => fallback,
    }
}

fn safe_enum<T: Copy>(value: Option<&Value>, allowed: &[(T, &str)], fallback: T) -> T {
    match value {
        Some(Value::String(s)) => allowed
            .iter()
            .find(|(_, text)| *text == s)
            .map_or(fallback, |(v, _)| *v),
        _ => fallback,
    }
}

fn safe_bool(value: Option<&Value>, fallback: bool) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        _ => fallback,
    }
}

/// `normalizeSettings(raw, now)`: repairs settings loaded from an older state.
///
/// Total and idempotent: anything missing takes its default, anything out of
/// range is clamped, anything over-long is cut. A stored persona `version`
/// outside `0..=u32::MAX` saturates (TS keeps any finite integer).
pub fn normalize_settings(raw: &Value, now: i64) -> ProfileSettings {
    let base = default_settings(now);
    let Value::Object(map) = raw else {
        return base;
    };

    let house_prompt = safe_text(
        map.get("housePrompt"),
        HOUSE_PROMPT_MAX_CHARS,
        &base.house_prompt,
        true,
    );
    let raw_personas: &[Value] = match map.get("personas") {
        Some(Value::Array(items)) => items,
        _ => &[],
    };

    let personas: [PersonaConfig; 3] = std::array::from_fn(|index| {
        let fallback = &base.personas[index];
        let candidate = raw_personas.get(index);
        if !is_record(candidate) {
            return PersonaConfig {
                house_prompt: house_prompt.clone(),
                ..fallback.clone()
            };
        }
        let get = |k: &str| candidate.and_then(|c| c.get(k));
        PersonaConfig {
            id: safe_id(get("id"), &fallback.id),
            version: saturate_u32(safe_int(get("version"), 1)),
            name: safe_text(get("name"), PERSONA_NAME_MAX_CHARS, &fallback.name, false),
            blurb: safe_text(
                get("blurb"),
                PERSONA_BLURB_MAX_CHARS,
                &fallback.blurb,
                false,
            ),
            prompt: safe_text(
                get("prompt"),
                PERSONA_PROMPT_MAX_CHARS,
                &fallback.prompt,
                true,
            ),
            house_prompt: house_prompt.clone(),
            difficulty: safe_enum(get("difficulty"), &DIFFICULTY_CHOICES, fallback.difficulty),
            aggression: safe_dial(get("aggression"), fallback.aggression),
            risk: safe_dial(get("risk"), fallback.risk),
            chattiness: safe_dial(get("chattiness"), fallback.chattiness),
            temperature: safe_dial(get("temperature"), fallback.temperature),
            model_id: safe_enum(get("modelId"), &ANTHROPIC_MODEL_IDS, fallback.model_id),
            bid_model_id: safe_enum(
                get("bidModelId"),
                &ANTHROPIC_MODEL_IDS,
                fallback.bid_model_id,
            ),
        }
    });

    let table_raw = match map.get("table") {
        Some(t @ Value::Object(_)) => Some(t),
        _ => None,
    };
    let t = |k: &str| table_raw.and_then(|v| v.get(k));
    let table = TablePrefs {
        pace: safe_enum(t("pace"), &PACES, base.table.pace),
        banter: safe_bool(t("banter"), base.table.banter),
        show_rationale: safe_bool(t("showRationale"), base.table.show_rationale),
        sound: safe_bool(t("sound"), base.table.sound),
        reduce_motion: safe_bool(t("reduceMotion"), base.table.reduce_motion),
    };

    ProfileSettings {
        schema: SchemaV1,
        version: safe_int(map.get("version"), 1),
        updated_at: safe_int(map.get("updatedAt"), now),
        house_prompt,
        personas,
        table,
    }
}

fn saturate_u32(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

/* -------------------------------------------------------------------------- */
/* Typed patch builder                                                         */
/* -------------------------------------------------------------------------- */

/// `PersonaPatch`: the editable fields of one persona. Absent fields are left alone.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaPatch {
    /// New display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// New blurb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blurb: Option<String>,
    /// New prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// New difficulty (validated server-side).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<String>,
    /// New aggression dial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggression: Option<f64>,
    /// New risk dial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<f64>,
    /// New chattiness dial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chattiness: Option<f64>,
    /// New temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// New play model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// New bid model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bid_model_id: Option<String>,
}

/// `Partial<TablePrefs>`.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePatch {
    /// New pace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace: Option<Pace>,
    /// New banter flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banter: Option<bool>,
    /// New rationale flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_rationale: Option<bool>,
    /// New sound flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<bool>,
    /// New reduce-motion flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
}

/// One `{ seat, patch }` entry of [`SettingsPatch::personas`].
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PersonaPatchEntry {
    /// The AI seat, 1..=3.
    pub seat: u8,
    /// The fields to change.
    pub patch: PersonaPatch,
}

/// `SettingsPatch`: what `updateSettings` accepts. Build one and send
/// [`SettingsPatch::to_value`]; the server still validates every field.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    /// New house prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub house_prompt: Option<String>,
    /// Table flag changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<TablePatch>,
    /// Per-seat persona changes (at most 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personas: Option<Vec<PersonaPatchEntry>>,
}

impl SettingsPatch {
    /// The patch as the untyped value [`apply_settings_patch`] takes.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
