//! `docs/port/requests/ai-profile-bodies.md`, seat side: the `recordEpisodes`
//! and `recordTokens` bodies this seat sends are accepted, field for field, by
//! the profile's own parsers (`euchre_profile_model`), with no shared Rust type
//! (D29: the JSON is the contract).

use anyhow::{Context as _, Result, bail, ensure};
use euchre_ai::ai_seat::handlers::{EpisodeFlush, on_game_end, on_hand_end};
use euchre_ai::ai_seat::state::{
    AiSeatCreateInput, AiSeatState, CreateStateParams, HandSnapshot, create_state,
};
use euchre_engine::{HandResult, Seat, Suit};
use euchre_profile_model::{TokenIncrement, body_internal_token, parse_record_episodes};
use euchre_protocol::{AiSeatLifecycleKind, AiSeatLifecycleMessage, EpisodeKind, default_personas};
use serde_json::{Value, json};

const TABLE_TOKEN: &str = "table-token-0123456789abcdef";
const PROFILE_TOKEN: &str = "profile-internal-0123456789abcdef";
const NOW: u64 = 1_761_955_200_000;

fn seat(n: u8) -> Result<Seat> {
    Seat::new(n).context("seat")
}

fn state() -> Result<AiSeatState> {
    let input = AiSeatCreateInput {
        game_id: "g1".to_owned(),
        seat: seat(1)?,
        internal_token: TABLE_TOKEN.to_owned(),
        fence_nonce: "nonce".to_owned(),
        persona: default_personas().remove(0).persona,
        dossier: None,
        profile_key: Some(vec!["user".to_owned(), "u1".to_owned()]),
        match_id: Some("m1".to_owned()),
    };
    let key: Vec<String> = ["table", "g1", "seat", "1"].map(str::to_owned).to_vec();
    Ok(create_state(CreateStateParams {
        key: &key,
        input: Some(&input),
    })?)
}

fn lifecycle(kind: AiSeatLifecycleKind, hand_no: Option<u32>) -> AiSeatLifecycleMessage {
    AiSeatLifecycleMessage {
        internal_token: TABLE_TOKEN.to_owned(),
        game_id: "g1".to_owned(),
        kind,
        hand_no,
    }
}

#[test]
fn record_episodes_body_passes_the_profile_parser() -> Result<()> {
    let mut s = state()?;
    // Seat 1's team (EW) called hearts alone and marched it: salience 0.9.
    s.memory.snapshot = Some(HandSnapshot {
        hand_no: 3,
        trump: Some(Suit::Hearts),
        maker_seat: Some(seat(1)?),
        alone_seat: Some(seat(1)?),
        tricks_won: [0, 5],
        result: Some(HandResult::LoneMarch),
    });
    let flush = on_hand_end(
        &mut s,
        &lifecycle(AiSeatLifecycleKind::HandEnd, Some(3)),
        NOW,
        Some(PROFILE_TOKEN),
    )
    .context("hand end dropped")?;
    let EpisodeFlush::Send { profile_key, body } = flush else {
        bail!("expected a send, got {flush:?}");
    };
    ensure!(profile_key == ["user", "u1"]);

    // The exact JSON the seat sends (`send` serializes the body with serde_json).
    let wire = serde_json::to_value(&body)?;
    let episode = &wire["episodes"][0];
    ensure!(
        wire.as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            == Some(vec!["internalToken".to_owned(), "episodes".to_owned()]),
        "top-level keys: {wire}"
    );
    ensure!(
        episode
            .as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            == Some(
                [
                    "role", "matchId", "handNo", "ts", "kind", "summary", "salience"
                ]
                .map(str::to_owned)
                .to_vec()
            ),
        "episode keys: {episode}"
    );
    ensure!(body_internal_token(&wire) == Some(PROFILE_TOKEN));

    let now = i64::try_from(NOW)?;
    let stored = parse_record_episodes(&wire, now).map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(stored.len() == 1, "stored {stored:?}");
    let e = &stored[0];
    ensure!(e.role == s.persona.id);
    ensure!(e.match_id == "m1");
    ensure!(e.hand_no == 3);
    ensure!(e.ts == now);
    ensure!(e.kind == EpisodeKind::Loner);
    ensure!(
        e.summary == "h3: I went alone and marched it",
        "{}",
        e.summary
    );
    ensure!((e.salience - 0.9).abs() < 1e-9);
    ensure!(e.episode_id == format!("{}.m1.3.loner", s.persona.id));
    Ok(())
}

#[test]
fn record_tokens_body_passes_the_profile_parser() -> Result<()> {
    let mut s = state()?;
    s.budget.tokens_in = 1234;
    s.budget.tokens_out = 56;
    s.budget.cache_read_tokens = 7;
    s.budget.calls = 3;
    let flush = on_game_end(
        &mut s,
        &lifecycle(AiSeatLifecycleKind::GameEnd, None),
        Some(PROFILE_TOKEN),
    )
    .context("game end dropped")?;
    ensure!(flush.episodes == EpisodeFlush::Nothing);
    let tokens = flush.tokens.context("no token flush")?;
    ensure!(tokens.profile_key == ["user", "u1"]);
    let wire = serde_json::to_value(&tokens.body)?;
    ensure!(
        wire == json!({
            "internalToken": PROFILE_TOKEN,
            "tokensIn": 1234,
            "tokensOut": 56,
            "cacheReadTokens": 7,
            "calls": 3
        }),
        "{wire}"
    );
    ensure!(body_internal_token(&wire) == Some(PROFILE_TOKEN));
    let inc = TokenIncrement::from_body(&wire);
    ensure!(
        (
            inc.tokens_in,
            inc.tokens_out,
            inc.cache_read_tokens,
            inc.calls
        ) == (1234, 56, 7, 3),
        "{inc:?}"
    );
    Ok(())
}

#[test]
fn every_episode_kind_the_seat_can_send_is_accepted() -> Result<()> {
    // `kind` is the only enum in the body: each value the seat can produce
    // must be one the profile stores.
    for kind in EpisodeKind::ALL {
        let body = json!({
            "internalToken": PROFILE_TOKEN,
            "episodes": [{
                "role": "rita", "matchId": "m1", "handNo": 0, "ts": NOW,
                "kind": kind.as_str(), "summary": "s", "salience": 0.5
            }]
        });
        let stored = parse_record_episodes(&body, i64::try_from(NOW)?)
            .map_err(|e| anyhow::anyhow!("{}: {e}", kind.as_str()))?;
        ensure!(stored.len() == 1 && stored[0].kind == *kind);
    }
    let _ = Value::Null;
    Ok(())
}
