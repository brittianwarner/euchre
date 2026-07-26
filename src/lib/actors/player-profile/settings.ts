/**
 * Settings: defaults, validation, clamping, and the state clone.
 *
 * Settings are small, bounded and read in full on every wake, which is exactly
 * the shape `c.state` is for: three personas, one house prompt, five preference
 * flags. Nothing here grows with use — history does, and history lives in
 * `c.db`.
 *
 * **The persona prompts are untrusted input and are treated as data at every
 * step.** They are sanitised and clamped on the way in ({@link sanitizeText}),
 * stored as parameters in a serialised blob, returned verbatim, and never once
 * interpolated into a query, a template, a log line or anything evaluated. The
 * caps applied are the ones declared in `$lib/protocol`, imported rather than
 * copied so a change there cannot silently fail to reach this actor.
 *
 * The one thing a caller may *not* set is a persona's `id` or `version`. Ids are
 * server-assigned (they double as the episode `role`, which is a database key),
 * and versions are bumped by the server on every accepted save so a `MatchRecord`
 * can name the exact persona it played against.
 */

import {
	DIAL_MAX,
	DIAL_MIN,
	DIFFICULTIES,
	HOUSE_PROMPT_MAX_CHARS,
	MODEL_BID,
	MODEL_PLAY,
	PERSONA_BLURB_MAX_CHARS,
	PERSONA_NAME_MAX_CHARS,
	PERSONA_PROMPT_MAX_CHARS,
	type AnthropicModelId,
	type Difficulty,
	type PersonaConfig,
	type PersonaView,
	type Seat
} from '$lib/protocol';
import {
	clampNumber,
	isRecord,
	protocolError,
	requireBoolean,
	requireOneOf,
	sanitizeText
} from './guards';
import { AI_SEATS, type AiSeat, type ProfileSettings, type TablePrefs } from './types';

/**
 * A writable `PersonaConfig`. The contract's version is `readonly` throughout
 * because it crosses the wire; the copy being edited here has not crossed
 * anything yet.
 */
type MutablePersona = { -readonly [K in keyof PersonaConfig]: PersonaConfig[K] };

/** The two model ids this application uses, as a runtime set. */
export const ANTHROPIC_MODEL_IDS = [MODEL_PLAY, MODEL_BID] as const satisfies readonly AnthropicModelId[];

/** Pacing options, as a runtime set. */
const PACES = ['brisk', 'normal', 'relaxed'] as const satisfies readonly TablePrefs['pace'][];

/** At most one patch entry per AI seat, so an `updateSettings` call is bounded. */
const MAX_PERSONA_PATCHES = AI_SEATS.length;

/* ========================================================================== */
/* Defaults                                                                   */
/* ========================================================================== */

/**
 * The shared house prompt a new profile starts with.
 *
 * Style only, and deliberately so: it says how the table sounds, never what is
 * legal. Legality is the engine's, and the model never gets a say in it.
 */
const DEFAULT_HOUSE_PROMPT =
	'This is a friendly Thursday-night euchre game in somebody’s kitchen. ' +
	'Keep the table talk short, dry and good-natured. No coaching, no gloating that lasts.';

interface PersonaSeed {
	readonly id: string;
	readonly name: string;
	readonly blurb: string;
	readonly prompt: string;
	readonly difficulty: Difficulty;
	readonly aggression: number;
	readonly risk: number;
	readonly chattiness: number;
	readonly temperature: number;
}

/**
 * The three seats a new profile is dealt, index `i` binding `AI_SEATS[i]`.
 *
 * Seat 2 is the human's partner (teams are `seat & 1`), so it is written as the
 * steady one; seats 1 and 3 are the opposition.
 */
const DEFAULT_PERSONAS: readonly [PersonaSeed, PersonaSeed, PersonaSeed] = [
	{
		id: 'marge',
		name: 'Marge',
		blurb: 'Counts every card and lets you know it.',
		prompt:
			'You have played in this kitchen for thirty years. You are precise, patient, and ' +
			'faintly amused by anybody who bids on three small trump. You speak in short sentences.',
		difficulty: 'expert',
		aggression: 0.35,
		risk: 0.25,
		chattiness: 0.4,
		temperature: 0.5
	},
	{
		id: 'dutch',
		name: 'Dutch',
		blurb: 'Your partner. Steady, and slightly deaf.',
		prompt:
			'You are the player’s partner. You are steady, encouraging and never second-guess ' +
			'a call out loud. When you are pleased you say so in four words or fewer.',
		difficulty: 'casual',
		aggression: 0.5,
		risk: 0.45,
		chattiness: 0.5,
		temperature: 0.6
	},
	{
		id: 'verna',
		name: 'Verna',
		blurb: 'Orders it up on anything red.',
		prompt:
			'You bid too often and enjoy it. You are cheerful about being euchred and insufferable ' +
			'about a march. You never explain your reasoning.',
		difficulty: 'casual',
		aggression: 0.8,
		risk: 0.75,
		chattiness: 0.75,
		temperature: 0.7
	}
];

const DEFAULT_TABLE_PREFS: TablePrefs = {
	pace: 'normal',
	banter: true,
	showRationale: true,
	sound: true,
	reduceMotion: false
};

function seedToPersona(seed: PersonaSeed, housePrompt: string): PersonaConfig {
	return {
		id: seed.id,
		version: 1,
		name: seed.name,
		blurb: seed.blurb,
		prompt: seed.prompt,
		housePrompt,
		difficulty: seed.difficulty,
		aggression: seed.aggression,
		risk: seed.risk,
		chattiness: seed.chattiness,
		temperature: seed.temperature,
		modelId: MODEL_PLAY,
		bidModelId: MODEL_BID
	};
}

/** A brand-new profile's settings. */
export function defaultSettings(now: number): ProfileSettings {
	return {
		schema: 1,
		version: 1,
		updatedAt: now,
		housePrompt: DEFAULT_HOUSE_PROMPT,
		personas: DEFAULT_PERSONAS.map((seed) => seedToPersona(seed, DEFAULT_HOUSE_PROMPT)),
		table: { ...DEFAULT_TABLE_PREFS }
	};
}

/* ========================================================================== */
/* Cloning                                                                    */
/* ========================================================================== */

/**
 * A plain deep copy of settings.
 *
 * Explicit rather than `structuredClone` because `c.state` is a write-through
 * proxy: copying it field by field guarantees what leaves this actor is inert
 * data with no live binding back into state, and it fails to compile — rather
 * than silently omitting a field — when the shape changes.
 */
export function cloneSettings(settings: ProfileSettings): ProfileSettings {
	return {
		schema: settings.schema,
		version: settings.version,
		updatedAt: settings.updatedAt,
		housePrompt: settings.housePrompt,
		personas: settings.personas.map((persona) => clonePersona(persona)),
		table: { ...settings.table }
	};
}

export function clonePersona(persona: PersonaConfig): PersonaConfig {
	return {
		id: persona.id,
		version: persona.version,
		name: persona.name,
		blurb: persona.blurb,
		prompt: persona.prompt,
		housePrompt: persona.housePrompt,
		difficulty: persona.difficulty,
		aggression: persona.aggression,
		risk: persona.risk,
		chattiness: persona.chattiness,
		temperature: persona.temperature,
		modelId: persona.modelId,
		bidModelId: persona.bidModelId
	};
}

/**
 * The client-safe face of a persona.
 *
 * Used by any surface that wants to *name* an opponent without handling its
 * prompt: `degraded` is not a profile concept (it belongs to a live seat's token
 * meter) and is reported `false` here.
 */
export function personaView(persona: PersonaConfig, seat: Seat): PersonaView {
	return {
		id: persona.id,
		version: persona.version,
		name: persona.name,
		blurb: persona.blurb,
		seat,
		difficulty: persona.difficulty,
		chattiness: persona.chattiness,
		degraded: false
	};
}

/* ========================================================================== */
/* Patch validation                                                           */
/* ========================================================================== */

function seatIndex(rawSeat: unknown): number {
	const index = AI_SEATS.findIndex((seat) => seat === rawSeat);
	if (index < 0) {
		throw protocolError('invalid_persona', `seat must be one of: ${AI_SEATS.join(', ')}`);
	}
	return index;
}

function applyPersonaPatch(current: PersonaConfig, rawPatch: unknown, seat: AiSeat): PersonaConfig {
	if (!isRecord(rawPatch)) {
		throw protocolError('invalid_persona', `patch for seat ${seat} must be an object`);
	}
	const next: MutablePersona = { ...clonePersona(current) };
	const field = (name: string): string => `personas[${seat}].${name}`;

	let changed = false;
	const set = <K extends keyof MutablePersona>(key: K, value: MutablePersona[K]): void => {
		if (next[key] !== value) changed = true;
		next[key] = value;
	};

	if ('name' in rawPatch) {
		const name = sanitizeText(rawPatch.name, {
			multiline: false,
			maxChars: PERSONA_NAME_MAX_CHARS,
			field: field('name')
		});
		if (name === '') {
			throw protocolError('invalid_persona', `${field('name')} is empty after sanitisation`);
		}
		set('name', name);
	}

	if ('blurb' in rawPatch) {
		set(
			'blurb',
			sanitizeText(rawPatch.blurb, {
				multiline: false,
				maxChars: PERSONA_BLURB_MAX_CHARS,
				field: field('blurb')
			})
		);
	}

	if ('prompt' in rawPatch) {
		// An empty prompt is legitimate — it means "no character, just play".
		set(
			'prompt',
			sanitizeText(rawPatch.prompt, {
				multiline: true,
				maxChars: PERSONA_PROMPT_MAX_CHARS,
				field: field('prompt')
			})
		);
	}

	if ('difficulty' in rawPatch) {
		set(
			'difficulty',
			requireOneOf(rawPatch.difficulty, DIFFICULTIES, field('difficulty'), 'invalid_persona')
		);
	}

	for (const dial of ['aggression', 'risk', 'chattiness', 'temperature'] as const) {
		if (dial in rawPatch) {
			set(dial, clampNumber(rawPatch[dial], DIAL_MIN, DIAL_MAX, field(dial)));
		}
	}

	for (const model of ['modelId', 'bidModelId'] as const) {
		if (model in rawPatch) {
			set(
				model,
				requireOneOf(rawPatch[model], ANTHROPIC_MODEL_IDS, field(model), 'invalid_persona')
			);
		}
	}

	if (changed) next.version = current.version + 1;
	return next;
}

function applyTablePatch(current: TablePrefs, rawPatch: unknown): TablePrefs {
	if (!isRecord(rawPatch)) {
		throw protocolError('invalid_persona', 'table must be an object');
	}
	const next: TablePrefs = { ...current };
	if ('pace' in rawPatch) {
		next.pace = requireOneOf(rawPatch.pace, PACES, 'table.pace', 'invalid_persona');
	}
	for (const flag of ['banter', 'showRationale', 'sound', 'reduceMotion'] as const) {
		if (flag in rawPatch) {
			next[flag] = requireBoolean(rawPatch[flag], `table.${flag}`);
		}
	}
	return next;
}

/**
 * Validates, clamps and applies a patch, returning a **new** settings object.
 *
 * Pure: the caller decides whether to commit it to `c.state`, so a patch that
 * throws half way through cannot leave settings partly written.
 */
export function applySettingsPatch(
	current: ProfileSettings,
	rawPatch: unknown,
	now: number
): ProfileSettings {
	if (!isRecord(rawPatch)) {
		throw protocolError('invalid_persona', 'settings patch must be an object');
	}

	const next = cloneSettings(current);

	if ('housePrompt' in rawPatch) {
		next.housePrompt = sanitizeText(rawPatch.housePrompt, {
			multiline: true,
			maxChars: HOUSE_PROMPT_MAX_CHARS,
			field: 'housePrompt'
		});
	}

	if ('table' in rawPatch) {
		next.table = applyTablePatch(next.table, rawPatch.table);
	}

	if ('personas' in rawPatch) {
		const rawPersonas = rawPatch.personas;
		if (!Array.isArray(rawPersonas)) {
			throw protocolError('invalid_persona', 'personas must be an array');
		}
		if (rawPersonas.length > MAX_PERSONA_PATCHES) {
			throw protocolError(
				'invalid_persona',
				`at most ${MAX_PERSONA_PATCHES} persona patches per call`
			);
		}
		const seen = new Set<number>();
		for (const entry of rawPersonas) {
			if (!isRecord(entry)) {
				throw protocolError('invalid_persona', 'each persona patch must be an object');
			}
			const index = seatIndex(entry.seat);
			if (seen.has(index)) {
				throw protocolError('invalid_persona', 'duplicate seat in persona patches');
			}
			seen.add(index);
			next.personas[index] = applyPersonaPatch(next.personas[index], entry.patch, AI_SEATS[index]);
		}
	}

	// The house prompt is authored once and mirrored onto every persona, because
	// `PersonaConfig` carries its own copy and the snapshot handed to a match must
	// be self-contained. One source of truth, three copies, written together.
	next.personas = next.personas.map((persona) =>
		persona.housePrompt === next.housePrompt
			? persona
			: { ...clonePersona(persona), housePrompt: next.housePrompt, version: persona.version + 1 }
	);

	next.version = current.version + 1;
	next.updatedAt = now;
	return next;
}

/**
 * Repairs settings loaded from an older `c.state`.
 *
 * `c.state` is deserialised whole on every wake, including state written by a
 * previous deployment. Rather than a migration table for a bounded object, every
 * field is re-derived through the same validators a caller's patch would face:
 * anything missing takes its default, anything out of range is clamped, anything
 * over-long is cut. Idempotent, and safe to run on already-valid settings.
 */
export function normalizeSettings(raw: unknown, now: number): ProfileSettings {
	const base = defaultSettings(now);
	if (!isRecord(raw)) return base;

	const housePrompt = safeText(raw.housePrompt, HOUSE_PROMPT_MAX_CHARS, base.housePrompt, true);
	const rawPersonas = Array.isArray(raw.personas) ? raw.personas : [];

	const personas = base.personas.map((fallback, index) => {
		const candidate = rawPersonas[index];
		if (!isRecord(candidate)) return { ...fallback, housePrompt };
		return {
			id: safeId(candidate.id, fallback.id),
			version: safeInt(candidate.version, 1),
			name: safeText(candidate.name, PERSONA_NAME_MAX_CHARS, fallback.name, false),
			blurb: safeText(candidate.blurb, PERSONA_BLURB_MAX_CHARS, fallback.blurb, false),
			prompt: safeText(candidate.prompt, PERSONA_PROMPT_MAX_CHARS, fallback.prompt, true),
			housePrompt,
			difficulty: safeEnum(candidate.difficulty, DIFFICULTIES, fallback.difficulty),
			aggression: safeDial(candidate.aggression, fallback.aggression),
			risk: safeDial(candidate.risk, fallback.risk),
			chattiness: safeDial(candidate.chattiness, fallback.chattiness),
			temperature: safeDial(candidate.temperature, fallback.temperature),
			modelId: safeEnum(candidate.modelId, ANTHROPIC_MODEL_IDS, fallback.modelId),
			bidModelId: safeEnum(candidate.bidModelId, ANTHROPIC_MODEL_IDS, fallback.bidModelId)
		} satisfies PersonaConfig;
	});

	const rawTable = isRecord(raw.table) ? raw.table : {};
	const table: TablePrefs = {
		pace: safeEnum(rawTable.pace, PACES, base.table.pace),
		banter: typeof rawTable.banter === 'boolean' ? rawTable.banter : base.table.banter,
		showRationale:
			typeof rawTable.showRationale === 'boolean'
				? rawTable.showRationale
				: base.table.showRationale,
		sound: typeof rawTable.sound === 'boolean' ? rawTable.sound : base.table.sound,
		reduceMotion:
			typeof rawTable.reduceMotion === 'boolean' ? rawTable.reduceMotion : base.table.reduceMotion
	};

	return {
		schema: 1,
		version: safeInt(raw.version, 1),
		updatedAt: safeInt(raw.updatedAt, now),
		housePrompt,
		personas,
		table
	};
}

/* -------------------------------------------------------------------------- */
/* Repair helpers — total, never throwing: a bad stored value is not a caller  */
/* error, and refusing to wake a profile over one would be the worse outcome.  */
/* -------------------------------------------------------------------------- */

function safeText(value: unknown, maxChars: number, fallback: string, multiline: boolean): string {
	if (typeof value !== 'string') return fallback;
	try {
		return sanitizeText(value, { multiline, maxChars, field: 'stored' });
	} catch {
		return fallback;
	}
}

function safeId(value: unknown, fallback: string): string {
	return typeof value === 'string' && /^[a-z0-9_-]{1,48}$/.test(value) ? value : fallback;
}

function safeInt(value: unknown, fallback: number): number {
	return typeof value === 'number' && Number.isFinite(value) ? Math.trunc(value) : fallback;
}

function safeDial(value: unknown, fallback: number): number {
	if (typeof value !== 'number' || !Number.isFinite(value)) return fallback;
	return Math.min(DIAL_MAX, Math.max(DIAL_MIN, value));
}

function safeEnum<const T extends readonly string[]>(
	value: unknown,
	allowed: T,
	fallback: T[number]
): T[number] {
	for (const candidate of allowed) {
		if (candidate === value) return candidate;
	}
	return fallback;
}
