//! Bounds on the table's create input (W3 follow-up, `reports/w3-crates.md` §3).
//!
//! The table snapshots its match id, ids, seed and personas at create and
//! copies all of them into the match's one `recordGame` body. TS trusted its
//! creator to keep them in bounds: 03 §2 says "3 entries, seats 1..3", and TS
//! `TableState` says the persona text is "capped by `$lib/protocol`". Nothing
//! checked it. An oversized input made every `recordGame` for the match fail
//! the same way (over rivetkit's 65,536-byte incoming-message limit, or the
//! profile's own record checks), and `flushProfile` retried it forever.
//!
//! The port enforces those bounds here, at create:
//!
//! - **Personas are clamped**, as TS `sanitizeText` clamps. The first entry
//!   for each AI seat (1..=3) is kept, in input order; a seat-0 entry or a
//!   second entry for a seat is dropped ([`TableState::persona_for`] already
//!   reads the first). Every text field is re-run through the profile's
//!   `sanitize_text` at its protocol cap. Those caps count code points, as TS
//!   `Array.from` does, so the creator's output (`personas_from_settings`) is a
//!   fixed point. The persona id is cut to 128 UTF-16 units (D27, the
//!   profile's id cap).
//! - **The seed is cut** to the profile's 256-unit record cap (D27). An empty
//!   seed would fail the profile's check: `create` uses a random seed for it,
//!   as for an absent one, and [`bounded_seed`] refuses it.
//! - **Ids are refused, not cut**, since the table cannot shorten an identity.
//!   The match id must match the profile's id pattern (the profile refuses any
//!   other `record.matchId`). An owner or profile id must be at most 128 UTF-16
//!   units. An empty profile id means "file under the owner", as an absent one
//!   does.
//!
//! With these bounds the largest `recordGame` body is 63,617 B (the
//! regression test `worst_case_record_game_fits_the_action_limit` in
//! `core/tests.rs` builds it and measures it).
//!
//! [`TableState::persona_for`]: crate::state::TableState::persona_for

use anyhow::{Result, bail};
use euchre_engine::HUMAN_SEAT;
use euchre_profile_model::guards::is_valid_id;
use euchre_profile_model::js::utf16_len;
use euchre_profile_model::{MAX_SEED_UNITS, SanitizeOptions, sanitize_text};
use euchre_protocol::{
    HOUSE_PROMPT_MAX_CHARS, PERSONA_BLURB_MAX_CHARS, PERSONA_DOSSIER_MAX_CHARS,
    PERSONA_NAME_MAX_CHARS, PERSONA_PROMPT_MAX_CHARS, PersonaAssignment, PersonaConfig,
    TableCreateInput,
};

use crate::text::utf16_prefix;

/// The most personas a table keeps: one per AI seat.
pub const MAX_PERSONAS: usize = 3;
/// Longest kept persona id, in UTF-16 units (the profile's id cap).
pub const PERSONA_ID_MAX_UNITS: usize = 128;
/// Longest accepted owner or profile id, in UTF-16 units.
pub const USER_ID_MAX_UNITS: usize = 128;
/// Longest kept seed, in UTF-16 units (the profile's `MAX_SEED_UNITS`).
pub const SEED_MAX_UNITS: usize = MAX_SEED_UNITS;

/// The create input's ids, checked: `(ownerUserId, profileUserId)`.
///
/// The owner is `""` until claimed. An empty profile id is `None`.
pub fn checked_ids(input: &TableCreateInput) -> Result<(String, Option<String>)> {
    let owner = input.owner_user_id.clone().unwrap_or_default();
    check_user_id("ownerUserId", &owner)?;
    let profile = input.profile_user_id.clone().filter(|p| !p.is_empty());
    if let Some(p) = &profile {
        check_user_id("profileUserId", p)?;
    }
    Ok((owner, profile))
}

fn check_user_id(field: &str, value: &str) -> Result<()> {
    let units = utf16_len(value);
    if units > USER_ID_MAX_UNITS {
        bail!(
            "euchreTable create input: {field} is {units} UTF-16 units, over {USER_ID_MAX_UNITS}"
        );
    }
    Ok(())
}

/// The match id, checked against the profile's id pattern.
pub fn check_match_id(game_id: &str) -> Result<()> {
    if !is_valid_id(game_id) {
        bail!("euchreTable key: the match id must match /^[A-Za-z0-9_-]{{1,128}}$/");
    }
    Ok(())
}

/// The seed, cut to [`SEED_MAX_UNITS`]. An empty seed is refused.
pub fn bounded_seed(seed: &str) -> Result<String> {
    if seed.is_empty() {
        bail!("euchreTable create input: seed must not be empty");
    }
    Ok(utf16_prefix(seed, SEED_MAX_UNITS))
}

/// The personas the table keeps (module docs): at most one per AI seat, every
/// text field at its cap.
pub fn bounded_personas(raw: &[PersonaAssignment]) -> Vec<PersonaAssignment> {
    let mut kept: Vec<PersonaAssignment> = Vec::with_capacity(MAX_PERSONAS);
    for assignment in raw {
        if assignment.seat == HUMAN_SEAT || kept.iter().any(|k| k.seat == assignment.seat) {
            continue;
        }
        kept.push(bounded_persona(assignment));
    }
    kept
}

fn text(value: &str, multiline: bool, max_chars: usize, field: &str) -> String {
    sanitize_text(value, SanitizeOptions::new(multiline, max_chars, field))
}

fn bounded_persona(assignment: &PersonaAssignment) -> PersonaAssignment {
    let p = &assignment.persona;
    PersonaAssignment {
        seat: assignment.seat,
        persona: PersonaConfig {
            id: utf16_prefix(&p.id, PERSONA_ID_MAX_UNITS),
            version: p.version,
            name: text(&p.name, false, PERSONA_NAME_MAX_CHARS, "persona.name"),
            blurb: text(&p.blurb, false, PERSONA_BLURB_MAX_CHARS, "persona.blurb"),
            prompt: text(&p.prompt, true, PERSONA_PROMPT_MAX_CHARS, "persona.prompt"),
            house_prompt: text(
                &p.house_prompt,
                true,
                HOUSE_PROMPT_MAX_CHARS,
                "persona.housePrompt",
            ),
            difficulty: p.difficulty,
            aggression: p.aggression,
            risk: p.risk,
            chattiness: p.chattiness,
            temperature: p.temperature,
            model_id: p.model_id,
            bid_model_id: p.bid_model_id,
        },
        dossier: text(
            &assignment.dossier,
            true,
            PERSONA_DOSSIER_MAX_CHARS,
            "dossier",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_engine::Seat;
    use euchre_profile_model::{apply_settings_patch, default_settings, personas_from_settings};
    use euchre_protocol::default_personas;
    use serde_json::json;

    fn assignment(seat: Seat, name: &str) -> PersonaAssignment {
        let mut a = default_personas()
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("a default persona"));
        a.seat = seat;
        a.persona.name = name.to_owned();
        a
    }

    #[test]
    fn the_creators_personas_are_a_fixed_point() {
        let defaults = default_personas();
        assert_eq!(bounded_personas(&defaults), defaults);

        let settings = default_settings(1);
        let from_settings = personas_from_settings(&settings);
        assert_eq!(bounded_personas(&from_settings), from_settings);

        // A settings patch at every cap, in 4-byte characters.
        let emoji = |n: usize| "\u{1F0CF}".repeat(n);
        let persona_patches: Vec<serde_json::Value> = [1, 2, 3]
            .into_iter()
            .map(|seat| {
                json!({
                    "seat": seat,
                    "patch": {
                        "name": emoji(PERSONA_NAME_MAX_CHARS * 2),
                        "blurb": emoji(PERSONA_BLURB_MAX_CHARS * 2),
                        "prompt": format!("{}\n\n\n{}", emoji(10), emoji(PERSONA_PROMPT_MAX_CHARS * 2)),
                    }
                })
            })
            .collect();
        let patch = json!({
            "housePrompt": emoji(HOUSE_PROMPT_MAX_CHARS * 2),
            "personas": persona_patches,
        });
        let maxed =
            apply_settings_patch(&settings, &patch, 1).unwrap_or_else(|e| panic!("patch: {e:?}"));
        let personas = personas_from_settings(&maxed);
        assert_eq!(
            personas[0].persona.name.chars().count(),
            PERSONA_NAME_MAX_CHARS
        );
        assert_eq!(bounded_personas(&personas), personas);
    }

    #[test]
    fn personas_keep_the_first_entry_per_ai_seat() {
        let raw = vec![
            assignment(Seat::SOUTH, "human"),
            assignment(Seat::NORTH, "two"),
            assignment(Seat::WEST, "one"),
            assignment(Seat::NORTH, "two again"),
            assignment(Seat::EAST, "three"),
            assignment(Seat::WEST, "one again"),
        ];
        let kept: Vec<(u8, String)> = bounded_personas(&raw)
            .into_iter()
            .map(|a| (a.seat.get(), a.persona.name))
            .collect();
        assert_eq!(
            kept,
            [(2, "two".into()), (1, "one".into()), (3, "three".into())]
        );
    }

    #[test]
    fn persona_text_is_clamped_to_the_protocol_caps() {
        let mut a = assignment(Seat::WEST, &"n\u{0}".repeat(100));
        a.persona.id = "\u{1F0CF}".repeat(200);
        a.persona.blurb = "b ".repeat(500);
        a.persona.prompt = "\u{1F0CF}".repeat(PERSONA_PROMPT_MAX_CHARS + 7);
        a.persona.house_prompt = "h\n".repeat(HOUSE_PROMPT_MAX_CHARS);
        a.dossier = "\u{1F0CF}".repeat(PERSONA_DOSSIER_MAX_CHARS * 3);
        let [b] = <[PersonaAssignment; 1]>::try_from(bounded_personas(&[a]))
            .unwrap_or_else(|_| panic!("one persona"));
        let p = &b.persona;
        assert_eq!(
            p.name,
            "n".repeat(PERSONA_NAME_MAX_CHARS),
            "controls stripped"
        );
        assert_eq!(
            utf16_len(&p.id),
            PERSONA_ID_MAX_UNITS,
            "64 pairs, none split"
        );
        assert!(p.blurb.chars().count() <= PERSONA_BLURB_MAX_CHARS);
        assert!(!p.blurb.ends_with(' '));
        assert_eq!(p.prompt.chars().count(), PERSONA_PROMPT_MAX_CHARS);
        assert!(p.house_prompt.chars().count() <= HOUSE_PROMPT_MAX_CHARS);
        assert_eq!(b.dossier.chars().count(), PERSONA_DOSSIER_MAX_CHARS);
    }

    #[test]
    fn the_seed_is_cut_on_a_utf16_boundary_and_an_empty_one_is_refused() {
        let ok = bounded_seed("seed").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ok, "seed");
        let long = format!("x{}", "\u{1F0CF}".repeat(200));
        let cut = bounded_seed(&long).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            utf16_len(&cut),
            SEED_MAX_UNITS - 1,
            "the straddling pair is dropped"
        );
        assert!(bounded_seed("").is_err());
    }

    #[test]
    fn ids_are_refused_over_the_cap_and_an_empty_profile_id_is_the_owner() {
        let input = |owner: &str, profile: Option<&str>| TableCreateInput {
            owner_user_id: Some(owner.to_owned()),
            profile_user_id: profile.map(str::to_owned),
            ..TableCreateInput::default()
        };
        let at_cap = "u".repeat(USER_ID_MAX_UNITS);
        let over = "u".repeat(USER_ID_MAX_UNITS + 1);
        assert!(checked_ids(&input(&at_cap, Some(&at_cap))).is_ok());
        assert!(checked_ids(&input(&over, None)).is_err());
        assert!(checked_ids(&input("guest-a", Some(&over))).is_err());
        let (owner, profile) =
            checked_ids(&input("guest-a", Some(""))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((owner.as_str(), profile), ("guest-a", None));
        let (owner, _) =
            checked_ids(&TableCreateInput::default()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(owner, "", "unclaimed");

        assert!(check_match_id("2f0c1a7e-0d7b-4f43-9a54-6d1f7c2f9e11").is_ok());
        assert!(check_match_id(&"m".repeat(128)).is_ok());
        assert!(check_match_id(&"m".repeat(129)).is_err());
        assert!(check_match_id("a.b").is_err());
        assert!(check_match_id("").is_err());
    }
}
