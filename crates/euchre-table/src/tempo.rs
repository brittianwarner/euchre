//! Server-held pauses and AI pacing (TS `euchre-table/tempo.ts`, 03 §9).
//!
//! Two profiles: [`Tempo::NORMAL`] and [`Tempo::FAST`] (`EUCHRE_FAST_TEMPO=1`,
//! local runs and tests). Every number is copied from TS.

use euchre_engine::fnv1a;
use euchre_protocol::{AIDecisionKind, TRICK_PRESENTATION_MS};

/// One AI decision's minimum think time, per decision kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThinkFloors {
    /// `cut`.
    pub cut: u64,
    /// `bid1`.
    pub bid1: u64,
    /// `bid2`.
    pub bid2: u64,
    /// `discard`.
    pub discard: u64,
    /// `play`.
    pub play: u64,
}

impl ThinkFloors {
    /// The floor for `kind`.
    pub const fn get(&self, kind: AIDecisionKind) -> u64 {
        match kind {
            AIDecisionKind::Cut => self.cut,
            AIDecisionKind::Bid1 => self.bid1,
            AIDecisionKind::Bid2 => self.bid2,
            AIDecisionKind::Discard => self.discard,
            AIDecisionKind::Play => self.play,
        }
    }
}

/// The TS `TEMPO` object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tempo {
    /// Read pause after a trick (`trick_resolve`).
    pub trick_resolve_ms: u64,
    /// Read pause after a trick that seals a euchre.
    pub trick_resolve_seals_euchre_ms: u64,
    /// Read pause on `hand_score`.
    pub hand_score_ms: u64,
    /// Minimum AI think time per decision kind.
    pub think_floor_ms: ThinkFloors,
    /// Deterministic jitter range added to the floor.
    pub think_jitter_ms: u64,
    /// The AI watchdog: the parked move plays at this deadline.
    pub ai_hard_cap_ms: u64,
    /// When the client may show "still thinking" (unused by the table, D23).
    pub ai_extended_at_ms: u64,
    /// Legacy human nudge (humans now wait forever).
    pub nudge_ms: u64,
    /// Legacy human abandon (humans now wait forever).
    pub abandon_ms: u64,
    /// Retry delay for a failed `recordGame`.
    pub profile_retry_ms: u64,
    /// Self-reap delay after the match is recorded (30 days).
    pub reap_ms: u64,
}

impl Tempo {
    /// Production pacing.
    pub const NORMAL: Tempo = Tempo {
        trick_resolve_ms: TRICK_PRESENTATION_MS,
        trick_resolve_seals_euchre_ms: TRICK_PRESENTATION_MS + 500,
        hand_score_ms: 2500,
        think_floor_ms: ThinkFloors {
            cut: 700,
            bid1: 1900,
            bid2: 1900,
            discard: 1100,
            play: 1000,
        },
        think_jitter_ms: 400,
        ai_hard_cap_ms: 4000,
        ai_extended_at_ms: 2200,
        nudge_ms: 90_000,
        abandon_ms: 240_000,
        profile_retry_ms: 30_000,
        reap_ms: 30 * 86_400_000,
    };

    /// `EUCHRE_FAST_TEMPO=1` pacing.
    pub const FAST: Tempo = Tempo {
        trick_resolve_ms: 120,
        trick_resolve_seals_euchre_ms: 180,
        hand_score_ms: 250,
        think_floor_ms: ThinkFloors {
            cut: 80,
            bid1: 120,
            bid2: 120,
            discard: 120,
            play: 100,
        },
        think_jitter_ms: 40,
        ai_hard_cap_ms: 1500,
        ai_extended_at_ms: 800,
        nudge_ms: 30_000,
        abandon_ms: 90_000,
        profile_retry_ms: 30_000,
        reap_ms: 30 * 86_400_000,
    };

    /// [`Tempo::FAST`] when `fast`, else [`Tempo::NORMAL`].
    pub const fn select(fast: bool) -> Tempo {
        if fast { Tempo::FAST } else { Tempo::NORMAL }
    }

    /// TS `thinkFloorMs(kind, turnId)`: the floor plus `fnv1a(turnId + "#pace") % jitter`.
    pub fn think_floor(&self, kind: AIDecisionKind, turn_id: &str) -> u64 {
        let jitter = u64::from(fnv1a(&format!("{turn_id}#pace"))) % self.think_jitter_ms.max(1);
        self.think_floor_ms.get(kind) + jitter
    }

    /// TS `trickResolveMs(sealsEuchre)`.
    pub const fn trick_resolve(&self, seals_euchre: bool) -> u64 {
        if seals_euchre {
            self.trick_resolve_seals_euchre_ms
        } else {
            self.trick_resolve_ms
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_tempo_ts() {
        let n = Tempo::NORMAL;
        assert_eq!(n.trick_resolve_ms, 3300);
        assert_eq!(n.trick_resolve_seals_euchre_ms, 3800);
        assert_eq!(n.hand_score_ms, 2500);
        assert_eq!(
            (
                n.think_floor_ms.cut,
                n.think_floor_ms.bid1,
                n.think_floor_ms.bid2,
                n.think_floor_ms.discard,
                n.think_floor_ms.play
            ),
            (700, 1900, 1900, 1100, 1000)
        );
        assert_eq!(n.think_jitter_ms, 400);
        assert_eq!(n.ai_hard_cap_ms, 4000);
        assert_eq!(n.ai_extended_at_ms, 2200);
        assert_eq!((n.nudge_ms, n.abandon_ms), (90_000, 240_000));
        assert_eq!(n.profile_retry_ms, 30_000);
        assert_eq!(n.reap_ms, 2_592_000_000);

        let f = Tempo::FAST;
        assert_eq!(
            (
                f.trick_resolve_ms,
                f.trick_resolve_seals_euchre_ms,
                f.hand_score_ms
            ),
            (120, 180, 250)
        );
        assert_eq!(
            (
                f.think_floor_ms.cut,
                f.think_floor_ms.bid1,
                f.think_floor_ms.bid2,
                f.think_floor_ms.discard,
                f.think_floor_ms.play
            ),
            (80, 120, 120, 120, 100)
        );
        assert_eq!((f.think_jitter_ms, f.ai_hard_cap_ms), (40, 1500));
        assert_eq!(f.ai_extended_at_ms, 800);
        assert_eq!((f.nudge_ms, f.abandon_ms), (30_000, 90_000));
        assert_eq!((f.profile_retry_ms, f.reap_ms), (30_000, 2_592_000_000));
        assert_eq!(Tempo::select(true), Tempo::FAST);
        assert_eq!(Tempo::select(false), Tempo::NORMAL);
    }

    #[test]
    fn think_floor_is_floor_plus_fnv_jitter() {
        let n = Tempo::NORMAL;
        for (turn, kind) in [
            ("t-1", AIDecisionKind::Play),
            ("abc", AIDecisionKind::Bid1),
            ("", AIDecisionKind::Cut),
            ("\u{1F0A1}", AIDecisionKind::Discard),
        ] {
            let jitter = u64::from(fnv1a(&format!("{turn}#pace"))) % 400;
            assert_eq!(
                n.think_floor(kind, turn),
                n.think_floor_ms.get(kind) + jitter
            );
            assert!(n.think_floor(kind, turn) < n.think_floor_ms.get(kind) + 400);
        }
        // Golden (TS under bun): fnv1a("#pace") = 664343349; % 400 = 149; % 40 = 29.
        assert_eq!(fnv1a("#pace"), 664_343_349);
        assert_eq!(n.think_floor(AIDecisionKind::Cut, ""), 849);
        assert_eq!(Tempo::FAST.think_floor(AIDecisionKind::Cut, ""), 109);
    }

    #[test]
    fn trick_resolve_picks_the_euchre_beat() {
        assert_eq!(Tempo::NORMAL.trick_resolve(false), 3300);
        assert_eq!(Tempo::NORMAL.trick_resolve(true), 3800);
        assert_eq!(Tempo::FAST.trick_resolve(true), 180);
    }
}
