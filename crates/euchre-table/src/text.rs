//! Small pure helpers of `euchre-table/index.ts`: chat screening, the fence
//! nonce, the seed, UTF-16 cuts and the decision kind of a phase.

use euchre_engine::{GamePhase, ObservationKind};
use euchre_protocol::AIDecisionKind;
use euchre_server_core::config::is_js_whitespace;
use rand::RngCore as _;

/// The longest `err` text a TS log line carries (`String(err).slice(0, 200)`).
pub const LOG_ERR_UNITS: usize = 200;

/// The first `max` UTF-16 code units of `s` (JS `.slice(0, max)`).
///
/// D27: a surrogate pair straddling the cut is dropped whole, where JS would
/// leave a lone high surrogate (a Rust `String` cannot hold one).
pub fn utf16_prefix(s: &str, max: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in s.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out
}

/// TS `String(err).slice(0, 200)` for a log field.
pub fn log_excerpt(text: &str) -> String {
    utf16_prefix(text, LOG_ERR_UNITS)
}

/// TS `screenLine(text, max)`: control characters (`< 0x20` and `0x7f`) become
/// spaces, every JS `\s` run collapses to one space, the ends are trimmed and the
/// result is cut to `max` UTF-16 units (never splitting a pair, D27).
pub fn screen_line(text: &str, max: usize) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        let code = u32::from(ch);
        let ch = if code < 0x20 || code == 0x7f { ' ' } else { ch };
        if is_js_whitespace(ch) {
            if !in_space {
                collapsed.push(' ');
                in_space = true;
            }
        } else {
            collapsed.push(ch);
            in_space = false;
        }
    }
    utf16_prefix(collapsed.trim_matches(is_js_whitespace), max)
}

/// TS `fenceNonce(matchId)`: FNV-1a over the UTF-16 units, then
/// `h.toString(16).padStart(8,'0') + ((h * 2654435761) >>> 0).toString(16).padStart(8,'0')`.
///
/// The second half is computed in `f64`, as JS does, so its rounding matches.
pub fn fence_nonce(match_id: &str) -> String {
    let h = euchre_engine::fnv1a(match_id);
    let product = f64::from(h) * 2_654_435_761.0;
    // `>>> 0`: ToUint32 of an integral f64 is its value modulo 2^32.
    let low = product.rem_euclid(4_294_967_296.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let low = low as u32;
    format!("{h:08x}{low:08x}")
}

/// TS `randomHex(bytes)`: `bytes` bytes of CSPRNG output as lowercase hex.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// TS `decisionKindFor(phase)`: which decision the seat to act is asked for.
pub const fn decision_kind_for(phase: GamePhase) -> Option<AIDecisionKind> {
    match phase {
        GamePhase::Cutting => Some(AIDecisionKind::Cut),
        GamePhase::BidRound1 => Some(AIDecisionKind::Bid1),
        GamePhase::DealerDiscard => Some(AIDecisionKind::Discard),
        GamePhase::BidRound2 => Some(AIDecisionKind::Bid2),
        GamePhase::TrickPlay => Some(AIDecisionKind::Play),
        _ => None,
    }
}

/// The observation kind with the same wire name as `kind`.
pub const fn observation_kind(kind: AIDecisionKind) -> ObservationKind {
    match kind {
        AIDecisionKind::Cut => ObservationKind::Cut,
        AIDecisionKind::Bid1 => ObservationKind::Bid1,
        AIDecisionKind::Bid2 => ObservationKind::Bid2,
        AIDecisionKind::Discard => ObservationKind::Discard,
        AIDecisionKind::Play => ObservationKind::Play,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_nonce_matches_ts_golden_vectors() {
        // Computed with the TS `fenceNonce` under bun.
        assert_eq!(fence_nonce("g-1"), "fe81d880da9b3000");
        assert_eq!(
            fence_nonce("6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b"),
            "3729351bb0647a00"
        );
        assert_eq!(fence_nonce(""), "811c9dc546ae3400");
        assert_eq!(fence_nonce("\u{1F0A1}x"), "11bbd1e29c9def80");
    }

    #[test]
    fn screen_line_matches_ts() {
        assert_eq!(
            screen_line("  hi\n\tthere\u{7} friend  ", 140),
            "hi there friend"
        );
        assert_eq!(
            screen_line("a\u{a0}\u{2003}b\u{3000}c\u{feff}d", 140),
            "a b c d"
        );
        // U+0085 is not JS whitespace and is not below 0x20: kept. U+180E is kept.
        assert_eq!(
            screen_line("\u{85}a\u{2028}b\u{180e}c", 140),
            "\u{85}a b\u{180e}c"
        );
        assert_eq!(screen_line("", 140), "");
        assert_eq!(screen_line(" \n\t ", 140), "");
        assert_eq!(screen_line("abcdef", 3), "abc");
        // TS leaves a lone high surrogate here; D27 drops the whole pair.
        let s = format!("{}\u{1F0A1}", "x".repeat(139));
        assert_eq!(screen_line(&s, 140), "x".repeat(139));
        assert_eq!(screen_line("del\u{7f}ete", 140), "del ete");
    }

    #[test]
    fn utf16_prefix_counts_units() {
        assert_eq!(utf16_prefix("abc", 64), "abc");
        assert_eq!(utf16_prefix(&"y".repeat(100), 64).len(), 64);
        assert_eq!(utf16_prefix("\u{1F0A1}\u{1F0A1}", 3), "\u{1F0A1}");
        assert_eq!(log_excerpt(&"z".repeat(300)).len(), 200);
    }

    #[test]
    fn random_hex_is_lowercase_hex_of_the_right_length() {
        let a = random_hex(16);
        assert_eq!(a.len(), 32);
        assert!(
            a.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_ne!(a, random_hex(16));
        assert_eq!(random_hex(8).len(), 16);
    }

    #[test]
    fn decision_kinds_follow_the_phase() {
        assert_eq!(
            decision_kind_for(GamePhase::Cutting),
            Some(AIDecisionKind::Cut)
        );
        assert_eq!(
            decision_kind_for(GamePhase::BidRound1),
            Some(AIDecisionKind::Bid1)
        );
        assert_eq!(
            decision_kind_for(GamePhase::DealerDiscard),
            Some(AIDecisionKind::Discard)
        );
        assert_eq!(
            decision_kind_for(GamePhase::BidRound2),
            Some(AIDecisionKind::Bid2)
        );
        assert_eq!(
            decision_kind_for(GamePhase::TrickPlay),
            Some(AIDecisionKind::Play)
        );
        for p in [
            GamePhase::Lobby,
            GamePhase::Deal,
            GamePhase::TrickResolve,
            GamePhase::HandScore,
            GamePhase::GameOver,
        ] {
            assert_eq!(decision_kind_for(p), None);
        }
        assert_eq!(
            observation_kind(AIDecisionKind::Bid2),
            ObservationKind::Bid2
        );
    }
}
