//! Personas: user-authored, therefore untrusted (protocol/index.ts section 5),
//! plus the built-in opponents (`server/personas.ts`).

use euchre_engine::{Seat, TablePlayer};
use serde::{Deserialize, Serialize};

use crate::serde_util::{js_f64, string_enum};

string_enum! {
    /// A model id a persona may name (`AnthropicModelId`).
    AnthropicModelId {
        /// The fast play model.
        ClaudeHaiku45 => "claude-haiku-4-5",
        /// The strong bid model.
        ClaudeOpus5 => "claude-opus-5",
    }
}

/// Neutral alias of [`AnthropicModelId`]; the AI lane owns model routing.
pub type ModelId = AnthropicModelId;

/// `MODEL_PLAY`: the default card-play model.
pub const MODEL_PLAY: AnthropicModelId = AnthropicModelId::ClaudeHaiku45;
/// `MODEL_BID`: the default bidding model.
pub const MODEL_BID: AnthropicModelId = AnthropicModelId::ClaudeOpus5;
/// `NO_SAMPLING_MODELS`: models that reject sampling parameters such as temperature.
pub const NO_SAMPLING_MODELS: [AnthropicModelId; 1] = [AnthropicModelId::ClaudeOpus5];

string_enum! {
    /// A persona's playing strength (`Difficulty`).
    Difficulty {
        /// Weakest.
        Rookie => "rookie",
        /// Middle.
        Casual => "casual",
        /// Strongest.
        Expert => "expert",
    }
}

/// `DIFFICULTIES`, in TS order.
pub const DIFFICULTIES: [Difficulty; 3] =
    [Difficulty::Rookie, Difficulty::Casual, Difficulty::Expert];

/// Max persona name length (chars).
pub const PERSONA_NAME_MAX_CHARS: usize = 32;
/// Max persona blurb length (chars).
pub const PERSONA_BLURB_MAX_CHARS: usize = 120;
/// Max persona prompt length (chars).
pub const PERSONA_PROMPT_MAX_CHARS: usize = 2000;
/// Max house prompt length (chars).
pub const HOUSE_PROMPT_MAX_CHARS: usize = 1000;
/// Max dossier length (chars).
pub const PERSONA_DOSSIER_MAX_CHARS: usize = 1600;
/// Length of the random fence nonce around untrusted text.
pub const PERSONA_FENCE_NONCE_CHARS: usize = 12;
/// Max banter line length (chars).
pub const BANTER_MAX_CHARS: usize = 140;
/// Max rationale length (chars).
pub const RATIONALE_MAX_CHARS: usize = 120;
/// Max episode summary length (chars).
pub const EPISODE_SUMMARY_MAX_CHARS: usize = 140;
/// Lowest persona dial value.
pub const DIAL_MIN: f64 = 0.0;
/// Highest persona dial value.
pub const DIAL_MAX: f64 = 1.0;
/// Lowest sampling temperature.
pub const LLM_TEMPERATURE_MIN: f64 = 0.2;
/// Highest sampling temperature.
pub const LLM_TEMPERATURE_MAX: f64 = 0.8;

/// One opponent's configuration (`PersonaConfig`). Text fields are untrusted.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaConfig {
    /// Stable id.
    pub id: String,
    /// Bumped on every edit.
    pub version: u32,
    /// Display name.
    pub name: String,
    /// One-line description.
    pub blurb: String,
    /// Persona prompt (untrusted).
    pub prompt: String,
    /// House prompt (untrusted).
    pub house_prompt: String,
    /// Playing strength.
    pub difficulty: Difficulty,
    /// Bidding aggression, 0..=1.
    #[serde(with = "js_f64")]
    pub aggression: f64,
    /// Risk appetite, 0..=1.
    #[serde(with = "js_f64")]
    pub risk: f64,
    /// How often it talks, 0..=1.
    #[serde(with = "js_f64")]
    pub chattiness: f64,
    /// Sampling temperature.
    #[serde(with = "js_f64")]
    pub temperature: f64,
    /// Card-play model.
    pub model_id: AnthropicModelId,
    /// Bidding model.
    pub bid_model_id: AnthropicModelId,
}

/// A persona in a chair (`PersonaAssignment`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaAssignment {
    /// Seat 1, 2 or 3.
    pub seat: Seat,
    /// The persona.
    pub persona: PersonaConfig,
    /// Remembered notes about the human (untrusted).
    pub dossier: String,
}

/// The client-safe persona summary (`PersonaView`, `aiSeat.getPersona()`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaView {
    /// Stable id.
    pub id: String,
    /// Version.
    pub version: u32,
    /// Display name.
    pub name: String,
    /// One-line description.
    pub blurb: String,
    /// The chair.
    pub seat: Seat,
    /// Strength.
    pub difficulty: Difficulty,
    /// Talkativeness.
    #[serde(with = "js_f64")]
    pub chattiness: f64,
    /// Whether the seat is running on its heuristic fallback.
    pub degraded: bool,
}

/// The shared house prompt of the built-in opponents.
pub const DEFAULT_HOUSE_PROMPT: &str = "A friendly kitchen-table euchre game. Warm, dry humour; one short line when something actually happens. Never coach or signal to a partner.";

/// The built-in opponent fields that differ per seat.
struct DefaultPlayer {
    id: &'static str,
    name: &'static str,
    blurb: &'static str,
    prompt: &'static str,
    aggression: f64,
    risk: f64,
    chattiness: f64,
}

const DEFAULT_PLAYERS: [DefaultPlayer; 3] = [
    DefaultPlayer {
        id: "west-rita",
        name: "Rita",
        blurb: "Patient. Remembers every trump.",
        prompt: "You have played euchre for forty years. Count public cards carefully, favour a dependable point over a speculative loner, and preserve partner winners. Your humour is understated: a raised eyebrow in a few words. Never claim to know hidden cards.",
        aggression: 0.3,
        risk: 0.25,
        chattiness: 0.35,
    },
    DefaultPlayer {
        id: "north-ned",
        name: "Ned",
        blurb: "Your steady, encouraging partner.",
        prompt: "You are a dependable experienced partner. Make sound calls, protect vulnerable partner winners, and save strength when their trick is secure. You celebrate good team play in a few warm words, never coach, scold, or give hints about your cards.",
        aggression: 0.5,
        risk: 0.45,
        chattiness: 0.45,
    },
    DefaultPlayer {
        id: "east-grandpa",
        name: "Grandpa",
        blurb: "Takes a chance. Enjoys the story.",
        prompt: "You are a seasoned kitchen-table player with a mischievous streak. Take a plausible close call, value a well-supported loner and play for the march when it is still available. You take calculated chances, never deliberately misplay or spend a bower when a small trump does the same job. Your short, good-natured quips sound like an old friend; no caricatures, coaching or hidden-card hints.",
        aggression: 0.8,
        risk: 0.8,
        chattiness: 0.65,
    },
];

/// The built-in opponents Rita (seat 1), Ned (seat 2) and Grandpa (seat 3) (`defaultPersonas`).
pub fn default_personas() -> Vec<PersonaAssignment> {
    DEFAULT_PLAYERS
        .iter()
        .zip(1u8..)
        .map(|(p, seat)| PersonaAssignment {
            seat: Seat::wrapping(seat),
            persona: PersonaConfig {
                id: p.id.to_owned(),
                version: 1,
                name: p.name.to_owned(),
                blurb: p.blurb.to_owned(),
                prompt: p.prompt.to_owned(),
                house_prompt: DEFAULT_HOUSE_PROMPT.to_owned(),
                difficulty: Difficulty::Expert,
                aggression: p.aggression,
                risk: p.risk,
                chattiness: p.chattiness,
                temperature: 0.6,
                model_id: AnthropicModelId::ClaudeHaiku45,
                bid_model_id: AnthropicModelId::ClaudeHaiku45,
            },
            dossier: String::new(),
        })
        .collect()
}

/// The style label the table shows for a persona's risk dial.
pub fn persona_style(risk: f64) -> &'static str {
    if risk >= 0.7 {
        "Bold bidder"
    } else if risk <= 0.3 {
        "Card counter"
    } else {
        "Steady player"
    }
}

/// `view.players` as the table builds it from its persona snapshot.
pub fn table_players(personas: &[PersonaAssignment]) -> Vec<TablePlayer> {
    personas
        .iter()
        .map(|a| TablePlayer {
            seat: a.seat,
            name: a.persona.name.clone(),
            blurb: a.persona.blurb.clone(),
            style: persona_style(a.persona.risk).to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_thresholds_are_inclusive() {
        assert_eq!(persona_style(0.7), "Bold bidder");
        assert_eq!(persona_style(0.69), "Steady player");
        assert_eq!(persona_style(0.3), "Card counter");
        assert_eq!(persona_style(0.31), "Steady player");
    }

    #[test]
    fn default_personas_sit_in_seats_one_to_three() {
        let seats: Vec<u8> = default_personas().iter().map(|a| a.seat.get()).collect();
        assert_eq!(seats, [1, 2, 3]);
    }
}
