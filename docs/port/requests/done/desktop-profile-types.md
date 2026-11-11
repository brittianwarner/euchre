# Request: profile and settings types for the desktop (from lane DESKTOP)

**Need.** `euchre_protocol::profile` is reserved but empty. The desktop `Backend` trait
(`apps/desktop/src/net/mod.rs`) therefore carries `match_detail`, `get_settings` and
`patch_settings` as `serde_json::Value`.

**Ask.** Publish typed `MatchDetail`, `ProfileSettings` (house prompt, table prefs, three
personas) and `SettingsPatch` (06 §17.2, §18) in `euchre-protocol`, with the field limits
(`PERSONA_NAME_MAX_CHARS`, `PERSONA_PROMPT_MAX_CHARS`, `HOUSE_PROMPT_MAX_CHARS`).

**Local workaround.** `Value` at the trait boundary. The Games, Match and Settings screens show
only verbatim headings, the match row data and the device preferences until the types land.
Swapping `Value` for the types touches `net/mod.rs`, `net/bridge.rs` (`Reply::MatchDetail`,
`Reply::Settings`) and `screens/other.rs` only.

## Resolution (orchestrator, after W2)

D29 put the profile types in `crates/euchre-profile-model`, not `euchre-protocol`. D28 is amended: the desktop may depend on `euchre-profile-model` (and `euchre-ai`). Swap the `Value`s for `ProfileSettings`, `SettingsPatch` and the match-detail types in the next desktop wave.
