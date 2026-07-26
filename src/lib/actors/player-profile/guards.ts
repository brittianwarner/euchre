/**
 * Runtime guards and text hygiene for `playerProfile`.
 *
 * Every argument that reaches an action arrives as decoded CBOR from somewhere
 * outside this actor — a browser, or another actor. TypeScript's parameter types
 * are erased at the boundary, so nothing in this module trusts a declared type:
 * each value is narrowed here first, and a value that will not narrow becomes a
 * `UserError` rather than a row in the database.
 *
 * The text helpers exist for one reason: persona prompts are free text written by
 * a human and later read by a model. This module does not fence them (that is
 * `aiSeat`'s job, and the fence is where it must be), but it does refuse to store
 * the characters whose only use is to smuggle instructions past a fence —
 * C0/C1 controls, zero-width and bidi-override runs, BOMs — and it clamps every
 * field to the cap the shared contract declares.
 *
 * Nothing here interpolates a caller's string into anything executable. SQL is
 * parameterised without exception; the only strings ever concatenated into a
 * statement are column names, which are literals in this repository's own source.
 */

import { UserError } from 'rivetkit';
import type { ProtocolErrorCode } from '$lib/protocol';

/* ========================================================================== */
/* Errors                                                                     */
/* ========================================================================== */

/** A `UserError` carrying one of the shared {@link ProtocolErrorCode}s. */
export function protocolError(
	code: ProtocolErrorCode,
	message: string,
	metadata?: Record<string, unknown>
): UserError {
	return new UserError(message, { code, metadata });
}

/* ========================================================================== */
/* Structural guards                                                          */
/* ========================================================================== */

/** True for a non-null, non-array object. */
export function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Reads `key` from an unknown value without asserting anything about it. */
export function pick(value: unknown, key: string): unknown {
	return isRecord(value) ? value[key] : undefined;
}

export function isFiniteNumber(value: unknown): value is number {
	return typeof value === 'number' && Number.isFinite(value);
}

/**
 * SQLite integers can surface as `number`, `bigint` or (via some drivers) a
 * decimal string. All three are accepted; anything else is a bug in the driver
 * rather than in the caller, so it throws.
 */
export function toInt(value: unknown, fallback = 0): number {
	if (typeof value === 'number' && Number.isFinite(value)) return Math.trunc(value);
	if (typeof value === 'bigint') return Number(value);
	if (typeof value === 'string' && value.trim() !== '') {
		const parsed = Number(value);
		if (Number.isFinite(parsed)) return Math.trunc(parsed);
	}
	if (value === null || value === undefined) return fallback;
	throw protocolError('internal_error', 'unexpected non-integer column value');
}

/** As {@link toInt}, but `NULL` stays `null`. */
export function toIntOrNull(value: unknown): number | null {
	if (value === null || value === undefined) return null;
	return toInt(value);
}

export function toFloat(value: unknown, fallback = 0): number {
	if (typeof value === 'number' && Number.isFinite(value)) return value;
	if (typeof value === 'bigint') return Number(value);
	if (typeof value === 'string' && value.trim() !== '') {
		const parsed = Number(value);
		if (Number.isFinite(parsed)) return parsed;
	}
	return fallback;
}

export function toStringColumn(value: unknown, fallback = ''): string {
	if (typeof value === 'string') return value;
	if (typeof value === 'number' || typeof value === 'bigint') return String(value);
	return fallback;
}

export function toStringOrNull(value: unknown): string | null {
	if (value === null || value === undefined) return null;
	return toStringColumn(value);
}

/* ========================================================================== */
/* Identifier and enum validation                                             */
/* ========================================================================== */

/**
 * Ids we accept from a caller: URL-safe, bounded, no separators we use in
 * cursors, no whitespace.
 *
 * This is not decoration. `matchId` is a primary key and part of the pagination
 * cursor; constraining its alphabet means a cursor can be parsed unambiguously
 * and a log line cannot be forged with an embedded newline.
 */
const ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;

export function requireId(value: unknown, field: string): string {
	if (typeof value !== 'string' || !ID_PATTERN.test(value)) {
		throw protocolError('internal_error', `${field} must match ${String(ID_PATTERN)}`);
	}
	return value;
}

/** As {@link requireId} but reports a client-facing code instead of a server fault. */
export function requireClientId(value: unknown, field: string): string {
	if (typeof value !== 'string' || !ID_PATTERN.test(value)) {
		throw protocolError('game_not_found', `${field} is not a valid id`);
	}
	return value;
}

export function requireOneOf<const T extends readonly string[]>(
	value: unknown,
	allowed: T,
	field: string,
	code: ProtocolErrorCode = 'internal_error'
): T[number] {
	if (typeof value === 'string') {
		for (const candidate of allowed) {
			if (candidate === value) return candidate;
		}
	}
	throw protocolError(code, `${field} must be one of: ${allowed.join(', ')}`);
}

export function requireInt(value: unknown, field: string, min: number, max: number): number {
	if (!isFiniteNumber(value) || !Number.isInteger(value) || value < min || value > max) {
		throw protocolError('internal_error', `${field} must be an integer in [${min}, ${max}]`);
	}
	return value;
}

/** Clamps into range. Non-finite input is a caller bug and throws. */
export function clampNumber(
	value: unknown,
	min: number,
	max: number,
	field: string,
	code: ProtocolErrorCode = 'invalid_persona'
): number {
	if (!isFiniteNumber(value)) {
		throw protocolError(code, `${field} must be a finite number`);
	}
	return Math.min(max, Math.max(min, value));
}

export function requireBoolean(value: unknown, field: string): boolean {
	if (typeof value !== 'boolean') {
		throw protocolError('invalid_persona', `${field} must be a boolean`);
	}
	return value;
}

/* ========================================================================== */
/* Text hygiene                                                               */
/* ========================================================================== */

/** C0 and C1 control characters, minus the ones a multiline field may keep. */
const CONTROL_CHARS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g;

/**
 * Characters that are invisible when rendered but present in a prompt: soft
 * hyphen, Arabic letter mark, Mongolian vowel separator, zero-width space
 * through right-to-left mark, the bidi overrides, the invisible operators, the
 * isolates, and the byte-order mark.
 *
 * A prompt is shown to the user in Settings and later handed to a model. Text
 * that reads as one thing on screen and another to the model is the mechanism
 * behind most fence-escape tricks, so it is removed at the point of storage.
 */
const INVISIBLE_CHARS =
	/[\u00ad\u061c\u180e\u200b-\u200f\u202a-\u202e\u2060-\u2064\u2066-\u206f\ufeff]/g;

/** Unicode line/paragraph separators, which are newlines to some parsers only. */
const EXOTIC_BREAKS = /[\u2028\u2029\u0085]/g;

export interface SanitizeOptions {
	/** `true` keeps `\n` and `\t`; `false` folds all whitespace to single spaces. */
	readonly multiline: boolean;
	readonly maxChars: number;
	readonly field: string;
	/** Code reported when the value is present but not a string. */
	readonly code?: ProtocolErrorCode;
}

/**
 * Normalises, strips, collapses and clamps one piece of user-authored text.
 *
 * The clamp counts **code points**, not UTF-16 units, so a cap can never split a
 * surrogate pair and leave a lone half in the database.
 */
export function sanitizeText(value: unknown, opts: SanitizeOptions): string {
	const code = opts.code ?? 'invalid_persona';
	if (typeof value !== 'string') {
		throw protocolError(code, `${opts.field} must be a string`);
	}

	let text = value.normalize('NFC');
	text = text.replace(EXOTIC_BREAKS, opts.multiline ? '\n' : ' ');
	text = text.replace(/\r\n?/g, '\n');
	text = text.replace(INVISIBLE_CHARS, '');
	text = text.replace(CONTROL_CHARS, '');

	if (opts.multiline) {
		// Collapse runs of blank lines to a single blank line and trailing spaces
		// away, so a prompt cannot be padded to push a fence off a context window.
		text = text.replace(/[^\S\n]+/g, ' ');
		text = text.replace(/ *\n */g, '\n');
		text = text.replace(/\n{3,}/g, '\n\n');
	} else {
		text = text.replace(/\s+/g, ' ');
	}

	text = text.trim();

	const points = Array.from(text);
	if (points.length > opts.maxChars) {
		text = points.slice(0, opts.maxChars).join('').trim();
	}
	return text;
}

/* ========================================================================== */
/* JSON columns                                                               */
/* ========================================================================== */

/**
 * Encodes a value written by this actor into a JSON column.
 *
 * Provenance matters here: everything stored as JSON either arrived inside an
 * internal-token-authenticated message or was built from validated scalars, and
 * it is read back only by this module. The blob is therefore round-tripped
 * rather than re-validated field by field on read — the alternative is a second
 * copy of the engine's type declarations that would drift.
 */
export function encodeJson(value: unknown): string {
	return JSON.stringify(value ?? null);
}

/** Decodes a JSON column written by {@link encodeJson}. `fallback` on any fault. */
export function decodeJson<T>(value: unknown, fallback: T): T {
	if (typeof value !== 'string' || value === '') return fallback;
	try {
		const parsed: unknown = JSON.parse(value);
		return parsed === null ? fallback : (parsed as T);
	} catch {
		// A malformed blob is corruption, not a caller error. Degrade to the
		// fallback so one bad row cannot take out the whole history page.
		return fallback;
	}
}

/** Reads a `[number, number]` pair column pair back into a tuple. */
export function pair(a: unknown, b: unknown): [number, number] {
	return [toInt(a), toInt(b)];
}

/* ========================================================================== */
/* Arrays                                                                     */
/* ========================================================================== */

export function asArray(value: unknown): readonly unknown[] {
	return Array.isArray(value) ? value : [];
}

/** Rejects rather than truncates: an over-long batch is a caller bug. */
export function requireArray(value: unknown, field: string, maxLength: number): unknown[] {
	if (!Array.isArray(value)) {
		throw protocolError('internal_error', `${field} must be an array`);
	}
	if (value.length > maxLength) {
		throw protocolError('internal_error', `${field} may hold at most ${maxLength} entries`);
	}
	return value;
}
