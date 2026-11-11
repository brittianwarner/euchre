//! `aiSeat/difficulty`: how wide a choice the model is given. Difficulty narrows
//! the candidate set, never legality, latency or token count.

use euchre_engine::{LegalMove, Mulberry32, Seat, fnv1a};
use euchre_protocol::{Difficulty, RankedMove};

/// How many moves `casual` and `rookie` see.
pub const NARROW_TO: usize = 3;
/// How much a `rookie`'s judgement wobbles, in raw score units.
pub const ROOKIE_NOISE: f64 = 1.2;

/// `turnRng(turnId, seat)`: `mulberry32(fnv1a(`${turnId}#${seat}`))`.
pub fn turn_rng(turn_id: &str, seat: Seat) -> Mulberry32 {
    Mulberry32::new(fnv1a(&format!("{turn_id}#{}", seat.get())))
}

/// Parameters of [`narrow_by_difficulty`].
#[derive(Debug)]
pub struct NarrowParams<'a> {
    /// The heuristic ranking.
    pub ranking: &'a [RankedMove],
    /// The legal set.
    pub legal: &'a [LegalMove],
    /// The persona's difficulty.
    pub difficulty: Difficulty,
    /// The turn's noise stream (read only for `rookie`).
    pub rnd: &'a mut Mulberry32,
}

/// `narrowByDifficulty(ranking, legal, difficulty, rnd)`: always a subset of
/// `legal`, non-empty whenever `legal` is, in the caller's own move objects.
pub fn narrow_by_difficulty(params: NarrowParams<'_>) -> Vec<LegalMove> {
    let NarrowParams {
        ranking,
        legal,
        difficulty,
        rnd,
    } = params;
    if legal.is_empty() {
        return Vec::new();
    }
    if difficulty == Difficulty::Expert {
        return legal.to_vec();
    }
    let ordered: Vec<&str> = if difficulty == Difficulty::Casual {
        ranking.iter().map(|r| r.id.as_str()).collect()
    } else {
        let mut noisy: Vec<(&str, f64)> = ranking
            .iter()
            .map(|r| {
                (
                    r.id.as_str(),
                    r.score + (rnd.next_f64() - 0.5) * ROOKIE_NOISE,
                )
            })
            .collect();
        // `(b.s !== a.s ? b.s - a.s : a.id < b.id ? -1 : 1)`, stable.
        noisy.sort_by(|a, b| {
            if a.1 != b.1 {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            } else if a.0 < b.0 {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });
        noisy.into_iter().map(|(id, _)| id).collect()
    };
    let mut out: Vec<LegalMove> = Vec::new();
    for id in ordered {
        // `byId` keeps the last legal move with a given id.
        if let Some(m) = legal.iter().rev().find(|m| m.id == id)
            && !out.iter().any(|o| o.id == m.id)
        {
            out.push(m.clone());
        }
        if out.len() >= NARROW_TO {
            break;
        }
    }
    if out.is_empty() { legal.to_vec() } else { out }
}
