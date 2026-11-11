//! Presentation timing shared by the server's trick hold and the client's
//! animation (`protocol/pacing.ts`, 02 section 9.1).

use serde::{Deserialize, Serialize};

/// Card-motion durations in ms (`TABLE_MOTION`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableMotion {
    /// A played card's flight.
    pub play: u64,
    /// How long a completed trick stays readable.
    pub read: u64,
    /// Each card's collect animation.
    pub collect: u64,
    /// Delay between successive collected cards.
    pub stagger: u64,
}

/// `TABLE_MOTION`.
pub const TABLE_MOTION: TableMotion = TableMotion {
    play: 700,
    read: 1500,
    collect: 780,
    stagger: 45,
};

/// Settle margin after the last collected card.
const TRICK_SETTLE_MS: u64 = 185;

/// `TRICK_PRESENTATION_MS`: how long the server holds `trick_resolve` (3300 ms).
pub const TRICK_PRESENTATION_MS: u64 = TABLE_MOTION.play
    + TABLE_MOTION.read
    + TABLE_MOTION.collect
    + 3 * TABLE_MOTION.stagger
    + TRICK_SETTLE_MS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trick_presentation_is_3300() {
        assert_eq!(TRICK_PRESENTATION_MS, 3300);
    }
}
