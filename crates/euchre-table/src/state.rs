//! The table's persisted state (TS `TableState`, 03 §3.1) plus the Rust-only
//! due-time ledger (03 §16.5).
//!
//! The whole value is one JSON row in the actor's SQLite database
//! ([`crate::actor::STATE_TABLE`]), reloaded in `create` on every wake. It holds
//! the full `GameState` (every hand, the kitty, the seed) and never leaves the
//! server: clients only ever see `project(game, seat)`.

use euchre_engine::{
    GameInit, GameState, HandResult, HandState, PlayerAction, SchemaV1, TableObservation, team_of,
    try_create_game,
};
use euchre_protocol::serde_util::double_option;
use euchre_protocol::{MatchStats, PersonaAssignment, TableCreateInput};
use serde::{Deserialize, Serialize};

use crate::create_input;

/// Cap of the per-hand replay journal (TS `.slice(-40)`).
pub const JOURNAL_CAP: usize = 40;
/// Cap of the public observations (TS `.slice(-32)`).
pub const OBSERVATIONS_CAP: usize = 32;
/// Cap of the client move-id ring (TS `.slice(-32)`, V18).
pub const MOVE_ID_RING: usize = 32;
/// Longest kept `clientMoveId`, in UTF-16 units (TS `.slice(0, 64)`).
pub const CLIENT_MOVE_ID_UNITS: usize = 64;

/// When the current turn opened, for the observation timings (TS `turnOpened`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnOpened {
    /// The turn nonce.
    pub turn_id: String,
    /// Epoch ms the turn opened.
    pub at: u64,
    /// Whether a player connection was live (only then is a human timing reliable).
    pub connected: bool,
}

/// A due time keyed by the turn it belongs to.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnDue {
    /// The turn the timer belongs to.
    pub turn_id: String,
    /// Epoch ms the timer is due.
    pub at: u64,
}

/// Due times of the timers whose meaning `GameState` does not already hold
/// (03 §16.5, D20). The AI reveal and watchdog use `pending.revealAt` and
/// `turnDeadlineAt`. Reconcile re-arms from these on every wake, because a
/// scheduled fire is at-most-once.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    /// The read pause (`onTempoGate`) of a `trick_resolve` / `hand_score` turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo: Option<TurnDue>,
    /// The self-reap (`onReap`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reap_at: Option<u64>,
    /// The `recordGame` retry (`flushProfile`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_retry_at: Option<u64>,
}

impl Ledger {
    /// True when no timer is recorded (the field is then left out of the row).
    pub fn is_empty(&self) -> bool {
        self.tempo.is_none() && self.reap_at.is_none() && self.profile_retry_at.is_none()
    }
}

/// The table's whole persisted state. Field order follows TS `createState`;
/// the fields TS adds later (`reviewTricks`, `reviewReadyTurnId`, `turnOpened`)
/// follow, then the Rust-only [`Ledger`].
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableState {
    /// Always `1`.
    pub schema: SchemaV1,
    /// Same value as `game.gameId`.
    pub match_id: String,
    /// Empty until the first authenticated connection claims the table.
    pub owner_user_id: String,
    /// The profile the history is filed under, when not the owner's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_user_id: Option<String>,
    /// Epoch ms the table was created.
    pub started_at: u64,
    /// The authoritative match state. Never sent to a client.
    pub game: GameState,
    /// The three opponents, snapshotted at creation.
    pub personas: Vec<PersonaAssignment>,
    /// The current hand's attributed actions. Cap [`JOURNAL_CAP`].
    pub journal: Vec<PlayerAction>,
    /// Public observations of the current hand. Cap [`OBSERVATIONS_CAP`].
    #[serde(default)]
    pub observations: Vec<TableObservation>,
    /// Match counters.
    pub stats: MatchStats,
    /// Finished hands.
    pub hands_played: u32,
    /// Guards the once-per-hand archive and journal flush (`-1` before any).
    pub last_journaled_hand_no: i64,
    /// Guards the once-per-hand `resetHand` fan-out (`-1` before any).
    pub last_reset_hand_no: i64,
    /// Set once the match is recorded (or cannot be). Gates the reap.
    pub recorded_at: Option<u64>,
    /// Legacy counter; human timeouts have no game effect.
    pub abandon_strikes: u32,
    /// The read-pause schedule id.
    pub tempo_id: Option<String>,
    /// Legacy abandon schedule id.
    pub abandon_id: Option<String>,
    /// The `flushProfile` schedule id.
    pub profile_retry_id: Option<String>,
    /// The `onReap` schedule id.
    pub reap_id: Option<String>,
    /// The player's trick-review preference; absent until first set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_tricks: Option<bool>,
    /// The `trick_resolve` turn held for the player's Continue. Absent, `null`
    /// (cleared) or the turn id, as TS distinguishes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub review_ready_turn_id: Option<Option<String>>,
    /// When the current turn opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_opened: Option<TurnOpened>,
    /// Rust-only due times (03 §16.5).
    #[serde(default, skip_serializing_if = "Ledger::is_empty")]
    pub ledger: Ledger,
}

impl TableState {
    /// TS `createState(c, input)`: a `lobby` game with a fresh per-match token.
    ///
    /// `game_id` is the key's second segment, `seed` the input seed or 32 hex
    /// characters of CSPRNG output, `internal_token` a v4 UUID.
    ///
    /// The ids, seed and personas are bounded first ([`crate::create_input`]),
    /// so the match's `recordGame` always fits the action limit: personas
    /// and the seed are clamped, an out-of-bounds id refuses the create.
    pub fn create(
        game_id: String,
        seed: String,
        internal_token: String,
        input: &TableCreateInput,
        now: u64,
    ) -> anyhow::Result<TableState> {
        create_input::check_match_id(&game_id)?;
        let (owner_user_id, profile_user_id) = create_input::checked_ids(input)?;
        let seed = create_input::bounded_seed(&seed)?;
        let personas =
            create_input::bounded_personas(input.personas.as_deref().unwrap_or_default());
        let game = try_create_game(GameInit {
            game_id: game_id.clone(),
            seed,
            cfg: input.cfg,
            internal_token: Some(internal_token),
            first_dealer: input.first_dealer,
        })?;
        Ok(TableState {
            schema: SchemaV1,
            match_id: game_id,
            owner_user_id,
            profile_user_id,
            started_at: now,
            game,
            personas,
            journal: Vec::new(),
            observations: Vec::new(),
            stats: MatchStats::default(),
            hands_played: 0,
            last_journaled_hand_no: -1,
            last_reset_hand_no: -1,
            recorded_at: None,
            abandon_strikes: 0,
            tempo_id: None,
            abandon_id: None,
            profile_retry_id: None,
            reap_id: None,
            review_tricks: None,
            review_ready_turn_id: None,
            turn_opened: None,
            ledger: Ledger::default(),
        })
    }

    /// The profile the history is filed under (`profileUserId ?? ownerUserId`).
    pub fn profile_user(&self) -> &str {
        self.profile_user_id
            .as_deref()
            .unwrap_or(&self.owner_user_id)
    }

    /// The persona assigned to `seat`, if any.
    pub fn persona_for(&self, seat: euchre_engine::Seat) -> Option<&PersonaAssignment> {
        self.personas.iter().find(|p| p.seat == seat)
    }
}

/// TS `accumulateStats(prev, hand)`: fold one finished hand into the counters,
/// from the human's (team 0) point of view.
pub fn accumulate_stats(prev: &MatchStats, hand: &HandState) -> MatchStats {
    let result = hand.result;
    let maker_team = hand.maker_seat.map(|s| team_of(s).get());
    let euchre = result == Some(HandResult::Euchre);
    let lone = hand.alone_seat.is_some();
    let add = |cond: bool| u32::from(cond);
    MatchStats {
        euchres_for: prev.euchres_for + add(euchre && maker_team == Some(1)),
        euchres_against: prev.euchres_against + add(euchre && maker_team == Some(0)),
        loners_attempted: prev.loners_attempted + add(lone),
        loners_made: prev.loners_made
            + add(lone
                && matches!(
                    result,
                    Some(HandResult::LonePoint) | Some(HandResult::LoneMarch)
                )),
        marches: prev.marches
            + add(matches!(
                result,
                Some(HandResult::March) | Some(HandResult::LoneMarch)
            )),
        throw_ins: prev.throw_ins + add(result == Some(HandResult::ThrowIn)),
        tricks: [
            prev.tricks[0] + u32::from(hand.tricks_won[0]),
            prev.tricks[1] + u32::from(hand.tricks_won[1]),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fresh() -> TableState {
        let input = TableCreateInput {
            owner_user_id: Some("guest-owner".into()),
            ..TableCreateInput::default()
        };
        TableState::create("g-1".into(), "seed".into(), "tok".into(), &input, 1_000)
            .unwrap_or_else(|e| panic!("create: {e}"))
    }

    #[test]
    fn fresh_state_has_the_ts_shape() {
        let st = fresh();
        let v = serde_json::to_value(&st).unwrap_or(Value::Null);
        let keys: Vec<&str> = v
            .as_object()
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(
            keys,
            [
                "schema",
                "matchId",
                "ownerUserId",
                "startedAt",
                "game",
                "personas",
                "journal",
                "observations",
                "stats",
                "handsPlayed",
                "lastJournaledHandNo",
                "lastResetHandNo",
                "recordedAt",
                "abandonStrikes",
                "tempoId",
                "abandonId",
                "profileRetryId",
                "reapId"
            ]
        );
        assert_eq!(v["recordedAt"], Value::Null);
        assert_eq!(v["lastJournaledHandNo"], json!(-1));
        assert_eq!(v["game"]["phase"], Value::Null, "phase lives in game.hand");
        assert_eq!(v["game"]["hand"]["phase"], json!("lobby"));
        assert_eq!(v["game"]["internalToken"], json!("tok"));
        assert_eq!(st.profile_user(), "guest-owner");
    }

    #[test]
    fn create_bounds_its_input() {
        let mut personas = euchre_protocol::default_personas();
        personas.extend(euchre_protocol::default_personas());
        let input = TableCreateInput {
            owner_user_id: Some("guest-owner".into()),
            profile_user_id: Some(String::new()),
            personas: Some(personas),
            ..TableCreateInput::default()
        };
        let st = TableState::create("g-1".into(), "x".repeat(300), "tok".into(), &input, 1)
            .unwrap_or_else(|e| panic!("create: {e}"));
        assert_eq!(st.personas, euchre_protocol::default_personas());
        assert_eq!(st.game.seed.len(), create_input::SEED_MAX_UNITS);
        assert_eq!(st.profile_user_id, None);
        assert_eq!(st.profile_user(), "guest-owner");

        let create = |game_id: &str, seed: &str, input: &TableCreateInput| {
            TableState::create(game_id.into(), seed.into(), "tok".into(), input, 1)
        };
        assert!(
            create("g.1", "seed", &input).is_err(),
            "match id off the pattern"
        );
        assert!(create("g-1", "", &input).is_err(), "empty seed");
        let long_owner = TableCreateInput {
            owner_user_id: Some("u".repeat(129)),
            ..TableCreateInput::default()
        };
        assert!(
            create("g-1", "seed", &long_owner).is_err(),
            "owner over 128 units"
        );
    }

    #[test]
    fn review_ready_distinguishes_absent_null_and_set() {
        let mut st = fresh();
        let absent = serde_json::to_value(&st).unwrap_or(Value::Null);
        assert!(absent.get("reviewReadyTurnId").is_none());
        st.review_ready_turn_id = Some(None);
        let null = serde_json::to_value(&st).unwrap_or(Value::Null);
        assert_eq!(null.get("reviewReadyTurnId"), Some(&Value::Null));
        st.review_ready_turn_id = Some(Some("t".into()));
        let text = serde_json::to_string(&st).unwrap_or_default();
        let back: TableState =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("round trip: {e}"));
        assert_eq!(back, st);
        let cleared: TableState = serde_json::from_value(null).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cleared.review_ready_turn_id, Some(None));
    }

    #[test]
    fn ledger_is_left_out_until_used() {
        let mut st = fresh();
        let v = serde_json::to_value(&st).unwrap_or(Value::Null);
        assert!(v.get("ledger").is_none());
        st.ledger.reap_at = Some(5);
        let v = serde_json::to_value(&st).unwrap_or(Value::Null);
        assert_eq!(v["ledger"], json!({ "reapAt": 5 }));
    }

    #[test]
    fn pre_upgrade_rows_without_observations_load() {
        let st = fresh();
        let mut v = serde_json::to_value(&st).unwrap_or(Value::Null);
        if let Some(m) = v.as_object_mut() {
            m.shift_remove("observations");
        }
        let back: TableState = serde_json::from_value(v).unwrap_or_else(|e| panic!("{e}"));
        assert!(back.observations.is_empty());
    }
}
