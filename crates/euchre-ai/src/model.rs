//! Model construction and the sampling-parameter allowlist (`ai/model.ts`).
//!
//! [`model_params`] is the only place sampling parameters are decided, and the
//! allowlist fails closed: an unclassified id gets no sampling parameters at
//! all. The provider is built once at a composition root and injected; with no
//! key the factory is `None`, which the ladders degrade through to the
//! heuristic (decisions) and the phrasebook (banter).

use std::fmt;
use std::sync::Arc;

use euchre_protocol::{
    AnthropicModelId, LLM_TEMPERATURE_MAX, LLM_TEMPERATURE_MIN, MODEL_BID, MODEL_PLAY,
    NO_SAMPLING_MODELS,
};
use serde_json::{Map, Value, json};

use crate::jev::{JevClient, JevClientOptions, create_jev_decision_client};
use crate::talk::{AnthropicTalk, OpenRouterTalk, TalkClient};

/// The upstreams this layer can talk to (`ProviderName`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ProviderName {
    /// OpenRouter, the shipped default.
    #[default]
    OpenRouter,
    /// Anthropic direct (text only).
    Anthropic,
}

impl ProviderName {
    /// `'openrouter'` / `'anthropic'`.
    pub const fn as_str(self) -> &'static str {
        match self {
            ProviderName::OpenRouter => "openrouter",
            ProviderName::Anthropic => "anthropic",
        }
    }
}

/// A sampling parameter (`SamplingParam`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SamplingParam {
    /// `temperature`.
    Temperature,
}

impl SamplingParam {
    /// The parameter name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SamplingParam::Temperature => "temperature",
        }
    }
}

const TEMPERATURE: &[SamplingParam] = &[SamplingParam::Temperature];
const NONE: &[SamplingParam] = &[];

/// `SAMPLING_ALLOWLIST`: exact ids we have classified.
pub const SAMPLING_ALLOWLIST: &[(&str, &[SamplingParam])] = &[
    ("claude-haiku-4-5", TEMPERATURE),
    ("claude-opus-5", NONE),
    ("claude-opus-4-8", NONE),
    ("claude-opus-4-7", NONE),
    ("claude-fable-5", NONE),
    ("claude-sonnet-5", NONE),
    ("google/gemini-3.6-flash", TEMPERATURE),
];

/// `FAMILY_RULES`: prefix rules for unlisted ids; first match wins.
pub const FAMILY_RULES: &[(&str, &[SamplingParam])] = &[
    ("claude-haiku-", TEMPERATURE),
    ("google/", TEMPERATURE),
    ("openai/", TEMPERATURE),
    ("meta-llama/", TEMPERATURE),
];

/// `allowedSamplingParams(modelId)`.
pub fn allowed_sampling_params(model_id: &str) -> &'static [SamplingParam] {
    if let Some((_, p)) = SAMPLING_ALLOWLIST.iter().find(|(id, _)| *id == model_id) {
        return p;
    }
    FAMILY_RULES
        .iter()
        .find(|(prefix, _)| model_id.starts_with(prefix))
        .map_or(NONE, |(_, p)| p)
}

/// `isNoSamplingModel(modelId)`.
pub fn is_no_sampling_model(model_id: &str) -> bool {
    allowed_sampling_params(model_id).is_empty()
}

/// `noSamplingListIsConsistent()`.
pub fn no_sampling_list_is_consistent() -> bool {
    NO_SAMPLING_MODELS
        .iter()
        .all(|id| is_no_sampling_model(id.as_str()))
}

/// `ModelParams`: `temperature` is absent (not `undefined`) for no-sampling ids.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelParams {
    /// The clamped temperature, when allowed.
    pub temperature: Option<f64>,
    /// `providerOptions`, keyed by provider name.
    pub provider_options: Map<String, Value>,
}

impl ModelParams {
    /// The TS object shape: `{ temperature?, providerOptions }`.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        if let Some(t) = self.temperature {
            m.insert("temperature".to_owned(), crate::js::num(t));
        }
        m.insert(
            "providerOptions".to_owned(),
            Value::Object(self.provider_options.clone()),
        );
        Value::Object(m)
    }
}

/// `modelParams(modelId, personaTemperature, provider = 'openrouter')`.
pub fn model_params(model_id: &str, temperature: f64, provider: ProviderName) -> ModelParams {
    let no_sampling = !allowed_sampling_params(model_id).contains(&SamplingParam::Temperature);
    let options = match provider {
        ProviderName::OpenRouter => json!({ "openrouter": { "reasoning": { "effort": "low" } } }),
        ProviderName::Anthropic => {
            if no_sampling {
                json!({ "anthropic": { "thinking": { "type": "disabled" }, "effort": "low" } })
            } else {
                json!({ "anthropic": { "thinking": { "type": "disabled" } } })
            }
        }
    };
    let provider_options = match options {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    ModelParams {
        temperature: (!no_sampling).then(|| clamp_temperature(temperature)),
        provider_options,
    }
}

/// `clampTemperature(t)`: non-finite → the minimum; else clamp to the band.
pub fn clamp_temperature(t: f64) -> f64 {
    if !t.is_finite() {
        return LLM_TEMPERATURE_MIN;
    }
    LLM_TEMPERATURE_MAX.min(LLM_TEMPERATURE_MIN.max(t))
}

/// `DEFAULT_OPENROUTER_MODEL`: conversation only; Jev decisions are pinned.
pub const DEFAULT_OPENROUTER_MODEL: &str = "google/gemini-3.6-flash";

/// `DEFAULT_MODELS`.
pub const DEFAULT_MODELS: (AnthropicModelId, AnthropicModelId) = (MODEL_PLAY, MODEL_BID);

/// How a factory maps a canonical id to an upstream slug.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlugMap {
    /// The id itself (`slugFor: (m) => m`).
    Identity,
    /// OpenRouter slugs: `(haiku, opus)`.
    OpenRouter {
        /// `claude-haiku-4-5`'s slug.
        haiku: String,
        /// `claude-opus-5`'s slug.
        opus: String,
    },
}

/// `ModelFactory`: built once, injected everywhere.
#[derive(Clone)]
pub struct ModelFactory {
    /// Which upstream `providerOptions` must be keyed for.
    pub provider: ProviderName,
    /// Jev structured decisions; text-only providers leave this `None`.
    pub decision: Option<Arc<dyn JevClient>>,
    /// The conversational model.
    pub talk: Arc<dyn TalkClient>,
    /// The slug map.
    pub slugs: SlugMap,
}

impl fmt::Debug for ModelFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelFactory")
            .field("provider", &self.provider)
            .field("decision", &self.decision.is_some())
            .field("slugs", &self.slugs)
            .finish_non_exhaustive()
    }
}

impl ModelFactory {
    /// `slugFor(modelId)`.
    pub fn slug_for(&self, model_id: AnthropicModelId) -> String {
        match &self.slugs {
            SlugMap::Identity => model_id.as_str().to_owned(),
            SlugMap::OpenRouter { haiku, opus } => match model_id {
                AnthropicModelId::ClaudeHaiku45 => haiku.clone(),
                AnthropicModelId::ClaudeOpus5 => opus.clone(),
            },
        }
    }
}

/// Options of [`create_anthropic_model_factory`].
#[derive(Clone, Default)]
pub struct AnthropicFactoryOptions {
    /// Absent or blank means no provider.
    pub api_key: Option<String>,
    /// `ANTHROPIC_BASE_URL`.
    pub base_url: Option<String>,
    /// An injected HTTP client.
    pub http: Option<reqwest::Client>,
}

impl fmt::Debug for AnthropicFactoryOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicFactoryOptions")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

fn trimmed_key(key: Option<&str>) -> Option<String> {
    let k = crate::js::js_trim(key?);
    (!k.is_empty()).then(|| k.to_owned())
}

/// `createAnthropicModelFactory(opts)`: text only, identity slugs; `None`
/// without a key.
pub fn create_anthropic_model_factory(opts: AnthropicFactoryOptions) -> Option<ModelFactory> {
    let api_key = trimmed_key(opts.api_key.as_deref())?;
    Some(ModelFactory {
        provider: ProviderName::Anthropic,
        decision: None,
        talk: Arc::new(AnthropicTalk::new(api_key, opts.base_url, opts.http)),
        slugs: SlugMap::Identity,
    })
}

/// Options of [`create_openrouter_model_factory`].
#[derive(Clone, Default)]
pub struct OpenRouterFactoryOptions {
    /// Absent or blank means no provider.
    pub api_key: Option<String>,
    /// `OPENROUTER_BASE_URL`.
    pub base_url: Option<String>,
    /// Override of `claude-haiku-4-5`'s slug.
    pub haiku_slug: Option<String>,
    /// Override of `claude-opus-5`'s slug.
    pub opus_slug: Option<String>,
    /// An injected HTTP client.
    pub http: Option<reqwest::Client>,
}

impl fmt::Debug for OpenRouterFactoryOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenRouterFactoryOptions")
            .field("base_url", &self.base_url)
            .field("haiku_slug", &self.haiku_slug)
            .field("opus_slug", &self.opus_slug)
            .finish_non_exhaustive()
    }
}

/// `createOpenRouterModelFactory(opts)`: talk plus the Jev decision client;
/// `None` without a key.
pub fn create_openrouter_model_factory(opts: OpenRouterFactoryOptions) -> Option<ModelFactory> {
    let api_key = trimmed_key(opts.api_key.as_deref())?;
    let decision = create_jev_decision_client(JevClientOptions {
        api_key: api_key.clone(),
        base_url: opts.base_url.clone(),
        http: opts.http.clone(),
    });
    Some(ModelFactory {
        provider: ProviderName::OpenRouter,
        decision: Some(Arc::new(decision)),
        talk: Arc::new(OpenRouterTalk::new(api_key, opts.base_url, opts.http)),
        slugs: SlugMap::OpenRouter {
            haiku: opts
                .haiku_slug
                .unwrap_or_else(|| DEFAULT_OPENROUTER_MODEL.to_owned()),
            opus: opts
                .opus_slug
                .unwrap_or_else(|| DEFAULT_OPENROUTER_MODEL.to_owned()),
        },
    })
}

/// `modelFactoryFromEnv(env)` with an explicit lookup and an optional shared
/// HTTP client: OpenRouter first (`OPENROUTER_API_KEY`, `OPENROUTER_BASE_URL`,
/// `OPENROUTER_MODEL_TALK` for both slugs, even when empty), then Anthropic
/// (`ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL`). The legacy
/// `OPENROUTER_MODEL_PLAY` is ignored.
pub fn model_factory_from_env_with(
    env: &dyn Fn(&str) -> Option<String>,
    http: Option<reqwest::Client>,
) -> Option<ModelFactory> {
    let talk = env("OPENROUTER_MODEL_TALK");
    let via_openrouter = create_openrouter_model_factory(OpenRouterFactoryOptions {
        api_key: env("OPENROUTER_API_KEY"),
        base_url: env("OPENROUTER_BASE_URL"),
        haiku_slug: talk.clone(),
        opus_slug: talk,
        http: http.clone(),
    });
    if via_openrouter.is_some() {
        return via_openrouter;
    }
    create_anthropic_model_factory(AnthropicFactoryOptions {
        api_key: env("ANTHROPIC_API_KEY"),
        base_url: env("ANTHROPIC_BASE_URL"),
        http,
    })
}

/// [`model_factory_from_env_with`] over an explicit environment record.
pub fn model_factory_from_env(env: &[(&str, &str)]) -> Option<ModelFactory> {
    let lookup = |k: &str| {
        env.iter()
            .rev()
            .find(|(name, _)| *name == k)
            .map(|(_, v)| (*v).to_owned())
    };
    model_factory_from_env_with(&lookup, None)
}

/// [`model_factory_from_env_with`] over the process environment. Call once, at
/// a composition root.
pub fn model_factory_from_process_env() -> Option<ModelFactory> {
    model_factory_from_env_with(&|k| std::env::var(k).ok(), None)
}

/// `createStaticModelFactory(model, provider)`: one talk model for every tier,
/// identity slugs, no decision capability unless one is attached.
pub fn create_static_model_factory(
    talk: Arc<dyn TalkClient>,
    provider: ProviderName,
) -> ModelFactory {
    ModelFactory {
        provider,
        decision: None,
        talk,
        slugs: SlugMap::Identity,
    }
}
