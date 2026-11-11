//! Wire types of every `playerProfile` action (05 §3.8, §3.9, 08 §14).
//!
//! Each action is a newtype over the **whole positional argument array**
//! (`Vec<Value>`): `decode_positional` binds the array to the `Vec` on its first
//! try (G32), so `listGames()`, `listGames(undefined)` (`[null]` in JSON) and
//! `getEpisodesForRole(token, role)` without a `limit` all decode, and the
//! handler validates each argument by hand as TS `guards.ts` does. A shape
//! error therefore surfaces as the TS `user/<code>` error, never as the masked
//! decode failure of G14.
//!
//! Build calls with the `new` constructors, which serialize to the same
//! positional array a TS client sends.

use euchre_profile_model::{
    ClientMatchReplay, ProfileSettings, ProfileStats, ServerMatchReplay, StoredEpisode,
};
use euchre_protocol::{MatchListPage, PersonaView};
use rivetkit::Action;
use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! profile_action {
    ($(#[$doc:meta])* $ty:ident, $name:literal, $out:ty) => {
        $(#[$doc])*
        #[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
        pub struct $ty(pub Vec<Value>);

        impl Action for $ty {
            type Output = $out;
            const NAME: &'static str = $name;
        }

        impl $ty {
            /// The positional argument at `index` (`undefined` and `null` alike
            /// read as absent, as `arg?.x` does in TS).
            pub fn arg(&self, index: usize) -> Option<&Value> {
                self.0.get(index).filter(|v| !v.is_null())
            }
        }
    };
}

profile_action!(
    /// `getSettings()` → `ProfileSettings`. Owner connection only.
    GetSettings,
    "getSettings",
    ProfileSettings
);
profile_action!(
    /// `updateSettings(patch)` → the saved `ProfileSettings`. Owner connection only.
    UpdateSettings,
    "updateSettings",
    ProfileSettings
);
profile_action!(
    /// `getPersonas()` → `PersonaView[]` (seats 1..3). Owner connection only.
    GetPersonas,
    "getPersonas",
    Vec<PersonaView>
);
profile_action!(
    /// `getActiveGame()` → `string | null`. Owner connection only.
    GetActiveGame,
    "getActiveGame",
    Option<String>
);
profile_action!(
    /// `getStats()` → `ProfileStats`, computed from the history. Owner connection only.
    GetStats,
    "getStats",
    ProfileStats
);
profile_action!(
    /// `listGames(req?: { limit?, cursor? })` → `MatchListPage`. Owner connection only.
    ListGames,
    "listGames",
    MatchListPage
);
profile_action!(
    /// `getReplay(matchId)` → `ClientMatchReplay | null` (redacted). Owner connection only.
    GetReplay,
    "getReplay",
    Option<ClientMatchReplay>
);
profile_action!(
    /// `listEpisodes(limit?)` → `StoredEpisode[]`. Owner connection only.
    ListEpisodes,
    "listEpisodes",
    Vec<StoredEpisode>
);
profile_action!(
    /// `forgetEpisode(episodeId)` → `boolean`. Owner connection only.
    ForgetEpisode,
    "forgetEpisode",
    bool
);
profile_action!(
    /// `getServerReplay(internalToken, matchId)` → `ServerMatchReplay | null`
    /// (seeds and deck order included). Internal token only.
    GetServerReplay,
    "getServerReplay",
    Option<ServerMatchReplay>
);
profile_action!(
    /// `getEpisodesForRole(internalToken, role, limit?)` → `StoredEpisode[]`,
    /// ranked by decayed salience. Internal token only.
    GetEpisodesForRole,
    "getEpisodesForRole",
    Vec<StoredEpisode>
);
profile_action!(
    /// `pruneEpisodes(internalToken)` → `null`. Internal token only.
    PruneEpisodes,
    "pruneEpisodes",
    ()
);
profile_action!(
    /// TS queue `recordGame`: `{ internalToken, record, hands? }` → `null`.
    RecordGame,
    "recordGame",
    ()
);
profile_action!(
    /// TS queue `recordHands`: `{ internalToken, matchId, hands }` → `null`.
    RecordHands,
    "recordHands",
    ()
);
profile_action!(
    /// TS queue `recordEpisodes`: `{ internalToken, episodes }` → `null`.
    RecordEpisodes,
    "recordEpisodes",
    ()
);
profile_action!(
    /// TS queue `setActiveGame`: `{ internalToken, gameId }` → `null`.
    SetActiveGame,
    "setActiveGame",
    ()
);
profile_action!(
    /// TS queue `recordTokens`: `{ internalToken, tokensIn, tokensOut,
    /// cacheReadTokens, calls }` → `null`.
    RecordTokens,
    "recordTokens",
    ()
);

impl GetSettings {
    /// `getSettings()`.
    pub fn new() -> Self {
        Self(Vec::new())
    }
}

impl UpdateSettings {
    /// `updateSettings(patch)`.
    pub fn new(patch: Value) -> Self {
        Self(vec![patch])
    }
}

impl GetPersonas {
    /// `getPersonas()`.
    pub fn new() -> Self {
        Self(Vec::new())
    }
}

impl GetActiveGame {
    /// `getActiveGame()`.
    pub fn new() -> Self {
        Self(Vec::new())
    }
}

impl GetStats {
    /// `getStats()`.
    pub fn new() -> Self {
        Self(Vec::new())
    }
}

impl ListGames {
    /// `listGames(req)`; `None` sends no argument.
    pub fn new(req: Option<Value>) -> Self {
        Self(req.into_iter().collect())
    }
}

impl GetReplay {
    /// `getReplay(matchId)`.
    pub fn new(match_id: impl Into<Value>) -> Self {
        Self(vec![match_id.into()])
    }
}

impl ListEpisodes {
    /// `listEpisodes(limit)`; `None` sends no argument.
    pub fn new(limit: Option<Value>) -> Self {
        Self(limit.into_iter().collect())
    }
}

impl ForgetEpisode {
    /// `forgetEpisode(episodeId)`.
    pub fn new(episode_id: impl Into<Value>) -> Self {
        Self(vec![episode_id.into()])
    }
}

impl GetServerReplay {
    /// `getServerReplay(internalToken, matchId)`.
    pub fn new(internal_token: impl Into<Value>, match_id: impl Into<Value>) -> Self {
        Self(vec![internal_token.into(), match_id.into()])
    }
}

impl GetEpisodesForRole {
    /// `getEpisodesForRole(internalToken, role, limit)`; a `None` limit is omitted.
    pub fn new(
        internal_token: impl Into<Value>,
        role: impl Into<Value>,
        limit: Option<Value>,
    ) -> Self {
        let mut args = vec![internal_token.into(), role.into()];
        args.extend(limit);
        Self(args)
    }
}

impl PruneEpisodes {
    /// `pruneEpisodes(internalToken)`.
    pub fn new(internal_token: impl Into<Value>) -> Self {
        Self(vec![internal_token.into()])
    }
}

macro_rules! queue_constructor {
    ($($ty:ident),+) => {
        $(
            impl $ty {
                /// The queue call with `body` as its single argument.
                pub fn new(body: Value) -> Self {
                    Self(vec![body])
                }

                /// The message body (TS `message.body`).
                pub fn body(&self) -> Option<&Value> {
                    self.arg(0)
                }
            }
        )+
    };
}

queue_constructor!(
    RecordGame,
    RecordHands,
    RecordEpisodes,
    SetActiveGame,
    RecordTokens
);

/// Every action name, in the order of [`crate::PlayerProfile`]'s action set.
pub const ACTION_NAMES: [&str; 17] = [
    "getSettings",
    "updateSettings",
    "getPersonas",
    "getActiveGame",
    "getStats",
    "listGames",
    "getReplay",
    "listEpisodes",
    "forgetEpisode",
    "getServerReplay",
    "getEpisodesForRole",
    "pruneEpisodes",
    "recordGame",
    "recordHands",
    "recordEpisodes",
    "setActiveGame",
    "recordTokens",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decode<A: Action>(args: Value) -> A {
        let bytes = rivetkit::action::encode_positional(&args);
        match bytes.and_then(|b| rivetkit::action::decode_positional::<A>(&b)) {
            Ok(a) => a,
            Err(e) => panic!("decode {}: {e:#}", A::NAME),
        }
    }

    #[test]
    fn whole_positional_array_binds() {
        assert_eq!(decode::<GetSettings>(json!([])), GetSettings::new());
        let l: ListGames = decode(json!([null]));
        assert_eq!(l.arg(0), None);
        let l: ListGames = decode(json!([{ "limit": 5, "cursor": "1:m" }]));
        assert_eq!(l.arg(0), Some(&json!({ "limit": 5, "cursor": "1:m" })));
        let g: GetEpisodesForRole = decode(json!(["tok", "marge"]));
        assert_eq!(
            (g.arg(0), g.arg(1), g.arg(2)),
            (Some(&json!("tok")), Some(&json!("marge")), None)
        );
        let r: RecordTokens = decode(json!([{ "internalToken": "t", "calls": 1 }]));
        assert_eq!(r.body(), Some(&json!({ "internalToken": "t", "calls": 1 })));
        // An argument that is itself an array stays one argument.
        let u: UpdateSettings = decode(json!([[1, 2]]));
        assert_eq!(u.arg(0), Some(&json!([1, 2])));
    }

    fn as_json<T: Serialize>(v: &T) -> Value {
        serde_json::to_value(v).unwrap_or_default()
    }

    #[test]
    fn constructors_match_ts_call_shapes() {
        assert_eq!(as_json(&ListGames::new(None)), json!([]));
        assert_eq!(as_json(&GetReplay::new("m1")), json!(["m1"]));
        assert_eq!(
            as_json(&GetEpisodesForRole::new("t", "r", Some(json!(3)))),
            json!(["t", "r", 3])
        );
        assert_eq!(as_json(&PruneEpisodes::new("t")), json!(["t"]));
        assert_eq!(
            as_json(&RecordTokens::new(json!({ "a": 1 }))),
            json!([{ "a": 1 }])
        );
    }
}
