//! Goldens generated from the real TS profile code by `scripts/gen-fixtures.ts`.
//! Every expected value is `JSON.stringify` text; Rust output must match it
//! byte for byte.

use std::collections::BTreeMap;
use std::path::PathBuf;

use euchre_profile_model::guards::SanitizeOptions;
use euchre_profile_model::sql::{
    CLIENT_REPLAY_HANDS_SQL, CLIENT_REPLAY_MATCH_SQL, EPISODE_INSERT_SQL, EPISODES_LIST_SQL,
    MATCH_EXISTING_SQL, SERVER_REPLAY_HANDS_SQL, SERVER_REPLAY_MATCH_SQL, STATS_SQL, UpsertPlan,
    hand_columns, hand_insert_sql, match_columns, match_insert_sql, match_update_params,
    match_update_sql, migration_ids, upsert_plan,
};
use euchre_profile_model::*;
use euchre_protocol::ProtocolErrorCode;
use serde::Serialize;
use serde_json::Value;

fn load(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    serde_json::from_str(&text).expect("fixture is JSON")
}

fn s(v: &Value) -> &str {
    v.as_str().expect("string")
}

fn arr(v: &Value) -> &Vec<Value> {
    v.as_array().expect("array")
}

fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("serializes")
}

/// Compares a result with a case's `ok` text or `error` object.
fn check<T: Serialize>(label: &str, case: &Value, got: ProfileResult<T>) {
    match (case.get("ok"), case.get("error"), got) {
        (Some(ok), None, Ok(v)) => assert_eq!(json(&v), s(ok), "{label}"),
        (None, Some(err), Err(e)) => {
            assert_eq!(e.code.as_str(), s(&err["code"]), "{label} code");
            assert_eq!(e.message, s(&err["message"]), "{label} message");
        }
        (ok, err, got) => panic!(
            "{label}: expected ok={ok:?} error={err:?}, got {:?}",
            got.map(|v| json(&v))
        ),
    }
}

fn column(v: &Value) -> Column {
    match v {
        Value::Null => Column::Null,
        Value::Number(n) => match n.as_i64() {
            Some(i) => Column::Integer(i),
            None => Column::Float(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(t) => Column::Text(t.clone()),
        Value::Bool(b) => Column::Integer(i64::from(*b)),
        other => Column::Text(other.to_string()),
    }
}

fn row(v: &Value) -> Row {
    v.as_object()
        .expect("row object")
        .iter()
        .map(|(k, v)| (k.clone(), column(v)))
        .collect()
}

fn rows(v: &Value) -> Vec<Row> {
    arr(v).iter().map(row).collect()
}

fn values(cols: Vec<(&'static str, Column)>) -> Vec<Column> {
    cols.into_iter().map(|(_, v)| v).collect()
}

fn code(name: Option<&Value>) -> ProtocolErrorCode {
    name.and_then(Value::as_str)
        .map(|c| c.parse().expect("known code"))
        .unwrap_or(ProtocolErrorCode::InvalidPersona)
}

/* -------------------------------------------------------------------------- */

#[test]
fn sanitize_text_matches_ts() {
    let cases = load("sanitize.json");
    for (i, case) in arr(&cases).iter().enumerate() {
        let field = s(&case["field"]).to_owned();
        let opts = SanitizeOptions {
            multiline: case["multiline"].as_bool().expect("bool"),
            max_chars: case["maxChars"].as_u64().expect("max") as usize,
            field: &field,
            code: code(case.get("code")),
        };
        check(
            &format!("sanitize #{i}"),
            case,
            sanitize_value(case.get("value"), opts),
        );
    }
}

#[test]
fn defaults_presets_and_views_match_ts() {
    let f = load("settings.json");
    let now = f["defaultSettings"]["now"].as_i64().expect("now");
    let settings = default_settings(now);
    assert_eq!(json(&settings), s(&f["defaultSettings"]["ok"]));
    assert_eq!(json(&persona_views(&settings)), s(&f["personaViews"]));
    let with_house = ProfileSettings {
        house_prompt: "Settings house.".to_owned(),
        ..settings.clone()
    };
    assert_eq!(
        json(&personas_from_settings(&with_house)),
        s(&f["personasFromSettings"])
    );
    assert_eq!(json(&PERSONA_PRESETS), s(&f["presets"]));
    assert_eq!(
        JsJson(default_engine_config_value()).to_js_string(),
        s(&f["defaultEngineConfig"])
    );
}

#[test]
fn settings_patch_matches_ts() {
    let f = load("settings.json");
    for case in arr(&f["patch"]) {
        let name = s(&case["name"]);
        let current: ProfileSettings =
            serde_json::from_str(s(&case["current"])).expect("current settings");
        assert_eq!(json(&current), s(&case["current"]), "{name}: round trip");
        let patch = case.get("patch").cloned().unwrap_or(Value::Null);
        let now = case["now"].as_i64().expect("now");
        check(name, case, apply_settings_patch(&current, &patch, now));
    }
}

#[test]
fn normalize_settings_matches_ts() {
    let f = load("settings.json");
    for case in arr(&f["normalize"]) {
        let name = s(&case["name"]);
        let raw = case.get("raw").cloned().unwrap_or(Value::Null);
        let now = case["now"].as_i64().expect("now");
        let got = normalize_settings(&raw, now);
        check::<ProfileSettings>(name, case, Ok(got.clone()));
        // Idempotent.
        let again = normalize_settings(&serde_json::to_value(&got).expect("value"), now);
        assert_eq!(again, got, "{name}: idempotent");
    }
}

#[test]
fn record_parsers_match_ts() {
    let f = load("records.json");
    for case in arr(&f["match"]) {
        let name = s(&case["name"]);
        let now = case["now"].as_i64().expect("now");
        check(
            name,
            case,
            parse_match_record(case.get("raw"), s(&case["owner"]), now),
        );
    }
    for case in arr(&f["hands"]) {
        let name = s(&case["name"]);
        check(
            name,
            case,
            parse_hands(case.get("raw"), s(&case["matchId"])),
        );
    }
    for case in arr(&f["episodes"]) {
        let name = s(&case["name"]);
        let now = case["now"].as_i64().expect("now");
        check(name, case, parse_episodes(case.get("raw"), now));
    }
    for case in arr(&f["clampLimit"]) {
        assert_eq!(
            clamp_limit(case.get("raw")).to_string(),
            s(&case["ok"]),
            "clampLimit {case}"
        );
    }
    let repo = load("repository.json");
    let batch = repo["writes"]
        .as_array()
        .and_then(|w| w.iter().find(|w| w["kind"] == "episodes"))
        .expect("episode write");
    let now = batch["now"].as_i64().expect("now");
    let ids: Vec<String> = parse_episodes(batch.get("raw"), now)
        .expect("episodes")
        .iter()
        .map(|e| episode_id(e.role.as_str(), &e.match_id, e.hand_no, e.kind))
        .collect();
    assert_eq!(serde_json::to_value(&ids).expect("ids"), f["episodeIds"]);
    for case in arr(&f["utcDay"]) {
        let now = case["now"].as_i64().expect("now");
        assert_eq!(state::utc_day(now), s(&case["ok"]), "utcDay {now}");
    }
}

#[test]
fn repository_writes_bind_what_ts_binds() {
    let f = load("repository.json");
    let owner = s(&f["owner"]);
    assert_eq!(
        serde_json::to_value(migration_ids()).expect("ids"),
        f["migrationIds"]
    );
    for w in arr(&f["writes"]) {
        let calls = arr(&w["calls"]);
        match s(&w["kind"]) {
            "match" => {
                let now = w["now"].as_i64().expect("now");
                let record = parse_match_record(w.get("raw"), owner, now).expect("record");
                assert_eq!(json(&record), s(&w["record"]));
                assert_eq!(s(&calls[0]["sql"]), MATCH_EXISTING_SQL);
                let existing = rows(&calls[0]["rows"]);
                let plan = upsert_plan(existing.first().map(|r| r.get("ended_at"))).expect("plan");
                let outcome = sql::RecordMatchOutcome {
                    match_id: record.match_id.clone(),
                    inserted: plan == UpsertPlan::Insert,
                    updated: plan == UpsertPlan::Update,
                };
                assert_eq!(json(&outcome), s(&w["outcome"]));
                match plan {
                    UpsertPlan::Insert => {
                        assert_eq!(s(&calls[1]["sql"]), match_insert_sql());
                        assert_eq!(
                            json(&values(match_columns(&record, now))),
                            json(&calls[1]["args"])
                        );
                    }
                    UpsertPlan::Update => {
                        assert_eq!(s(&calls[1]["sql"]), match_update_sql());
                        assert_eq!(
                            json(&match_update_params(&record, now)),
                            json(&calls[1]["args"])
                        );
                    }
                    UpsertPlan::Skip => assert_eq!(calls.len(), 1),
                }
            }
            "hands" => {
                let hands: Vec<HandRow> = serde_json::from_str(s(&w["rows"])).expect("hands");
                if !w["raw"].is_null() {
                    let parsed = parse_hands(w.get("raw"), s(&w["matchId"])).expect("parse");
                    assert_eq!(json(&parsed), s(&w["rows"]));
                }
                assert_eq!(json(&hands), s(&w["rows"]), "hand rows round trip");
                assert_eq!(calls.len(), hands.len());
                for (hand, call) in hands.iter().zip(calls) {
                    assert_eq!(s(&call["sql"]), hand_insert_sql());
                    assert_eq!(json(&values(hand_columns(hand))), json(&call["args"]));
                }
            }
            "episodes" => {
                let now = w["now"].as_i64().expect("now");
                let parsed = parse_episodes(w.get("raw"), now).expect("episodes");
                assert_eq!(json(&parsed), s(&w["rows"]));
                for (episode, call) in parsed.iter().zip(calls) {
                    assert_eq!(s(&call["sql"]), EPISODE_INSERT_SQL);
                    assert_eq!(json(&sql::episode_params(episode)), json(&call["args"]));
                }
            }
            other => panic!("unknown write {other}"),
        }
    }
}

fn check_list(label: &str, cursor_raw: Option<&Value>, limit: i64, case: &Value) {
    let decoded = match decode_cursor(cursor_raw) {
        Ok(c) => c,
        Err(e) => {
            check::<()>(label, case, Err(e));
            return;
        }
    };
    let call = &arr(&case["calls"])[0];
    let (sql, args) = list_matches_query(limit, decoded.as_ref());
    assert_eq!(sql, s(&call["sql"]), "{label} sql");
    assert_eq!(json(&args), json(&call["args"]), "{label} args");
    check(label, case, build_page(&rows(&call["rows"]), limit));
}

#[test]
fn history_pages_and_cursors_match_ts() {
    let f = load("repository.json");
    for (i, case) in arr(&f["lists"]).iter().enumerate() {
        let label = format!("list #{i} {}", case["req"]);
        let (limit, cursor) = list_games_args(case.get("req"));
        assert_eq!(limit, case["limit"].as_i64().expect("limit"), "{label}");
        check_list(&label, cursor, limit, case);
    }
    let mut pages = 0;
    for (i, case) in arr(&f["walk"]).iter().enumerate() {
        check_list(&format!("walk #{i}"), case.get("cursor"), 2, case);
        pages += 1;
    }
    assert!(pages >= 4, "the walk spans several pages");
}

#[test]
fn replays_redact_and_round_trip_like_ts() {
    let f = load("repository.json");
    for case in arr(&f["replays"]) {
        let id = s(&case["matchId"]);
        let cc = arr(&case["clientCalls"]);
        let sc = arr(&case["serverCalls"]);
        assert_eq!(s(&cc[0]["sql"]), CLIENT_REPLAY_MATCH_SQL, "{id}");
        assert_eq!(s(&sc[0]["sql"]), SERVER_REPLAY_MATCH_SQL, "{id}");
        let client_match = rows(&cc[0]["rows"]);
        let Some(m) = client_match.first() else {
            assert_eq!(s(&case["client"]), "null");
            assert_eq!(s(&case["server"]), "null");
            continue;
        };
        assert_eq!(s(&cc[1]["sql"]), CLIENT_REPLAY_HANDS_SQL, "{id}");
        assert_eq!(s(&sc[1]["sql"]), SERVER_REPLAY_HANDS_SQL, "{id}");
        let client = client_replay_from_rows(m, &rows(&cc[1]["rows"])).expect("client");
        let client_text = json(&client);
        assert_eq!(client_text, s(&case["client"]), "{id} client");
        assert!(!client_text.contains("\"seed\""), "{id}: no seed");
        assert!(!client_text.contains("deckOrder"), "{id}: no deck");

        let server_match = rows(&sc[0]["rows"]);
        let server =
            server_replay_from_rows(&server_match[0], &rows(&sc[1]["rows"])).expect("server");
        assert_eq!(json(&server), s(&case["server"]), "{id} server");
    }
}

#[test]
fn stats_match_ts_from_sql_and_in_memory() {
    let f = load("repository.json");
    let now = f["stats"]["now"].as_i64().expect("now");
    let call = &arr(&f["stats"]["calls"])[0];
    assert_eq!(s(&call["sql"]), STATS_SQL);
    let aggregate = rows(&call["rows"]);
    let from_sql = stats_from_row(aggregate.first(), now).expect("stats");
    assert_eq!(json(&from_sql), s(&f["stats"]["ok"]));

    // The table after every write: inserts and live updates, finished rows immutable.
    let owner = s(&f["owner"]);
    let mut table: BTreeMap<String, MatchRow> = BTreeMap::new();
    for w in arr(&f["writes"]) {
        if s(&w["kind"]) != "match" {
            continue;
        }
        let outcome: Value = serde_json::from_str(s(&w["outcome"])).expect("outcome");
        if outcome["inserted"] == Value::Bool(true) || outcome["updated"] == Value::Bool(true) {
            let now = w["now"].as_i64().expect("now");
            let record = parse_match_record(w.get("raw"), owner, now).expect("record");
            table.insert(record.match_id.clone(), record);
        }
    }
    let rows: Vec<MatchRow> = table.into_values().collect();
    assert_eq!(json(&compute_stats(&rows, now)), s(&f["stats"]["ok"]));
    let lifetime = lifetime_from_stats(&from_sql);
    assert_eq!(lifetime.matches, 7);
    assert_eq!(lifetime.updated_at, now as u64);
}

#[test]
fn episode_ranking_matches_ts_power_order() {
    let f = load("repository.json");
    for case in arr(&f["ranked"]) {
        let label = format!("{} {} {}", case["role"], case["limit"], case["now"]);
        let candidates: Vec<StoredEpisode> = rows(&case["candidates"])
            .iter()
            .map(|r| sql::episode_from_row(r).expect("episode"))
            .collect();
        let limit = case["limit"].as_u64().expect("limit") as usize;
        let now = case["now"].as_i64().expect("now");
        assert_eq!(
            json(&rank_episodes(candidates, limit, now)),
            s(&case["ok"]),
            "{label}"
        );
    }
    let le = &f["listEpisodes"];
    let call = &arr(&le["calls"])[0];
    assert_eq!(s(&call["sql"]), EPISODES_LIST_SQL);
    assert_eq!(
        json(&call["args"]),
        json(&[le["now"].clone(), le["limit"].clone()])
    );
    let listed: Vec<StoredEpisode> = rows(&call["rows"])
        .iter()
        .map(|r| sql::episode_from_row(r).expect("episode"))
        .collect();
    assert_eq!(json(&listed), s(&le["ok"]));
}
