/**
 * `$lib/ai` — the LLM decision layer.
 *
 * One job: turn an `AIDecideRequest` into one legal move id, in character, inside
 * a latency budget, without ever being able to produce an illegal move.
 *
 * ```ts
 * import { decide, modelFactoryFromEnv } from '#lib/ai/index.ts';
 *
 * // Once, at a composition root — never inside a decision:
 * const factory = modelFactoryFromEnv();          // null when there is no API key
 *
 * // Per decision, inside the aiSeat workflow's `llm` step:
 * const out = await decide(
 *   { factory, persona, dossier, nonce, log, signal },
 *   req,                                          // AIDecideRequest from the table
 *   candidates,                                   // legal set, narrowed by difficulty
 *   { degraded: state.budget.degraded }
 * );
 * ```
 *
 * What this module guarantees:
 *
 * - **An illegal move is unrepresentable.** The output schema is `z.enum(ids)`
 *   rebuilt per decision from the table-supplied legal set (`schema.ts`).
 * - **It never stalls a game.** Every rung of the ladder is terminal and the last
 *   one needs no network; every model call carries its own `AbortSignal` clamped
 *   against `deadlineAt` (`decide.ts`).
 * - **It never throws for an operational reason.** No API key, a dead provider, a
 *   hung socket, a hostile persona and malformed JSON all land on the heuristic.
 * - **It cannot leak a card.** The only state it sees is `PublicGameView`; banter
 *   sees a narrower slice still; and every line of generated text that reaches a
 *   human passes `screenBanter` (`screen.ts`).
 *
 * What it deliberately does **not** do: rank moves (that is euchre judgement and
 * lives with the AI seat, arriving here as `AIDecideRequest.ranking`), decide
 * timing (table policy — it only clamps against `deadlineAt`), touch `rivetkit`,
 * or read an environment variable anywhere but `modelFactoryFromEnv`.
 */

export {
	DEFAULT_DECISION_BUDGET,
	allowsEscalation,
	isDeliberate,
	resolveBudget,
	tierOf
} from './config';

export {
	NoLegalMovesError,
	callModel,
	decide,
	heuristicOutcome,
	type CallModelOptions,
	type DecideOptions
} from './decide';

export {
	DEFAULT_MODELS,
	allowedSamplingParams,
	clampTemperature,
	createAnthropicModelFactory,
	createStaticModelFactory,
	isNoSamplingModel,
	modelFactoryFromEnv,
	modelParams,
	noSamplingListIsConsistent,
	type AnthropicFactoryOptions
} from './model';

export {
	encodeForLlm,
	estimateTokens,
	publicCards,
	publicStateOnly,
	splitTricks,
	type EncodeOptions
} from './notation';

export {
	L0_BID,
	L0_PLAY,
	MIN_CACHEABLE_PREFIX_TOKENS,
	buildLayers,
	cachePrefixTokens,
	sanitizePersona,
	toInstructions,
	type BuildLayersInput,
	type PromptLayer,
	type PromptLayers,
	type SystemLayerMessage
} from './prompt';

export {
	EmptyCandidateSetError,
	SCHEMA_DESCRIPTION,
	bidSchema,
	decisionSchema,
	discardSchema,
	idsOf,
	moveIdEnum,
	playSchema,
	schemaNameFor,
	type DecisionObject
} from './schema';

export {
	PHRASEBOOK,
	cardsNamedIn,
	cleanRationale,
	phrasebookLine,
	screenBanter,
	screenText,
	type ScreenOptions,
	type ScreenReject,
	type ScreenResult
} from './screen';

export { generateBanter, type BanterOutcome, type BanterRequest } from './banter';

export {
	ZERO_USAGE,
	noopLog,
	type AiLogEvent,
	type AiLogFn,
	type DecideDeps,
	type DecideOutcome,
	type DecideUsage,
	type DecisionBudget,
	type DecisionRequest,
	type ModelFactory,
	type ModelParams,
	type SamplingParam
} from './types';

/**
 * `./testing` is intentionally **not** re-exported here. It imports `ai/test`,
 * which has no place in a production import graph. Tests import it by path:
 * `import { scriptedModel } from '#lib/ai/testing.ts';`
 */
