//! The pure rungs of the `aiSeat` decision ladder (`ai-seat/decide.ts`): the
//! ranking it trusts, the heuristic pick, the per-attempt time budget, and the
//! outcome shape, and [`run_ladder`], which drives the model attempts through
//! the bridge.

use euchre_engine::{AiDecisionSource, LegalMove, LegalMoveId};
use euchre_protocol::{AIDecideRequest, AIDecisionKind, PersonaConfig, RankedMove};
use serde_json::{Map, Value, json};

use crate::ai_seat::bridge::{LlmDecideInput, LlmDecider, call_decider};
use crate::ai_seat::difficulty::{NarrowParams, narrow_by_difficulty, turn_rng};
use crate::config::{
    DEADLINE_RESERVE_MS, LLM_ESCALATION_MS, LLM_FIRST_ATTEMPT_MS, LLM_MIN_ATTEMPT_MS,
};
use crate::heuristic::{HeuristicOptions, RankParams, rank_moves};
use crate::js::num;
use crate::screen::seat::{SeatRationaleParams, clean_rationale, publicly_named_cards};
use crate::types::{Clock, LlmUsage, SeatLog, ZERO_LLM_USAGE, seat_events};

/// One answered decision (`LadderOutcome`).
#[derive(Clone, Debug, PartialEq)]
pub struct LadderOutcome {
    /// The chosen move.
    pub move_id: LegalMoveId,
    /// Where it came from.
    pub source: AiDecisionSource,
    /// The screened rationale.
    pub rationale: String,
    /// The ranking used.
    pub ranking: Vec<RankedMove>,
    /// Candidate-set size.
    pub candidates: usize,
    /// Model calls made.
    pub llm_calls: u64,
    /// Rejected attempts.
    pub illegal_attempts: u64,
    /// The heuristic produced nothing and the first legal move was taken.
    pub first_legal_rescue: bool,
    /// Token usage.
    pub usage: LlmUsage,
}

/// `rankingFor(req, persona)`: the supplied ranking when it covers exactly the
/// legal set, otherwise the heuristic with the persona's dials.
pub fn ranking_for(req: &AIDecideRequest, persona: &PersonaConfig) -> Vec<RankedMove> {
    if let Some(supplied) = req.ranking.as_ref()
        && supplied.len() == req.legal.len()
        && supplied
            .iter()
            .all(|r| req.legal.iter().any(|m| m.id == r.id))
    {
        return supplied.clone();
    }
    rank_moves(
        RankParams::new(&req.view)
            .with_hand(&req.view.hand)
            .with_legal(&req.legal)
            .with_opts(HeuristicOptions {
                aggression: Some(persona.aggression),
                risk: Some(persona.risk),
                game_to: None,
            }),
    )
}

/// `heuristicChoice`'s result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeuristicChoice {
    /// The move.
    pub id: LegalMoveId,
    /// Its reason.
    pub why: String,
    /// No ranking entry was legal; `legal[0]` was taken.
    pub rescued: bool,
}

/// `heuristicChoice(ranking, legal)`; `None` only for an empty legal set (TS throws).
pub fn heuristic_choice(ranking: &[RankedMove], legal: &[LegalMove]) -> Option<HeuristicChoice> {
    if let Some(r) = ranking.iter().find(|r| legal.iter().any(|m| m.id == r.id)) {
        return Some(HeuristicChoice {
            id: r.id.clone(),
            why: r.why.clone(),
            rescued: false,
        });
    }
    legal.first().map(|m| HeuristicChoice {
        id: m.id.clone(),
        why: String::new(),
        rescued: true,
    })
}

/// Parameters of [`budget_for`].
#[derive(Clone, Copy, Debug)]
pub struct BudgetParams {
    /// The rung's nominal budget.
    pub nominal: f64,
    /// The request deadline (epoch ms).
    pub deadline_at: f64,
    /// Now (epoch ms).
    pub now: f64,
}

/// `budgetFor(nominal, deadlineAt, now)`: `min(nominal, deadline - now - reserve)`.
pub fn budget_for(params: BudgetParams) -> f64 {
    let remaining = params.deadline_at - params.now - DEADLINE_RESERVE_MS as f64;
    let budget = params.nominal.min(remaining);
    if budget.is_finite() { budget } else { 0.0 }
}

/* ========================================================================== */
/* runLadder                                                                  */
/* ========================================================================== */

/// What [`run_ladder`] needs that is not in the request (`LadderDeps`).
#[derive(Clone, Copy)]
pub struct LadderDeps<'a> {
    /// Snapshotted at match creation; `prompt` and `housePrompt` are untrusted.
    pub persona: &'a PersonaConfig,
    /// Cross-game notes.
    pub dossier: &'a str,
    /// The per-game fence nonce.
    pub nonce: &'a str,
    /// The token circuit breaker has tripped: heuristic only.
    pub degraded: bool,
    /// The injected clock.
    pub clock: &'a (dyn Clock + Send + Sync),
    /// The seat's log sink.
    pub log: &'a (dyn SeatLog + Send + Sync),
    /// `resolveDecider()`: `None` when no provider is configured.
    pub decider: Option<&'a dyn LlmDecider>,
}

impl std::fmt::Debug for LadderDeps<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LadderDeps")
            .field("persona", &self.persona.id)
            .field("degraded", &self.degraded)
            .field("decider", &self.decider.is_some())
            .finish_non_exhaustive()
    }
}

fn seat_fields(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// `runLadder(deps, req)`: forced → degraded → no provider → one attempt (two
/// off the play path), each clamped to `deadlineAt - now - 300` and skipped
/// below 400 ms → the heuristic. `None` only for an empty legal set; otherwise
/// the move is always a member of `req.legal`.
pub async fn run_ladder(deps: LadderDeps<'_>, req: &AIDecideRequest) -> Option<LadderOutcome> {
    let legal = &req.legal;
    if legal.is_empty() {
        return None;
    }
    let ranking = ranking_for(req, deps.persona);
    let fallback_pick = heuristic_choice(&ranking, legal)?;
    let allowed_cards = publicly_named_cards(&req.view);
    let fallback_rationale = || {
        clean_rationale(SeatRationaleParams {
            raw: None,
            fallback: &fallback_pick.why,
            allowed: &allowed_cards,
        })
    };

    if legal.len() == 1 {
        return Some(LadderOutcome {
            move_id: legal[0].id.clone(),
            source: AiDecisionSource::Forced,
            rationale: fallback_rationale(),
            ranking,
            candidates: 1,
            llm_calls: 0,
            illegal_attempts: 0,
            first_legal_rescue: false,
            usage: ZERO_LLM_USAGE,
        });
    }

    let heuristic_outcome = |ranking: Vec<RankedMove>,
                             llm_calls: u64,
                             illegal_attempts: u64,
                             usage: LlmUsage,
                             candidates: usize| LadderOutcome {
        move_id: fallback_pick.id.clone(),
        source: AiDecisionSource::Fallback,
        rationale: fallback_rationale(),
        ranking,
        candidates,
        llm_calls,
        illegal_attempts,
        first_legal_rescue: fallback_pick.rescued,
        usage,
    };

    if deps.degraded {
        return Some(heuristic_outcome(
            ranking,
            0,
            0,
            ZERO_LLM_USAGE,
            legal.len(),
        ));
    }

    let candidates: Vec<LegalMove> = match req.kind {
        AIDecisionKind::Play | AIDecisionKind::Bid1 | AIDecisionKind::Bid2 => legal.clone(),
        _ => {
            let mut rnd = turn_rng(&req.turn_id, req.seat);
            narrow_by_difficulty(NarrowParams {
                ranking: &ranking,
                legal,
                difficulty: deps.persona.difficulty,
                rnd: &mut rnd,
            })
        }
    };

    let seat = json!(req.seat.get());
    let kind = json!(req.kind.as_str());
    let Some(decider) = deps.decider else {
        deps.log.info(
            seat_events::AI_NO_PROVIDER,
            &seat_fields(&[("seat", seat), ("kind", kind)]),
        );
        let n = candidates.len();
        return Some(heuristic_outcome(ranking, 0, 0, ZERO_LLM_USAGE, n));
    };

    let attempts: &[bool] = if req.kind == AIDecisionKind::Play {
        &[false]
    } else {
        &[false, true]
    };
    let mut llm_calls = 0u64;
    let mut illegal_attempts = 0u64;
    let mut usage = ZERO_LLM_USAGE;

    for &escalate in attempts {
        let nominal = if escalate {
            LLM_ESCALATION_MS
        } else {
            LLM_FIRST_ATTEMPT_MS
        };
        let budget_ms = budget_for(BudgetParams {
            nominal: nominal as f64,
            deadline_at: req.deadline_at as f64,
            now: deps.clock.now_ms() as f64,
        });
        if budget_ms < LLM_MIN_ATTEMPT_MS as f64 {
            deps.log.info(
                seat_events::AI_NO_BUDGET,
                &seat_fields(&[
                    ("seat", seat.clone()),
                    ("escalate", json!(escalate)),
                    ("budgetMs", num(budget_ms)),
                ]),
            );
            break;
        }
        let input = LlmDecideInput {
            kind: req.kind,
            seat: req.seat,
            view: req.view.clone(),
            candidates: candidates.clone(),
            ranking: ranking.clone(),
            persona: deps.persona.clone(),
            dossier: deps.dossier.to_owned(),
            nonce: deps.nonce.to_owned(),
            budget_ms,
            escalate,
            deadline_at: req.deadline_at,
        };
        llm_calls += 1;
        let Some(result) = call_decider(decider, &input, deps.log).await else {
            illegal_attempts += 1;
            deps.log.info(
                seat_events::AI_ATTEMPT_FAILED,
                &seat_fields(&[
                    ("seat", seat.clone()),
                    ("kind", kind.clone()),
                    ("escalate", json!(escalate)),
                ]),
            );
            continue;
        };
        usage = result.usage.unwrap_or(ZERO_LLM_USAGE);
        let Some(chosen) = candidates.iter().find(|m| m.id == result.move_id) else {
            illegal_attempts += 1;
            deps.log.info(
                seat_events::AI_ILLEGAL_ID,
                &seat_fields(&[("seat", seat.clone()), ("got", json!(result.move_id))]),
            );
            continue;
        };
        let why = ranking
            .iter()
            .find(|r| r.id == chosen.id)
            .map_or(fallback_pick.why.as_str(), |r| r.why.as_str());
        let rationale = clean_rationale(SeatRationaleParams {
            raw: result.rationale.as_deref(),
            fallback: why,
            allowed: &allowed_cards,
        });
        return Some(LadderOutcome {
            move_id: chosen.id.clone(),
            source: AiDecisionSource::Llm,
            rationale,
            ranking,
            candidates: candidates.len(),
            llm_calls,
            illegal_attempts,
            first_legal_rescue: false,
            usage,
        });
    }

    let n = candidates.len();
    Some(heuristic_outcome(
        ranking,
        llm_calls,
        illegal_attempts,
        usage,
        n,
    ))
}
