/**
 * Constants the player-profile repository needs without importing the whole
 * protocol/engine surface. Kept as a thin re-export layer so repository SQL
 * stays free of deep `$lib/euchre/*` paths.
 */

export { EPISODE_SUMMARY_MAX_CHARS, PERSONA_NAME_MAX_CHARS } from '#lib/protocol/index.ts';

export { DEFAULT_ENGINE_CONFIG } from '#lib/euchre/index.ts';
