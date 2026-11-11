//! Pure `playerProfile` logic (lane SERVER-PROFILE, D13/D29): settings and
//! their patch/repair algorithms, text hygiene, persona views and presets,
//! record validation for the queue bodies, the SQLite schema and row codecs,
//! stats, history cursors, replay redaction and episode ranking.
//!
//! Nothing here does I/O. The `playerProfile` actor (`crates/euchre-profile`)
//! and the HTTP lane call these functions and map a [`ProfileError`] to a
//! `UserError` with the same code and message. Types the protocol owns
//! (`PersonaConfig`, `MatchRecord`, `MatchListPage`, ...) are reused, never
//! redefined.

pub mod cursor;
pub mod episodes;
pub mod error;
pub mod guards;
pub mod js;
pub mod presets;
pub mod records;
pub mod replay;
pub mod settings;
pub mod sql;
pub mod state;
pub mod stats;

pub use cursor::*;
pub use episodes::*;
pub use error::*;
pub use guards::{
    SanitizeOptions, is_valid_episode_id, require_client_id, require_episode_id, require_id,
    sanitize_text, sanitize_value,
};
pub use js::JsJson;
pub use presets::*;
pub use records::*;
pub use replay::*;
pub use settings::*;
pub use sql::{Column, Row};
pub use state::*;
pub use stats::*;
