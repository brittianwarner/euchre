# Request: switch the two `forgetEpisode` callers to the D30 episode-id guard (from lane S2-CRATES, W3)

**For:** the owner of `crates/euchre-profile` (the actor and `tests/profile_e2e.rs`) and the owner of `crates/euchre-http` (`api/profile.rs` and `api/tests.rs`).

**Status:** the model side is done. Each caller needs a one-line change plus a test update.

## Background

D30 says `forgetEpisode` must accept the ids it stores, which have the form `role.matchId.handNo.kind`. TS checked them with `requireId`, which allows no `.`, so it refused every real id.

`euchre-profile-model` now exports a guard for this. The functions are additive; `require_id`, `is_valid_id` and `ID_PATTERN_TEXT` are unchanged:

```rust
pub use euchre_profile_model::{is_valid_episode_id, require_episode_id}; // also under guards::
pub const EPISODE_ID_PATTERN_TEXT: &str = "<role>.<matchId>.<handNo>.<kind>";
pub fn is_valid_episode_id(value: &str) -> bool;
pub fn require_episode_id(value: Option<&Value>) -> ProfileResult<String>;
// error: internal_error "episodeId must match <role>.<matchId>.<handNo>.<kind>"
```

What the guard accepts:
- Exactly four `.`-separated parts.
- `role` and `matchId` match `/^[A-Za-z0-9_-]{1,128}$/`.
- `handNo` is a canonical decimal in `0..=10000`.
- `kind` is one of `bid`, `play`, `loner`, `euchre`, `read`.

Anything else is refused, including `/`, `%`, whitespace, `..`, leading zeros, extra parts and unknown kinds. The proof is in `crates/euchre-profile-model/src/guards.rs` tests and in `reports/profile-model.md` deviation 10.

Until both callers switch, D30 is not live:
- the actor still refuses every stored id;
- `DELETE /api/episodes/{id}` answers `400` for every stored id before it reaches the actor.

## 1. `crates/euchre-profile/src/actor.rs`

Add `require_episode_id` to the `euchre_profile_model::{...}` import, then change `Handler<ForgetEpisode>`:

```diff
-        let episode_id = require_id(a.0.first(), "episodeId").map_err(profile_error)?;
+        let episode_id = require_episode_id(a.0.first()).map_err(profile_error)?;
```

## 2. `crates/euchre-profile/tests/profile_e2e.rs` (about lines 771-787)

The current block pins the TS bug. Replace it so the test forgets the stored id, then re-records that episode so the later counts (3, 3, 5) still hold:

```rust
    // forgetEpisode (D30): forgets by the stored `role.matchId.handNo.kind` id,
    // which TS refused. Malformed ids are still refused.
    let forget = format!("verna.{m1}.0.bid");
    ensure!(
        stateless_call_with_timeout(&owner, ForgetEpisode::new(forget.clone()), CALL).await?,
        "forgetEpisode stored id"
    );
    let listed = stateless_call_with_timeout(&owner, ListEpisodes::new(None), CALL).await?;
    ensure!(
        listed.len() == 2 && listed.iter().all(|e| e.episode_id != forget),
        "forgotten episode still listed: {listed:?}"
    );
    ensure!(
        !stateless_call_with_timeout(&owner, ForgetEpisode::new(forget), CALL).await?,
        "forgetEpisode twice"
    );
    ensure!(
        !stateless_call_with_timeout(&owner, ForgetEpisode::new(format!("marge.{m1}.9.bid")), CALL)
            .await?,
        "forgetEpisode unknown well-formed id"
    );
    for bad in ["no_such_episode", "verna.m.03.bid", "../m.0.bid", "verna.m.0.bid.x"] {
        expect_error_message(
            stateless_call_with_timeout(&owner, ForgetEpisode::new(bad), CALL).await,
            "internal_error",
            "episodeId must match <role>.<matchId>.<handNo>.<kind>",
            "forgetEpisode malformed id",
        )?;
    }
    // Put it back so the prune checks below see the same three live rows.
    stateless_call_with_timeout(
        &anon,
        RecordEpisodes::new(json!({
            "internalToken": INTERNAL,
            "episodes": [episode("verna", m1, 0, 0.5, None)]
        })),
        CALL,
    )
    .await
    .map_err(call("re-record forgotten episode"))?;
```

The anonymous `ForgetEpisode::new("e1")` check near line 389 does not change: `require_owner` runs first, so it is still `forbidden`.

## 3. `crates/euchre-http/src/api/profile.rs`

Add `is_valid_episode_id` next to `is_valid_id` in the import. The match-id check at about line 267 does not change. In `forget_episode`:

```diff
-    if !is_valid_id(&episode_id) {
+    if !is_valid_episode_id(&episode_id) {
         return Err(ApiError::bad_request(BAD_ID));
     }
```

## 4. `crates/euchre-http/src/api/tests.rs`

- In the identity-route test, the two `DELETE /api/episodes/ep-1` requests (about lines 1142 and 1152) become `DELETE /api/episodes/verna.m1.0.bid`. With the new guard, `ep-1` gets `400` before the actor call, so the bearer case would stop reaching `503 SCRAPBOOK_UNAVAILABLE`.
- In `malformed_ids_are_refused_before_any_call`, add these cases to the existing `has%20space` case:
  - `/api/episodes/ep-1`: an old-style id;
  - `/api/episodes/verna.m1.03.bid`;
  - `/api/episodes/verna.m1.0.nope`.

  Each expects `400 bad_request BAD_ID`.

## Gates

- `cargo fmt -p euchre-profile -p euchre-http --check`
- `cargo clippy -p euchre-profile -p euchre-http --all-targets -- -D warnings`
- `cargo test -p euchre-http`
- `EUCHRE_E2E=1 cargo test -p euchre-profile --test profile_e2e`
- Then tick the D30 line in `W3-checklist.md`.

## Resolution (lane S1-BOOT, 2026-11-11)

Done as asked, in the W3 server review fixes.

- **Actor.** `Handler<ForgetEpisode>` now uses `require_episode_id`.
- **`profile_e2e.rs`.** The TS-bug block is replaced by the block above, word for word. The test forgets `verna.<m1>.0.bid`, checks the listing, forgets it twice, forgets an unknown well-formed id and refuses four malformed ones, then re-records the episode.
- **`euchre-http`.** `forget_episode` checks `is_valid_episode_id`. The match-id check is unchanged.
- **`api/tests.rs`.** Both identity-route requests use `verna.m1.0.bid`. `malformed_ids_are_refused_before_any_call` also refuses `ep-1`, `verna.m1.03.bid` and `verna.m1.0.nope` with `400 bad_request BAD_ID`.

Gates: fmt and clippy are clean for both crates. `cargo test -p euchre-http` passes 134 unit tests. `EUCHRE_E2E=1 cargo test -p euchre-profile --test profile_e2e` passes 3 of 3, twice. The D30 line in `W3-checklist.md` is ticked.

Moved to `done/`.
