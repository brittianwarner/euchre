//! Table talk (`ai/banter.ts`): a separate generation on public state only.
//!
//! Three gates, in order: salience (`1 - chattiness`, unless forced), budget
//! (one short streamed call with its own deadline), and the screen, which runs
//! on the settled prefix at every delta and on the whole buffer at the end. A
//! rejected line is replaced by a phrasebook line, never regenerated. This
//! function never fails: every failure path resolves to a phrasebook line.

use std::time::Duration;

use euchre_protocol::PublicGameView;
use futures::StreamExt;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::resolve_budget;
use crate::decide::DecideDeps;
use crate::js::num;
use crate::model::model_params;
use crate::notation::public_state_only;
use crate::prompt::{BuildLayersInput, SystemLayerMessage, build_layers, to_instructions};
use crate::screen::ai::{ScreenResult, phrasebook_line, screen_banter};
use crate::talk::{TalkChunk, TalkRequest};
use crate::types::{AiLogEvent, DecideUsage, ZERO_USAGE};
use euchre_protocol::AIDecisionKind;

/// The banter system line, appended after the layers (byte-exact).
pub const BANTER_SYSTEM: &str = "One line of table talk, at most 90 characters. Public events only. Never name a card, never describe your own holdings, never hint at what you hold, never speculate about another player’s cards. No markup. Never advise, encourage, direct, or address a partner about a move. No table coaching or signals.";

/// One banter request (`BanterRequest`).
#[derive(Clone)]
pub struct BanterRequest<'a> {
    /// The redacted view; only its public slice is rendered.
    pub view: &'a PublicGameView,
    /// A phrasebook key and a prompt hint.
    pub situation: &'a str,
    /// `0..1`, compared against `1 - chattiness`.
    pub salience: f64,
    /// Bypass the salience gate.
    pub force: bool,
    /// Deterministic phrasebook index.
    pub seq: i64,
    /// Called with each raw delta that passed the settled-prefix screen.
    pub on_delta: Option<&'a (dyn Fn(&str) + Send + Sync)>,
    /// The caller's cancellation.
    pub cancel: Option<CancellationToken>,
}

impl std::fmt::Debug for BanterRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BanterRequest")
            .field("situation", &self.situation)
            .field("salience", &self.salience)
            .field("force", &self.force)
            .field("seq", &self.seq)
            .finish_non_exhaustive()
    }
}

/// Where a spoken line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BanterSource {
    /// The model's screened line.
    Llm,
    /// A canned line.
    Phrasebook,
}

impl BanterSource {
    /// `'llm'` / `'phrasebook'`.
    pub const fn as_str(self) -> &'static str {
        match self {
            BanterSource::Llm => "llm",
            BanterSource::Phrasebook => "phrasebook",
        }
    }
}

/// `BanterOutcome`.
#[derive(Clone, Debug, PartialEq)]
pub enum BanterOutcome {
    /// `{ spoke: false, reason: 'not_salient' }`.
    NotSalient,
    /// A line to say.
    Spoke {
        /// The line.
        text: String,
        /// Its source.
        source: BanterSource,
        /// Set when a generated line was rejected.
        screened_reason: Option<String>,
        /// Tokens spent (still metered when screened at the end).
        usage: DecideUsage,
    },
}

impl BanterOutcome {
    /// The spoken text, if any.
    pub fn text(&self) -> Option<&str> {
        match self {
            BanterOutcome::NotSalient => None,
            BanterOutcome::Spoke { text, .. } => Some(text),
        }
    }
}

fn clamp01(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn canned(req: &BanterRequest<'_>, reason: Option<&str>, usage: DecideUsage) -> BanterOutcome {
    BanterOutcome::Spoke {
        text: phrasebook_line(req.situation, req.seq).to_owned(),
        source: BanterSource::Phrasebook,
        screened_reason: reason.map(str::to_owned),
        usage,
    }
}

fn fields(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// The full system message list: the layers plus [`BANTER_SYSTEM`].
pub fn banter_instructions(layers: &crate::prompt::PromptLayers) -> Vec<SystemLayerMessage> {
    let mut system = to_instructions(layers);
    system.push(SystemLayerMessage {
        content: BANTER_SYSTEM.to_owned(),
        cache: false,
    });
    system
}

/// The prompt body: `${publicStateOnly(view)}\nSITUATION=${situation}`.
pub fn banter_body(view: &PublicGameView, situation: &str) -> String {
    format!("{}\nSITUATION={situation}", public_state_only(view))
}

enum Ended {
    Done,
    Screened(String),
    Failed(&'static str),
}

/// `generateBanter(deps, req)`.
///
/// Deviation: when the banter deadline (or the caller's cancellation) fires
/// mid-stream, the outcome is the phrasebook line with a `banter_error` log
/// (`e: "TimeoutError"` / `"AbortError"`), never a truncated model line.
pub async fn generate_banter(deps: &DecideDeps<'_>, req: BanterRequest<'_>) -> BanterOutcome {
    let log = deps.log;
    let budget = resolve_budget(deps.budget.as_ref());
    let chattiness = clamp01(deps.persona.chattiness);

    if !req.force && req.salience < 1.0 - chattiness {
        log.log(
            AiLogEvent::BanterSkipped,
            &fields(&[
                ("situation", json!(req.situation)),
                ("salience", num(req.salience)),
            ]),
        );
        return BanterOutcome::NotSalient;
    }

    let Some(factory) = deps.factory else {
        log.log(
            AiLogEvent::NoProvider,
            &fields(&[("where", json!("banter"))]),
        );
        return canned(&req, None, ZERO_USAGE);
    };

    let model_id = deps.persona.model_id;
    let body = banter_body(req.view, req.situation);
    let layers = build_layers(BuildLayersInput {
        kind: AIDecisionKind::Play,
        conversation: true,
        persona: deps.persona,
        dossier: deps.dossier,
        nonce: deps.nonce,
        seat: req.view.you,
        body: &body,
        log: Some(log),
    });
    let slug = factory.slug_for(model_id);
    let request = TalkRequest {
        params: model_params(&slug, deps.persona.temperature, factory.provider),
        model: slug,
        system: banter_instructions(&layers),
        user: layers.user.clone(),
        max_output_tokens: budget.max_output_tokens_banter,
    };

    let abort = req
        .cancel
        .as_ref()
        .map_or_else(CancellationToken::new, CancellationToken::child_token);
    let deadline = tokio::time::sleep(Duration::from_millis(budget.banter_ms));
    tokio::pin!(deadline);

    let mut buffer = String::new();
    let mut usage = ZERO_USAGE;

    let opened = tokio::select! {
        biased;
        () = &mut deadline => Err(Ended::Failed("TimeoutError")),
        r = factory.talk.stream(request, abort.clone()) => Ok(r),
    };
    let ended = match opened {
        Err(e) => e,
        Ok(Err(e)) => {
            // ai-sdk routes provider errors to `onError` and ends the stream.
            log.log(
                AiLogEvent::BanterError,
                &fields(&[("e", json!(e.to_string()))]),
            );
            Ended::Done
        }
        Ok(Ok(mut stream)) => loop {
            let next = tokio::select! {
                biased;
                () = &mut deadline => break Ended::Failed("TimeoutError"),
                n = stream.next() => n,
            };
            match next {
                None => break Ended::Done,
                Some(Ok(TalkChunk::Usage(u))) => usage = u,
                Some(Ok(TalkChunk::Text(delta))) => {
                    buffer.push_str(&delta);
                    // Screen the settled prefix; hold back the trailing partial word.
                    let settled = buffer.rfind(' ').map_or("", |i| &buffer[..=i]);
                    if !settled.is_empty()
                        && let ScreenResult::Rejected { reason, .. } =
                            screen_banter(settled, req.view)
                    {
                        break Ended::Screened(reason.as_str().to_owned());
                    }
                    if let Some(cb) = req.on_delta {
                        cb(&delta);
                    }
                }
                Some(Err(crate::talk::TalkError::Aborted)) => break Ended::Failed("AbortError"),
                Some(Err(e)) => {
                    log.log(
                        AiLogEvent::BanterError,
                        &fields(&[("e", json!(e.to_string()))]),
                    );
                    usage = ZERO_USAGE;
                    break Ended::Done;
                }
            }
        },
    };
    abort.cancel();

    match ended {
        Ended::Failed(name) => {
            log.log(AiLogEvent::BanterError, &fields(&[("e", json!(name))]));
            return canned(&req, None, ZERO_USAGE);
        }
        Ended::Screened(reason) => {
            log.log(
                AiLogEvent::BanterScreened,
                &fields(&[
                    ("persona", json!(deps.persona.id)),
                    ("reason", json!(reason)),
                    ("where", json!("stream")),
                ]),
            );
            return canned(&req, Some(&reason), ZERO_USAGE);
        }
        Ended::Done => {}
    }

    match screen_banter(&buffer, req.view) {
        ScreenResult::Ok(text) => BanterOutcome::Spoke {
            text,
            source: BanterSource::Llm,
            screened_reason: None,
            usage,
        },
        ScreenResult::Rejected { reason, .. } => {
            log.log(
                AiLogEvent::BanterScreened,
                &fields(&[
                    ("persona", json!(deps.persona.id)),
                    ("reason", json!(reason.as_str())),
                    ("where", json!("final")),
                ]),
            );
            canned(&req, Some(reason.as_str()), usage)
        }
    }
}
