//! One-tap persona starting points for the Settings editor
//! (`routes/settings/presets.ts`, 05 §3.5.5).
//!
//! Client-facing convenience only: a preset fills a seat's blurb, prompt and
//! dials in the form (never the name) and nothing is saved until the user
//! saves. No actor reads these.

use euchre_protocol::Difficulty;
use serde::Serialize;

/// `PersonaPreset`.
#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
pub struct PersonaPreset {
    /// Stable id.
    pub id: &'static str,
    /// Shown on the preset's chip.
    pub label: &'static str,
    /// One line under the chip.
    pub hint: &'static str,
    /// Blurb to fill in.
    pub blurb: &'static str,
    /// Prompt to fill in.
    pub prompt: &'static str,
    /// Difficulty to fill in.
    pub difficulty: Difficulty,
    /// Aggression dial.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub aggression: f64,
    /// Risk dial.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub risk: f64,
    /// Chattiness dial.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub chattiness: f64,
}

/// `PERSONA_PRESETS`, in TS order.
pub const PERSONA_PRESETS: [PersonaPreset; 7] = [
    PersonaPreset {
        id: "grandpa",
        label: "Grandpa",
        hint: "Experienced, warm, takes a calculated chance.",
        blurb: "Takes a chance. Enjoys the story.",
        prompt: "You are a seasoned kitchen-table player with a mischievous streak. Take plausible close calls and well-supported loners, then play sound partnership euchre. Never deliberately misplay, waste secured winners, coach, or hint at your cards. Your humour is warm and understated, like an old friend.",
        difficulty: Difficulty::Expert,
        aggression: 0.8,
        risk: 0.8,
        chattiness: 0.65,
    },
    PersonaPreset {
        id: "counter",
        label: "The Counter",
        hint: "Precise, patient, plays it by the book.",
        blurb: "Counts every card and rarely misplays one.",
        prompt: "You are careful and exact. You track what has been played and almost never make a mistake. You speak rarely, and when you do it is short and dry.",
        difficulty: Difficulty::Expert,
        aggression: 0.3,
        risk: 0.25,
        chattiness: 0.25,
    },
    PersonaPreset {
        id: "steady",
        label: "Steady Partner",
        hint: "Encouraging, dependable, never second-guesses out loud.",
        blurb: "Solid and supportive \u{2014} never rattled.",
        prompt: "You are calm and encouraging. You play sound, sensible euchre and never criticize a partner\u{2019}s call out loud. When something goes well you say so briefly.",
        difficulty: Difficulty::Casual,
        aggression: 0.45,
        risk: 0.4,
        chattiness: 0.45,
    },
    PersonaPreset {
        id: "bold",
        label: "Bold Bidder",
        hint: "Orders it up often, plays for the march.",
        blurb: "Bids first, asks questions never.",
        prompt: "You bid aggressively and love going for a march. You are cheerful even when a bid goes wrong, and a little smug when it doesn\u{2019}t.",
        difficulty: Difficulty::Casual,
        aggression: 0.85,
        risk: 0.8,
        chattiness: 0.6,
    },
    PersonaPreset {
        id: "chatty",
        label: "Chatterbox",
        hint: "Warm, talkative, keeps the table lively.",
        blurb: "Has something to say about every hand.",
        prompt: "You are warm and talkative, with a running commentary on the game. You are good-natured about losing and generous in victory.",
        difficulty: Difficulty::Casual,
        aggression: 0.55,
        risk: 0.5,
        chattiness: 0.85,
    },
    PersonaPreset {
        id: "quiet-pro",
        label: "Quiet Pro",
        hint: "Expert-level play, almost never talks.",
        blurb: "Says little. Plays like it matters.",
        prompt: "You are focused and nearly silent. You play at a high level and let the cards do the talking. On the rare occasion you speak, it is one short sentence.",
        difficulty: Difficulty::Expert,
        aggression: 0.5,
        risk: 0.45,
        chattiness: 0.1,
    },
    PersonaPreset {
        id: "wildcard",
        label: "Wildcard",
        hint: "Unpredictable and a little reckless.",
        blurb: "You never quite know what this one will do.",
        prompt: "You are unpredictable and take chances other players wouldn\u{2019}t. You are delighted by a wild outcome, win or lose.",
        difficulty: Difficulty::Rookie,
        aggression: 0.7,
        risk: 0.85,
        chattiness: 0.65,
    },
];

/// The preset with `id`, if any.
pub fn persona_preset(id: &str) -> Option<&'static PersonaPreset> {
    PERSONA_PRESETS.iter().find(|p| p.id == id)
}
