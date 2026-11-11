//! JSON goldens: every fixture entry written by `scripts/gen-fixtures.ts` from
//! the real TS engine deserializes into its Rust type and re-serializes to the
//! same bytes. Optional (`T?`) fields are omitted; nullable (`T | null`) fields
//! are present as `null`.

mod common;

use common::{assert_absent, assert_bytes, assert_null, compact, entry, round_trip};
use euchre_protocol::*;
use serde_json::{Value, json};

/* -------------------------------------------------------------------------- */
/* Views and sync                                                              */
/* -------------------------------------------------------------------------- */

#[test]
fn table_view_round_trips_with_metadata_last() {
    let json = compact(&entry("view", "tableView"));
    let view: PublicGameView = serde_json::from_str(&json).expect("table view");
    let back = serde_json::to_string(&WireView(&view)).expect("serialize");
    assert_eq!(back, json);
    assert!(view.players.is_some() && view.rules.is_some());
    assert_null(&WireView(&view), &["trick/ledSuit", "trick/winnerSeat"]);
    let players = view.players.as_deref().expect("players");
    assert_eq!(table_players(&default_personas()), players);
}

#[test]
fn pure_projection_omits_actor_metadata() {
    let view: PublicGameView = round_trip("view", "pureProject");
    assert_absent(
        &WireView(&view),
        &[
            "players",
            "observations",
            "reviewTricks",
            "rules",
            "awaitingTrickReview",
        ],
    );
    let _: PublicGameView = round_trip("view", "storeTest");
}

#[test]
fn sync_events_round_trip() {
    for key in ["first", "afterPlay", "deal", "reviewGate"] {
        let sync: SyncEvent = round_trip("sync", key);
        assert_eq!(sync.v, sync.view.v, "{key}: v equals view.v");
    }
    let first: SyncEvent = round_trip("sync", "first");
    assert!(first.steps.is_empty());
}

#[test]
fn sync_event_steps_default_to_empty() {
    let mut v = entry("sync", "first");
    if let Value::Object(m) = &mut v {
        m.shift_remove("steps");
    }
    let sync: SyncEvent = serde_json::from_value(v).expect("steps optional on read");
    assert!(sync.steps.is_empty());
}

#[test]
fn every_step_type_round_trips() {
    let steps: Vec<Step> = common::round_trip_value("steps", &common::fixture("steps"));
    assert_eq!(steps.len(), 12);
}

#[test]
fn step_nulls_are_present() {
    let steps: Vec<Step> = serde_json::from_value(common::fixture("steps")).expect("steps");
    assert_null(
        &steps,
        &["1/packets/1/cards", "5/aloneSeat", "5/sittingSeat"],
    );
}

/* -------------------------------------------------------------------------- */
/* Events                                                                      */
/* -------------------------------------------------------------------------- */

#[test]
fn events_round_trip() {
    let _: ThinkingEvent = round_trip("events", "thinking");
    for key in ["chatCall", "chatSystem", "chatBanter"] {
        let _: ChatEvent = round_trip("events", key);
    }
    let _: ChatDeltaEvent = round_trip("events", "chatDelta");
    let _: PresenceEvent = round_trip("events", "presence");
}

#[test]
fn event_name_constants_match_ts() {
    assert_bytes(&TABLE_EVENT_NAMES, "events", "tableEventNames");
    assert_bytes(&PER_CONNECTION_EVENTS, "events", "perConnectionEvents");
    let names: Vec<&str> = TableEventName::ALL.iter().map(|n| n.as_str()).collect();
    assert_eq!(names, TABLE_EVENT_NAMES);
    assert!(TableEventName::Sync.is_per_connection());
    assert!(!TableEventName::Chat.is_per_connection());
}

#[test]
fn table_events_decode_by_name() {
    let e = TableEvent::decode("chat", entry("events", "chatBanter")).expect("chat");
    assert_eq!(e.name(), TableEventName::Chat);
    let e = TableEvent::decode("sync", entry("sync", "afterPlay")).expect("sync");
    assert_eq!(e.name(), TableEventName::Sync);
    assert!(TableEvent::decode("nope", json!({})).is_err());
    assert!(TableEvent::decode("presence", json!({"seat": 0})).is_err());
}

#[test]
fn chat_final_must_be_true() {
    let mut v = entry("events", "chatCall");
    v["final"] = json!(false);
    assert!(serde_json::from_value::<ChatEvent>(v).is_err());
}

#[test]
fn seats_out_of_range_are_rejected() {
    assert!(serde_json::from_value::<PresenceEvent>(json!({"seat": 4, "online": true})).is_err());
    // A connect-time seat claim is not parsed as a `Seat`: the table refuses an
    // out-of-range claim with `seat_mismatch`, as TS does, rather than failing
    // the parse (see `connect_claims_follow_the_ts_guard`).
    let claim: TableConnectParams =
        serde_json::from_value(json!({"token": "t", "seat": 7})).expect("raw claim");
    assert_eq!(claim.claim_error(), Some(ProtocolErrorCode::SeatMismatch));
    let mut sync = entry("sync", "first");
    sync["view"]["you"] = json!(9);
    assert!(serde_json::from_value::<SyncEvent>(sync).is_err());
    let mut rec = entry("records", "matchRecord");
    rec["winnerTeam"] = json!(2);
    assert!(serde_json::from_value::<MatchRecord>(rec).is_err());
}

/* -------------------------------------------------------------------------- */
/* Actions                                                                     */
/* -------------------------------------------------------------------------- */

#[test]
fn submit_move_and_acks_round_trip() {
    let req: SubmitMoveRequest = round_trip("actions", "submitMove");
    assert!(req.turn_id.is_some() && req.has_client_move_id());
    let ok: MoveAck = round_trip("actions", "ackOk");
    assert_eq!(ok, MoveAck::Ok { v: 17 });
    let err: MoveAck = round_trip("actions", "ackErr");
    match err {
        MoveAck::Rejected { code, legal } => {
            assert_eq!(code, ErrorCode::Rule(RuleCode::StaleTurn));
            assert_eq!(legal.len(), 3);
        }
        MoveAck::Ok { .. } => panic!("ackErr is a rejection"),
    }
    let done: MoveAck = round_trip("actions", "ackGameComplete");
    assert_eq!(
        done,
        MoveAck::Rejected {
            code: ErrorCode::Protocol(ProtocolErrorCode::GameComplete),
            legal: vec![]
        }
    );
}

/// D9 and the TS `submitMove` check order: no field problem fails the parse,
/// so the table can answer `illegal_move` / `internal_error` / no stale check.
#[test]
fn submit_move_reads_leniently_for_the_ts_check_order() {
    // No turnId: accepted, and the stale check is skipped (apply turnId=undefined).
    let no_turn: SubmitMoveRequest =
        serde_json::from_value(json!({"moveId": "play:JH", "clientMoveId": "x"}))
            .expect("missing turnId is accepted");
    assert_eq!(no_turn.turn_id, None);
    assert_eq!(no_turn.move_id, "play:JH");
    assert!(no_turn.has_client_move_id());
    assert_eq!(
        serde_json::to_string(&no_turn).expect("ser"),
        r#"{"moveId":"play:JH","clientMoveId":"x"}"#
    );
    // No clientMoveId: parses, so an unknown moveId still reads as illegal_move first.
    let bogus: SubmitMoveRequest =
        serde_json::from_value(json!({"moveId": "bogus", "turnId": "t"})).expect("parses");
    assert_eq!(bogus.move_id, "bogus");
    assert!(!bogus.has_client_move_id());
    // A legal moveId with no clientMoveId reaches the internal_error check.
    let missing: SubmitMoveRequest =
        serde_json::from_value(json!({"moveId": "play:JH", "turnId": "t"})).expect("parses");
    assert_eq!(missing.turn_id.as_deref(), Some("t"));
    assert!(!missing.has_client_move_id());
    let numeric: SubmitMoveRequest =
        serde_json::from_value(json!({"moveId": "play:JH", "turnId": "t", "clientMoveId": 7}))
            .expect("parses");
    assert!(!numeric.has_client_move_id(), "typeof !== 'string'");
    // A present non-string turnId is stale (null !== state.turnId), never skipped.
    let null_turn: SubmitMoveRequest =
        serde_json::from_value(json!({"moveId": "play:JH", "turnId": null, "clientMoveId": "x"}))
            .expect("parses");
    assert_eq!(null_turn.turn_id.as_deref(), Some(""));
    // Not an object at all (TS `request?.moveId`): every field empty.
    for raw in [Value::Null, json!("play:JH"), json!([1, 2])] {
        let r: SubmitMoveRequest = serde_json::from_value(raw).expect("parses");
        assert_eq!(r.move_id, "");
        assert_eq!(r.turn_id, None);
        assert!(!r.has_client_move_id());
    }
    let msg = MoveMsg {
        seq: 1,
        turn_id: None,
        move_id: "play:JH".into(),
        client_move_id: "x".into(),
        user_id: "u".into(),
    };
    assert_eq!(
        serde_json::to_string(&msg).expect("ser"),
        r#"{"seq":1,"moveId":"play:JH","clientMoveId":"x","userId":"u"}"#
    );
}

/// `js_f64` against `JSON.stringify` for a few hundred TS-written numbers.
#[test]
fn js_numbers_match_json_stringify() {
    #[derive(serde::Serialize)]
    struct N(#[serde(with = "euchre_protocol::serde_util::js_f64")] f64);
    let cases = common::fixture("numbers");
    let cases = cases.as_array().expect("array");
    assert!(cases.len() > 200);
    for case in cases {
        let bits = case[0].as_str().expect("bits");
        let want = case[1].as_str().expect("text");
        let v = f64::from_bits(u64::from_str_radix(bits, 16).expect("hex"));
        assert_eq!(serde_json::to_string(&N(v)).expect("ser"), want, "{bits}");
        assert_eq!(serde_util::js_number_string(v), want, "{bits}");
    }
}

#[test]
fn review_requests_round_trip() {
    let set: ReviewTrickRequest = round_trip("actions", "reviewSet");
    assert_eq!(set, ReviewTrickRequest::Set { enabled: true });
    let cont: ReviewTrickRequest = round_trip("actions", "reviewContinue");
    assert!(matches!(cont, ReviewTrickRequest::Continue { .. }));
    let _: ReviewTrickAck = round_trip("actions", "reviewAck");
    assert_bytes(&ReviewTrickAck::default(), "actions", "reviewAck");
}

#[test]
fn connect_params_round_trip() {
    let p: TableConnectParams = round_trip("actions", "connectParams");
    assert_absent(&p, &["internalToken", "seat"]);
    assert_eq!(
        p,
        TableConnectParams::player("eyJhbGciOiJIUzI1NiJ9.e30.sig")
    );
    let internal: TableConnectParams = round_trip("actions", "connectParamsInternal");
    assert_eq!(internal.token, "");
    let _: TableConnectParams = round_trip("actions", "connectParamsFull");
    let bare: TableConnectParams =
        serde_json::from_value(json!({"internalToken": "x"})).expect("token defaults");
    assert_eq!(bare.token, "");
    // `internalToken: null` reads as absent (TS `tokenOk` is false), but a
    // present `seat: null` is kept: TS `null !== undefined` refuses it.
    let nulls: TableConnectParams =
        serde_json::from_value(json!({"token": "t", "seat": null, "internalToken": null}))
            .expect("null optional");
    assert_eq!(
        serde_json::to_string(&nulls).expect("ser"),
        r#"{"token":"t","seat":null}"#
    );
    assert_eq!(nulls.claim_error(), Some(ProtocolErrorCode::SeatMismatch));
    assert_eq!(p.claim_error(), None);
    let full: TableConnectParams = round_trip("actions", "connectParamsFull");
    assert!(full.seat_claim_ok() && full.protocol_version_ok());
    assert_eq!(
        serde_json::to_value(
            TableConnectParams::player("t").with_seat(Seat::new(0).expect("seat"))
        )
        .expect("ser"),
        json!({"token": "t", "seat": 0, "protocolVersion": 1})
    );
}

/// The TS `createConnState` claim checks, after the internal-token path:
/// `protocolVersion !== undefined && !== 1` -> `protocol_version_mismatch`,
/// then `seat !== undefined && !== 0` -> `seat_mismatch` (strict `===`).
#[test]
fn connect_claims_follow_the_ts_guard() {
    use ProtocolErrorCode::{ProtocolVersionMismatch as Version, SeatMismatch as Seat};
    let cases = [
        (json!({"token": "t"}), None),
        (json!({"token": "t", "seat": 0, "protocolVersion": 1}), None),
        (
            json!({"token": "t", "seat": 0.0, "protocolVersion": 1.0}),
            None,
        ),
        (json!({"token": "t", "seat": -0.0}), None),
        (json!({"token": "t", "seat": 7}), Some(Seat)),
        (json!({"token": "t", "seat": 1}), Some(Seat)),
        (json!({"token": "t", "seat": "0"}), Some(Seat)),
        (json!({"token": "t", "seat": null}), Some(Seat)),
        (json!({"token": "t", "seat": 0.5}), Some(Seat)),
        (json!({"token": "t", "seat": [0]}), Some(Seat)),
        (json!({"token": "t", "protocolVersion": 1.5}), Some(Version)),
        (json!({"token": "t", "protocolVersion": -1}), Some(Version)),
        (json!({"token": "t", "protocolVersion": "1"}), Some(Version)),
        (
            json!({"token": "t", "protocolVersion": null}),
            Some(Version),
        ),
        (json!({"token": "t", "protocolVersion": 2}), Some(Version)),
        // The version check runs first.
        (
            json!({"token": "t", "seat": 3, "protocolVersion": 2}),
            Some(Version),
        ),
    ];
    for (params, want) in cases {
        let parsed: TableConnectParams =
            serde_json::from_value(params.clone()).unwrap_or_else(|e| panic!("{params}: {e}"));
        assert_eq!(parsed.claim_error(), want, "{params}");
        assert_eq!(
            serde_json::to_value(&parsed).expect("ser"),
            params,
            "raw claims round-trip"
        );
    }
    // `params?.token ?? ''`: a missing, null or non-string token reads as ''
    // and is refused later with `invalid_token`, not by the parser.
    for params in [json!({}), json!({"token": null}), json!({"token": 5})] {
        let parsed: TableConnectParams = serde_json::from_value(params).expect("lenient token");
        assert_eq!(parsed.token, "");
    }
    let odd: TableConnectParams =
        serde_json::from_value(json!({"internalToken": 5, "token": "t"})).expect("lenient");
    assert_eq!(
        odd.internal_token, None,
        "a non-string internal token never matches"
    );
}

#[test]
fn conn_state_and_session_round_trip() {
    let s: TableConnState = round_trip("actions", "connState");
    assert_eq!(s.role, ConnRole::Player);
    let i: TableConnState = round_trip("actions", "connStateInternal");
    assert_eq!(i.role, ConnRole::Internal);
    let _: TableSessionResponse = round_trip("actions", "tableSession");
    let bare: TableSessionResponse = round_trip("actions", "tableSessionNoGateway");
    assert_absent(&bare, &["gatewayToken"]);
    assert!(!bare.is_stale(bare.expires_at - 10_001));
    assert!(bare.is_stale(bare.expires_at - 10_000));
}

#[test]
fn create_input_round_trips() {
    let full: TableCreateInput = round_trip("actions", "createInput");
    assert_eq!(
        full.personas.as_deref(),
        Some(default_personas().as_slice())
    );
    let empty: TableCreateInput = round_trip("actions", "createInputEmpty");
    assert_eq!(empty, TableCreateInput::default());
    assert_eq!(serde_json::to_string(&empty).expect("ser"), "{}");
}

#[test]
fn errors_round_trip() {
    let ue: UserErrorPayload = round_trip("actions", "userError");
    assert_eq!(ue.code, ErrorCode::Rule(RuleCode::IllegalMove));
    assert_eq!(ue.illegal_move_legal().map(|l| l.len()), Some(3));
    let pe: ProtocolError = round_trip("actions", "protocolError");
    assert_eq!(pe.retryable, Some(false));
    let bare: ProtocolError = round_trip("actions", "protocolErrorBare");
    assert_absent(&bare, &["retryable"]);
}

#[test]
fn internal_queue_bodies_round_trip() {
    let _: MoveMsg = round_trip("actions", "moveMsg");
    let set: ReviewMsg = round_trip("actions", "reviewMsgSet");
    assert_eq!(set.enabled, Some(true));
    let cont: ReviewMsg = round_trip("actions", "reviewMsgContinue");
    assert_absent(&cont, &["enabled"]);
    let tick: TickMsg = round_trip("actions", "tickMsg");
    assert_absent(&tick, &["seat"]);
    let timeout: TickMsg = round_trip("actions", "tickMsgTimeout");
    assert_eq!(timeout.kind, TickKind::AiTimeout);
}

/* -------------------------------------------------------------------------- */
/* AI                                                                          */
/* -------------------------------------------------------------------------- */

#[test]
fn ai_decide_requests_round_trip_with_view_in_ts_order() {
    let r: AIDecideRequest = round_trip("ai", "decideRequest");
    assert_absent(&r, &["ranking"]);
    assert!(r.view.players.is_some(), "AI views carry actor metadata");
    let ranked: AIDecideRequest = round_trip("ai", "decideRequestRanked");
    let scores: Vec<f64> = ranked
        .ranking
        .as_deref()
        .expect("ranking")
        .iter()
        .map(|m| m.score)
        .collect();
    assert_eq!(scores, [12.0, 2.25, 1.25, 0.25, -0.75]);
}

#[test]
fn ai_decisions_round_trip() {
    let d: AIDecision = round_trip("ai", "decision");
    assert_absent(&d, &["banter"]);
    let full: AIDecision = round_trip("ai", "decisionFull");
    assert_eq!(full.source, AiDecisionSource::Fallback);
    let null_banter: AIDecision = serde_json::from_value({
        let mut v = entry("ai", "decision");
        v["banter"] = Value::Null;
        v
    })
    .expect("null optional");
    assert_eq!(null_banter.banter, None);
}

#[test]
fn ai_say_and_lifecycle_round_trip() {
    let fin: AiSayMsg = round_trip("ai", "sayFinal");
    assert_absent(&fin, &["delta"]);
    // The real aiSeat producer's key order (ai-seat/index.ts:400-409).
    let keys: Vec<String> = serde_json::to_value(&fin)
        .expect("ser")
        .as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        [
            "internalToken",
            "gameId",
            "seat",
            "turnId",
            "handNo",
            "msgId",
            "text",
            "final"
        ]
    );
    let delta: AiSayMsg = round_trip("ai", "sayDelta");
    assert_absent(&delta, &["text", "handNo"]);
    let _: AiSeatLifecycleMessage = round_trip("ai", "lifecycleResetHand");
    let end: AiSeatLifecycleMessage = round_trip("ai", "lifecycleGameEnd");
    assert_absent(&end, &["handNo"]);
}

#[test]
fn default_personas_match_ts() {
    let personas: Vec<PersonaAssignment> = round_trip("ai", "personas");
    assert_eq!(personas, default_personas());
    assert_bytes(&default_personas(), "ai", "personas");
    let _: PersonaView = round_trip("ai", "personaView");
}

/* -------------------------------------------------------------------------- */
/* Hand review                                                                 */
/* -------------------------------------------------------------------------- */

#[test]
fn hand_reviews_round_trip() {
    let scored: CompletedHandReview = round_trip("review", "scored");
    assert_eq!(scored.tricks.len(), 5);
    let throw_in: CompletedHandReview = round_trip("review", "throwIn");
    assert_null(&throw_in, &["trump", "makerSeat", "result", "delta"]);
    assert_eq!(throw_in.unplayed.len(), 20);
    let legacy: CompletedHandReview = round_trip("review", "legacy");
    assert_absent(&legacy, &["dealerSeat", "tricksWon", "upCard"]);
    let null_up: CompletedHandReview = round_trip("review", "nullUpCard");
    assert_eq!(null_up.up_card, Some(None));
    assert_null(&null_up, &["upCard"]);
}

#[test]
fn build_hand_review_matches_archive_hand() {
    for (hand_key, review_key) in [("scoredHand", "scored"), ("throwInHand", "throwIn")] {
        let hand: HandStateFixture =
            serde_json::from_value(entry("review", hand_key)).expect("hand state");
        assert_bytes(&build_hand_review(&hand.0), "review", review_key);
    }
}

/// The engine's `HandState`, which this crate deliberately does not re-export.
#[derive(serde::Deserialize)]
#[serde(transparent)]
struct HandStateFixture(euchre_engine::HandState);

/* -------------------------------------------------------------------------- */
/* Records and pacing                                                          */
/* -------------------------------------------------------------------------- */

#[test]
fn records_round_trip() {
    let _: MatchRecord = round_trip("records", "matchRecord");
    let live: MatchRecord = round_trip("records", "liveMatchRecord");
    assert_null(&live, &["endedAt", "winnerTeam"]);
    let _: HandJournalEntry = round_trip("records", "handJournalEntry");
    let _: MatchSummary = round_trip("records", "matchSummary");
    let page: MatchListPage = round_trip("records", "matchListPage");
    assert_null(&page, &["cursor"]);
    let _: LifetimeStats = round_trip("records", "lifetimeStats");
    let state: PlayerProfileState = round_trip("records", "playerProfileState");
    assert_null(&state, &["activeGameId"]);
    let _: EpisodeRecord = round_trip("records", "episode");
}

#[test]
fn nullable_fields_are_required_on_read() {
    let mut page = entry("records", "matchListPage");
    if let Value::Object(m) = &mut page {
        m.shift_remove("cursor");
    }
    // serde treats a missing Option as None; the TS type requires the key, and
    // we always write it, so a missing key still reads but writes back as null.
    let p: MatchListPage = serde_json::from_value(page).expect("reads");
    assert_eq!(
        serde_json::to_string(&p).expect("ser"),
        r#"{"rows":[],"cursor":null}"#
    );
}

#[test]
fn pacing_matches_ts() {
    let v = common::fixture("pacing");
    assert_eq!(
        serde_json::to_string(&TABLE_MOTION).expect("ser"),
        compact(&v["tableMotion"])
    );
    assert_eq!(v["trickPresentationMs"], json!(TRICK_PRESENTATION_MS));
}
