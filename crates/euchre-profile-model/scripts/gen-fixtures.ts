/**
 * Generates the euchre-profile-model goldens from the real TS profile code.
 *
 *   bun run crates/euchre-profile-model/scripts/gen-fixtures.ts
 *
 * Reads the TS sources read-only from `EUCHRE_TS_SRC` (default `~/goods/euchre`)
 * and writes JSON fixtures to `crates/euchre-profile-model/tests/fixtures/`.
 * Every clock is fixed. The repository runs against an in-memory bun:sqlite
 * database behind a RawAccess shim that logs each statement, its bound values
 * and the rows SQLite returned, so the Rust row codecs are checked against the
 * exact rows TS saw. Expected values are stored as `JSON.stringify` text; the
 * Rust tests compare `serde_json::to_string` output byte for byte.
 */

import { Database } from 'bun:sqlite';
import { mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

const SRC = resolve(process.env.EUCHRE_TS_SRC ?? join(homedir(), 'goods', 'euchre'));
const OUT = resolve(import.meta.dir, '..', 'tests', 'fixtures');
const PP = join(SRC, 'src', 'lib', 'actors', 'player-profile');

const guards = await import(join(PP, 'guards.ts'));
const settings = await import(join(PP, 'settings.ts'));
const repo = await import(join(PP, 'repository.ts'));
const schema = await import(join(PP, 'schema.ts'));
const serverPersonas = await import(join(SRC, 'src', 'lib', 'server', 'personas.ts'));
const presets = await import(join(SRC, 'src', 'routes', 'settings', 'presets.ts'));
const engine = await import(join(SRC, 'src', 'lib', 'euchre', 'index.ts'));

const NOW = 1_762_819_200_000; // 2025-11-11T00:00:00.000Z
const DAY = 86_400_000;

type Outcome = { ok: string } | { error: { code: string; message: string } };

function run(fn: () => unknown): Outcome {
	try {
		return { ok: JSON.stringify(fn()) };
	} catch (err) {
		const e = err as { code?: unknown; message?: unknown };
		if (typeof e?.code !== 'string') throw err;
		return { error: { code: e.code, message: String(e.message) } };
	}
}

async function runAsync(fn: () => Promise<unknown>): Promise<Outcome> {
	try {
		return { ok: JSON.stringify(await fn()) };
	} catch (err) {
		const e = err as { code?: unknown; message?: unknown };
		if (typeof e?.code !== 'string') throw err;
		return { error: { code: e.code, message: String(e.message) } };
	}
}

/** Absent keys stay absent: `undefined` inputs are written as a missing field. */
function write(name: string, data: unknown): void {
	mkdirSync(OUT, { recursive: true });
	writeFileSync(join(OUT, name), JSON.stringify(data, null, '\t') + '\n');
	console.log(`wrote ${name}`);
}

/* -------------------------------------------------------------------------- */
/* sanitizeText                                                                */
/* -------------------------------------------------------------------------- */

const emoji40 = '\u{1F600}'.repeat(40);
const sanitizeInputs: { value?: unknown; multiline: boolean; maxChars: number; code?: string }[] = [
	{ value: 'Marge', multiline: false, maxChars: 32 },
	{ value: '  padded\tname  ', multiline: false, maxChars: 32 },
	{ value: 'éclair', multiline: false, maxChars: 32 },
	{ value: 'line sep para\u0085nel', multiline: false, maxChars: 64 },
	{ value: 'line sep para\u0085nel', multiline: true, maxChars: 64 },
	{ value: 'a\r\nb\rc\n\rd', multiline: true, maxChars: 64 },
	{ value: 'zero​width‮RLO⁦iso­shy﻿bom؜alm᠎mvs⁤', multiline: false, maxChars: 64 },
	{ value: 'ctl\u0000\u0007\u000b\u000c\u001b\u007f\u0090end', multiline: false, maxChars: 64 },
	{ value: 'tab\there\u000bvt', multiline: true, maxChars: 64 },
	{ value: 'p1  \n\n\n\n  p2 \t \n p3', multiline: true, maxChars: 200 },
	{ value: 'x \n \n \n y', multiline: true, maxChars: 200 },
	{ value: ' nbsp　ideo em ogham nnbsp mmsp ', multiline: false, maxChars: 64 },
	{ value: ' 　  \n   ', multiline: true, maxChars: 64 },
	{ value: emoji40, multiline: false, maxChars: 32 },
	{ value: 'abc def ghi', multiline: false, maxChars: 4 },
	{ value: 'a'.repeat(2100), multiline: true, maxChars: 2000 },
	{ value: ' lead\n\n\ntrail \n', multiline: true, maxChars: 2000 },
	{ value: '\u{1F600}‍\u{1F468}', multiline: false, maxChars: 2 },
	{ value: '', multiline: false, maxChars: 32 },
	{ value: 42, multiline: false, maxChars: 32 },
	{ value: null, multiline: false, maxChars: 32 },
	{ multiline: false, maxChars: 32 },
	{ value: { name: 'x' }, multiline: true, maxChars: 32 },
	{ value: ['x'], multiline: false, maxChars: 32, code: 'internal_error' },
	{ value: true, multiline: false, maxChars: 32, code: 'internal_error' }
];

write(
	'sanitize.json',
	sanitizeInputs.map((input, i) => ({
		...input,
		field: `f${i}`,
		...run(() =>
			guards.sanitizeText(input.value, {
				multiline: input.multiline,
				maxChars: input.maxChars,
				field: `f${i}`,
				...(input.code ? { code: input.code } : {})
			})
		)
	}))
);

/* -------------------------------------------------------------------------- */
/* Settings                                                                    */
/* -------------------------------------------------------------------------- */

const base = settings.defaultSettings(NOW);
const longText = 'word '.repeat(300);

const patchCases: { name: string; patch?: unknown; chain?: boolean }[] = [
	{ name: 'null patch', patch: null },
	{ name: 'array patch', patch: [] },
	{ name: 'string patch', patch: 'housePrompt' },
	{ name: 'missing patch' },
	{ name: 'empty patch', patch: {} },
	{ name: 'unknown keys ignored', patch: { theme: 'dark', id: 'x', version: 99 } },
	{ name: 'house prompt', patch: { housePrompt: '  Quiet table.\r\n\r\n\r\nNo coaching.  ' } },
	{ name: 'house prompt same text', patch: { housePrompt: base.housePrompt } },
	{ name: 'house prompt non-string', patch: { housePrompt: 7 } },
	{ name: 'house prompt null', patch: { housePrompt: null } },
	{ name: 'house prompt capped', patch: { housePrompt: longText } },
	{ name: 'house prompt empty', patch: { housePrompt: '​ \n ' } },
	{ name: 'table pace', patch: { table: { pace: 'brisk' } } },
	{ name: 'table flags', patch: { table: { banter: false, reduceMotion: true, extra: 1 } } },
	{ name: 'table bad pace', patch: { table: { pace: 'fast' } } },
	{ name: 'table pace null', patch: { table: { pace: null } } },
	{ name: 'table flag not boolean', patch: { table: { sound: 'yes' } } },
	{ name: 'table not object', patch: { table: [true] } },
	{ name: 'table null', patch: { table: null } },
	{ name: 'personas not array', patch: { personas: { seat: 1 } } },
	{ name: 'personas empty', patch: { personas: [] } },
	{ name: 'too many persona patches', patch: { personas: [{}, {}, {}, {}] } },
	{ name: 'persona entry not object', patch: { personas: [1] } },
	{ name: 'seat zero', patch: { personas: [{ seat: 0, patch: {} }] } },
	{ name: 'seat four', patch: { personas: [{ seat: 4, patch: {} }] } },
	{ name: 'seat string', patch: { personas: [{ seat: '1', patch: {} }] } },
	{ name: 'seat fractional', patch: { personas: [{ seat: 1.5, patch: {} }] } },
	{ name: 'seat missing', patch: { personas: [{ patch: {} }] } },
	{ name: 'seat float integral', patch: { personas: [{ seat: 2.0, patch: { name: 'Dutchie' } }] } },
	{ name: 'duplicate seat', patch: { personas: [{ seat: 1, patch: {} }, { seat: 1, patch: {} }] } },
	{ name: 'patch not object', patch: { personas: [{ seat: 3, patch: 'Verna' }] } },
	{ name: 'patch missing', patch: { personas: [{ seat: 3 }] } },
	{ name: 'empty persona patch keeps version', patch: { personas: [{ seat: 1, patch: {} }] } },
	{ name: 'same name keeps version', patch: { personas: [{ seat: 1, patch: { name: 'Marge' } }] } },
	{ name: 'rename', patch: { personas: [{ seat: 1, patch: { name: '  Big​  Marge \u0007' } }] } },
	{ name: 'name empty after sanitisation', patch: { personas: [{ seat: 2, patch: { name: '​ ­ ' } }] } },
	{ name: 'name too long', patch: { personas: [{ seat: 2, patch: { name: emoji40 } }] } },
	{ name: 'name not string', patch: { personas: [{ seat: 2, patch: { name: 12 } }] } },
	{ name: 'name null', patch: { personas: [{ seat: 2, patch: { name: null } }] } },
	{ name: 'blurb and prompt', patch: { personas: [{ seat: 3, patch: { blurb: 'Bold.\nVery bold.', prompt: 'Line one.\r\n\r\n\r\n\r\nLine two.\t\tTabbed.' } }] } },
	{ name: 'empty prompt allowed', patch: { personas: [{ seat: 3, patch: { prompt: '' } }] } },
	{ name: 'prompt capped', patch: { personas: [{ seat: 3, patch: { prompt: 'x'.repeat(2500) } }] } },
	{ name: 'blurb not string', patch: { personas: [{ seat: 3, patch: { blurb: false } }] } },
	{ name: 'difficulty', patch: { personas: [{ seat: 1, patch: { difficulty: 'rookie' } }] } },
	{ name: 'difficulty bad', patch: { personas: [{ seat: 1, patch: { difficulty: 'hard' } }] } },
	{ name: 'dials clamp', patch: { personas: [{ seat: 2, patch: { aggression: -1, risk: 2, chattiness: 0.125, temperature: 1e-7 } }] } },
	{ name: 'dial same value keeps version', patch: { personas: [{ seat: 2, patch: { aggression: 0.5 } }] } },
	{ name: 'dial string', patch: { personas: [{ seat: 2, patch: { risk: '0.5' } }] } },
	{ name: 'dial null', patch: { personas: [{ seat: 2, patch: { temperature: null } }] } },
	{ name: 'models', patch: { personas: [{ seat: 1, patch: { modelId: 'claude-opus-5', bidModelId: 'claude-haiku-4-5' } }] } },
	{ name: 'model bad', patch: { personas: [{ seat: 1, patch: { bidModelId: 'gpt-5' } }] } },
	{ name: 'error after earlier seat is all-or-nothing', patch: { personas: [{ seat: 1, patch: { name: 'Ok' } }, { seat: 2, patch: { risk: 'x' } }] } },
	{ name: 'everything at once', patch: { housePrompt: 'House.', table: { pace: 'relaxed', showRationale: false }, personas: [{ seat: 3, patch: { name: 'V', difficulty: 'expert', chattiness: 0.3 } }, { seat: 1, patch: { blurb: '' } }] } },
	{ name: 'chained second patch', patch: { personas: [{ seat: 3, patch: { name: 'V2' } }] }, chain: true },
	{ name: 'chained house prompt revert', patch: { housePrompt: base.housePrompt }, chain: true }
];

let chainCurrent = base;
const patchFixtures = patchCases.map((c, i) => {
	const current = c.chain ? chainCurrent : base;
	const now = NOW + (i + 1) * 1000;
	const outcome = run(() => settings.applySettingsPatch(current, c.patch, now));
	if ('ok' in outcome) chainCurrent = JSON.parse(outcome.ok);
	return { name: c.name, current: JSON.stringify(current), patch: c.patch, now, ...outcome };
});

const normalizeCases: { name: string; raw?: unknown }[] = [
	{ name: 'undefined' },
	{ name: 'null', raw: null },
	{ name: 'array', raw: [base] },
	{ name: 'empty object', raw: {} },
	{ name: 'already valid', raw: base },
	{ name: 'legacy without personas or table', raw: { schema: 1, version: 4, updatedAt: NOW - DAY, housePrompt: 'Old house.' } },
	{
		name: 'legacy partial personas',
		raw: {
			version: 2.9,
			updatedAt: '2025-01-01',
			housePrompt: 'Mixed\u0000 house\n\n\n\nprompt',
			personas: [
				{ id: 'marge', version: 3, name: 'Marge', difficulty: 'expert' },
				{ id: 'Dutch!', version: 1.7, name: 42, blurb: 'b​b', aggression: 3, risk: -2, chattiness: '0.2', temperature: 0.33, modelId: 'claude-opus-5', bidModelId: 'nope' },
				'verna'
			],
			table: { pace: 'relaxed', banter: 'no', sound: false, reduceMotion: 1 }
		}
	},
	{ name: 'personas not array', raw: { personas: { 0: base.personas[0] }, table: [] } },
	{ name: 'extra personas dropped', raw: { personas: [...base.personas, base.personas[0]], extra: true } },
	{ name: 'over-long fields cut', raw: { housePrompt: longText, personas: [{ name: emoji40, blurb: 'b'.repeat(200), prompt: 'p '.repeat(1200) }] } },
	{ name: 'house prompt not string', raw: { housePrompt: ['x'], personas: [{ housePrompt: 'ignored' }] } },
	{ name: 'id rules', raw: { personas: [{ id: 'a'.repeat(48) }, { id: 'a'.repeat(49) }, { id: '' }] } },
	{ name: 'version zero kept', raw: { version: 0, updatedAt: 0, personas: [{ version: 0 }] } }
];

const normalizeFixtures = normalizeCases.map((c) => ({
	name: c.name,
	raw: c.raw,
	now: NOW,
	...run(() => settings.normalizeSettings(c.raw, NOW))
}));

write('settings.json', {
	defaultSettings: { now: NOW, ok: JSON.stringify(settings.defaultSettings(NOW)) },
	personaViews: JSON.stringify(base.personas.map((p: unknown, i: number) => settings.personaView(p, i + 1))),
	personasFromSettings: JSON.stringify(
		serverPersonas.personasFromSettings({ ...base, housePrompt: 'Settings house.' })
	),
	presets: JSON.stringify(presets.PERSONA_PRESETS),
	defaultEngineConfig: JSON.stringify(engine.DEFAULT_ENGINE_CONFIG),
	patch: patchFixtures,
	normalize: normalizeFixtures
});

/* -------------------------------------------------------------------------- */
/* Repository over bun:sqlite                                                  */
/* -------------------------------------------------------------------------- */

interface Call {
	sql: string;
	args: unknown[];
	rows: unknown[];
}

const sqlite = new Database(':memory:');
let calls: Call[] = [];
const rawAccess = {
	async execute(sql: string, ...args: unknown[]): Promise<unknown[]> {
		const rows = sqlite.query(sql).all(...(args as never[])) as unknown[];
		calls.push({ sql, args, rows });
		return rows;
	},
	async transaction<T>(cb: (tx: unknown) => Promise<T>): Promise<T> {
		return cb(rawAccess);
	}
};
await schema.applyMigrations(rawAccess);
calls = [];

function take(): Call[] {
	const out = calls;
	calls = [];
	return out;
}

const OWNER = 'user_7f3a';
const assignments = serverPersonas.personasFromSettings(base) as Record<string, unknown>[];
const dirtyAssignments = [
	assignments[0],
	{ ...assignments[1], persona: { ...(assignments[1].persona as object), name: '  Dutch​\u0007  the   Elder  ' } },
	{ seat: 3, persona: { name: 'Verna' }, dossier: 'x' },
	{ seat: 1, persona: { name: 'Shadow' } }
];

function record(over: Record<string, unknown>): Record<string, unknown> {
	return {
		schema: 1,
		matchId: 'm_base',
		userId: OWNER,
		seed: 'seed-base',
		cfg: engine.DEFAULT_ENGINE_CONFIG,
		firstDealer: 1,
		startedAt: NOW - 10 * DAY,
		endedAt: NOW - 10 * DAY + 1_800_000,
		status: 'complete',
		outcome: 'won',
		score: [10, 7],
		winnerTeam: 0,
		handsPlayed: 9,
		personas: assignments,
		stats: { euchresFor: 2, euchresAgainst: 1, lonersAttempted: 1, lonersMade: 1, marches: 2, throwIns: 0, tricks: { 0: 27, 1: 18 } },
		tokens: { tokensIn: 12000, tokensOut: 3400, cacheReadTokens: 50000, calls: 61 },
		...over
	};
}

const matchRecords: Record<string, unknown>[] = [
	record({ matchId: 'm_alpha' }),
	record({ matchId: 'm_bravo', outcome: 'lost', score: [6, 10], winnerTeam: 1, startedAt: NOW - 8 * DAY, endedAt: NOW - 8 * DAY + 2_000_000, personas: dirtyAssignments, stats: { euchresFor: 0, euchresAgainst: 3, lonersAttempted: 2, lonersMade: 0, marches: 1, throwIns: 2, tricks: [20, 25] }, tokens: { tokensIn: 9000.7, tokensOut: '120', cacheReadTokens: -5, calls: null } }),
	record({ matchId: 'm_charlie', startedAt: NOW - 5 * DAY, endedAt: NOW - 5 * DAY + 1_000_000, cfg: { gameTo: 5, stickTheDealer: false, 2: 'two', 10: 'ten', custom: { b: 1.5, a: 1.0 } } }),
	record({ matchId: 'm_delta', startedAt: NOW - 5 * DAY - 1_000_000 + 1_000_000, endedAt: null, status: 'active', outcome: 'abandoned', score: [3, 4], winnerTeam: null, handsPlayed: 4, firstDealer: undefined, stats: undefined, tokens: undefined, cfg: 'not-a-record' }),
	record({ matchId: 'm_echo', startedAt: NOW - 5 * DAY, endedAt: NOW - 5 * DAY + 1_000_000, outcome: 'lost', score: [0, 10], winnerTeam: 1, handsPlayed: 6 }),
	record({ matchId: 'm_foxtrot', status: 'abandoned', outcome: 'abandoned', startedAt: NOW - DAY, endedAt: NOW - DAY + 60_000, score: [2, 0], winnerTeam: null, handsPlayed: 1, personas: [] }),
	record({ matchId: 'm_golf', startedAt: NOW - 2 * DAY, endedAt: NOW - 2 * DAY, score: [10, 9], handsPlayed: 12, seed: 's'.repeat(256) })
];

const writes: unknown[] = [];
for (const raw of matchRecords) {
	const now = NOW;
	const parsed = repo.parseMatchRecord(raw, OWNER, now);
	take();
	const outcome = await repo.upsertMatch(rawAccess, parsed, now);
	writes.push({ kind: 'match', raw, now, record: JSON.stringify(parsed), outcome: JSON.stringify(outcome), calls: take() });
}
// A live match advanced at a later hand boundary, then a late retry of a finished one.
{
	const raw = record({ matchId: 'm_delta', startedAt: NOW - 5 * DAY, endedAt: null, status: 'active', outcome: 'abandoned', score: [7, 4], winnerTeam: null, handsPlayed: 6 });
	const parsed = repo.parseMatchRecord(raw, OWNER, NOW + 5000);
	take();
	const outcome = await repo.upsertMatch(rawAccess, parsed, NOW + 5000);
	writes.push({ kind: 'match', raw, now: NOW + 5000, record: JSON.stringify(parsed), outcome: JSON.stringify(outcome), calls: take() });
}
{
	const raw = record({ matchId: 'm_alpha', outcome: 'lost', score: [1, 10] });
	const parsed = repo.parseMatchRecord(raw, OWNER, NOW + 6000);
	take();
	const outcome = await repo.upsertMatch(rawAccess, parsed, NOW + 6000);
	writes.push({ kind: 'match', raw, now: NOW + 6000, record: JSON.stringify(parsed), outcome: JSON.stringify(outcome), calls: take() });
}

const move = (seat: number, id: string, m: Record<string, unknown>, extra: Record<string, unknown> = {}) => ({ seat, id, move: m, kind: seat === 0 ? 'human' : 'ai', ...extra });
const handsRaw = [
	{
		matchId: 'm_alpha',
		handNo: 0,
		seed: 'hand-seed-0',
		dealerSeat: 1,
		deckOrder: ['9S', 'TS', 'JS', 'QS', 'KS', 'AS', 7, null, '9H'],
		moves: [
			move(2, 'a1', { t: 'pass' }, { source: 'llm' }),
			move(3, 'a2', { t: 'orderUp', alone: false }),
			move(1, 'a3', { t: 'discard', card: 'JH' }),
			move(0, 'a4', { t: 'play', card: 'AS' }),
			move(3, 'a5', { t: 'discard', card: '9D' }, { reason: 'ai_timeout', source: 'fallback' }),
			move(0, 'a6', { t: 'discard', card: 'TC' }),
			'not-an-action',
			{ seat: 2, move: 'discard' },
			{ seat: '0', move: { t: 'discard' } },
			{ move: { t: 'play', card: 'KD', extra: { 1: 'x', z: 0.1 } }, seat: 1 }
		],
		trump: 'S',
		makerSeat: 3,
		aloneSeat: null,
		tricksWon: [2, 3],
		result: 'point',
		delta: [0, 1],
		endedAt: NOW - 10 * DAY + 300_000
	},
	{ handNo: 1, dealerSeat: 2, moves: [], trump: null, result: 'throw_in', delta: null, endedAt: NOW - 10 * DAY + 400_000 },
	{ matchId: 'm_alpha', handNo: 2, seed: 7, makerSeat: 0, aloneSeat: 0, trump: 'H', tricksWon: [5, 0], result: 'lone_march', delta: [4, 0], moves: [move(0, 'b1', { t: 'call', suit: 'H', alone: true })] }
];
take();
const handsParsed = repo.parseHands(handsRaw, 'm_alpha');
await repo.insertHands(rawAccess, handsParsed);
writes.push({ kind: 'hands', raw: handsRaw, matchId: 'm_alpha', rows: JSON.stringify(handsParsed), calls: take() });
const retryHands = repo.parseHands([handsRaw[1]], 'm_alpha');
await repo.insertHands(rawAccess, retryHands);
take();
const bravoHands = repo.parseHands([{ handNo: 0, dealerSeat: 0, moves: [move(0, 'c1', { t: 'discard', card: '9C' })], trump: 'D', makerSeat: 0, tricksWon: [3, 2], result: 'point', delta: [1, 0], endedAt: NOW - 8 * DAY + 10 }], 'm_bravo');
await repo.insertHands(rawAccess, bravoHands);
writes.push({ kind: 'hands', raw: null, matchId: 'm_bravo', rows: JSON.stringify(bravoHands), calls: take() });

const episodesRaw = [
	{ role: 'marge', matchId: 'm_alpha', handNo: 0, kind: 'bid', ts: NOW - 30 * DAY, summary: 'Ordered up on three small trump.', salience: 0.9 },
	{ role: 'marge', matchId: 'm_alpha', handNo: 1, kind: 'euchre', ts: NOW - 15 * DAY, summary: 'Got euchred​ by the\nhuman.', salience: 0.6 },
	{ role: 'marge', matchId: 'm_bravo', handNo: 0, kind: 'read', ts: NOW - 13 * DAY, summary: 'Human leads trump when strong.', salience: 0.5 },
	{ role: 'marge', matchId: 'm_bravo', handNo: 1, kind: 'loner', ts: NOW - 2 * DAY, summary: 'x'.repeat(200), salience: 0.2 },
	{ role: 'marge', matchId: 'm_charlie', handNo: 0, kind: 'play', ts: NOW - 28 * DAY - 1, summary: 'Old but bright.', salience: '1.0' },
	{ role: 'marge', matchId: 'm_charlie', handNo: 1, kind: 'play', ts: NOW - 14 * DAY, summary: 'Exactly one half-life.', salience: 0.3 },
	{ role: 'marge', matchId: 'm_charlie', handNo: 2, kind: 'bid', ts: NOW - 14 * DAY + 1, summary: 'Just under one half-life.', salience: 0.3 },
	{ role: 'marge', matchId: 'm_echo', handNo: 0, kind: 'read', ts: NOW + DAY, summary: 'From the future.', salience: 0.1 },
	{ role: 'marge', matchId: 'm_echo', handNo: 1, kind: 'bid', ts: NOW - DAY, summary: 'Expired already.', salience: 1, expiresAt: NOW - 1 },
	{ role: 'marge', matchId: 'm_echo', handNo: 2, kind: 'bid', ts: NOW - 3 * DAY, summary: 'No salience.', salience: true },
	{ role: 'dutch', matchId: 'm_alpha', handNo: 0, kind: 'play', ts: NOW - 3 * DAY, summary: 'Partner trusted the call.', salience: 2 },
	{ role: 'verna', matchId: 'm_alpha', handNo: 0, kind: 'euchre', ts: NOW - DAY, summary: 'Loved the march.', salience: -0.5, expiresAt: NOW + 400 * DAY },
	{ role: 'verna', matchId: 'm_golf', handNo: 4, kind: 'loner', summary: 'Default ts.', salience: 0.75 }
];
take();
const episodesParsed = repo.parseEpisodes(episodesRaw, NOW - 1000);
await repo.insertEpisodes(rawAccess, episodesParsed);
writes.push({ kind: 'episodes', raw: episodesRaw, now: NOW - 1000, rows: JSON.stringify(episodesParsed), calls: take() });

// Reads.
const listCases: { limit?: unknown; cursor?: unknown }[] = [
	{},
	{ limit: 2 },
	{ limit: 2, cursor: `${NOW - 5 * DAY + 1_000_000}:m_echo` },
	{ limit: 3, cursor: `${NOW - 5 * DAY + 1_000_000}:m_charlie` },
	{ limit: 50, cursor: `${NOW - 10 * DAY + 1_800_000}:m_alpha` },
	{ limit: 1, cursor: '' },
	{ limit: 1, cursor: null },
	{ limit: 0.5 },
	{ limit: 999 },
	{ limit: '5' },
	{ limit: -3 },
	{ cursor: 17 },
	{ cursor: '1'.repeat(161) },
	{ cursor: ':m_alpha' },
	{ cursor: 'no-separator' },
	{ cursor: '12a:m_alpha' },
	{ cursor: '1234567890123456:m_alpha' },
	{ cursor: '123:bad id' },
	{ cursor: '123:' },
	{ cursor: '123:m:alpha' },
	{ cursor: '000:m_alpha' }
];

const lists = [];
for (const c of listCases) {
	take();
	const limit = repo.clampLimit(c.limit);
	const outcome = await runAsync(() => repo.listMatches(rawAccess, limit, c.cursor ?? null));
	lists.push({ req: c, limit, ...outcome, calls: take() });
}
// Walk every page at limit 2.
const walk = [];
{
	let cursor: string | null = null;
	for (let i = 0; i < 10; i += 1) {
		take();
		const page = await repo.listMatches(rawAccess, 2, cursor);
		walk.push({ cursor, ok: JSON.stringify(page), calls: take() });
		cursor = page.cursor;
		if (cursor === null) break;
	}
}

const replays = [];
for (const matchId of ['m_alpha', 'm_bravo', 'm_delta', 'm_missing']) {
	take();
	const client = await repo.readClientReplay(rawAccess, matchId);
	const clientCalls = take();
	const server = await repo.readServerReplay(rawAccess, matchId);
	const serverCalls = take();
	replays.push({ matchId, client: JSON.stringify(client), clientCalls, server: JSON.stringify(server), serverCalls });
}

take();
const stats = await repo.computeStats(rawAccess, NOW);
const statsCalls = take();

const ranked = [];
for (const [role, limit, now] of [
	['marge', 20, NOW],
	['marge', 3, NOW],
	['marge', 20, NOW + 30 * DAY],
	['dutch', 5, NOW],
	['verna', 1, NOW],
	['nobody', 5, NOW]
] as const) {
	take();
	const out = await repo.selectEpisodesForRole(rawAccess, role, limit, now);
	take();
	const candidates = sqlite
		.query('SELECT * FROM episodes WHERE role = ? AND expires_at > ? ORDER BY ts DESC LIMIT 500')
		.all(role, now);
	ranked.push({ role, limit, now, ok: JSON.stringify(out), candidates });
}

take();
const listed = await repo.listEpisodes(rawAccess, 5, NOW);
const listedCalls = take();

write('repository.json', {
	owner: OWNER,
	migrationIds: schema.MIGRATION_IDS,
	writes,
	lists,
	walk,
	replays,
	stats: { now: NOW, ok: JSON.stringify(stats), calls: statsCalls },
	ranked,
	listEpisodes: { now: NOW, limit: 5, ok: JSON.stringify(listed), calls: listedCalls }
});

/* -------------------------------------------------------------------------- */
/* Record validation                                                           */
/* -------------------------------------------------------------------------- */

const recordCases: { name: string; raw?: unknown; owner?: string }[] = [
	{ name: 'valid', raw: record({}) },
	{ name: 'not object', raw: [record({})] },
	{ name: 'missing' },
	{ name: 'other owner', raw: record({ userId: 'someone_else' }) },
	{ name: 'owner not string', raw: record({ userId: 7 }) },
	{ name: 'bad match id', raw: record({ matchId: 'has space' }) },
	{ name: 'match id too long', raw: record({ matchId: 'a'.repeat(129) }) },
	{ name: 'seed empty', raw: record({ seed: '' }) },
	{ name: 'seed 256 units', raw: record({ seed: '\u{1F600}'.repeat(128) }) },
	{ name: 'seed 257 units', raw: record({ seed: 'a' + '\u{1F600}'.repeat(128) }) },
	{ name: 'seed not string', raw: record({ seed: 5 }) },
	{ name: 'startedAt default', raw: record({ startedAt: undefined, endedAt: null }) },
	{ name: 'startedAt negative', raw: record({ startedAt: -1 }) },
	{ name: 'startedAt fractional', raw: record({ startedAt: 1.5 }) },
	{ name: 'endedAt string', raw: record({ endedAt: '123' }) },
	{ name: 'endedAt max safe', raw: record({ endedAt: Number.MAX_SAFE_INTEGER }) },
	{ name: 'endedAt over max safe', raw: record({ endedAt: Number.MAX_SAFE_INTEGER + 2 }) },
	{ name: 'personas sliced to three', raw: record({ personas: [1, 2, 3, 4, 5] }) },
	{ name: 'personas not array', raw: record({ personas: 'x' }) },
	{ name: 'cfg array falls back', raw: record({ cfg: [1] }) },
	{ name: 'cfg null falls back', raw: record({ cfg: null }) },
	{ name: 'firstDealer null', raw: record({ firstDealer: null }) },
	{ name: 'firstDealer four', raw: record({ firstDealer: 4 }) },
	{ name: 'status bad', raw: record({ status: 'done' }) },
	{ name: 'outcome bad', raw: record({ outcome: 'draw' }) },
	{ name: 'status checked before outcome', raw: record({ status: 1, outcome: 2 }) },
	{ name: 'score not pair', raw: record({ score: [1, 2, 3] }) },
	{ name: 'score not array', raw: record({ score: { 0: 1, 1: 2 } }) },
	{ name: 'score out of range', raw: record({ score: [1001, 0] }) },
	{ name: 'score second bad', raw: record({ score: [0, 'x'] }) },
	{ name: 'score negative', raw: record({ score: [-2, 10] }) },
	{ name: 'winnerTeam two', raw: record({ winnerTeam: 2 }) },
	{ name: 'winnerTeam undefined', raw: record({ winnerTeam: undefined }) },
	{ name: 'handsPlayed default', raw: record({ handsPlayed: undefined }) },
	{ name: 'handsPlayed too many', raw: record({ handsPlayed: 10001 }) },
	{ name: 'stats tricks as array reads zero', raw: record({ stats: { tricks: [5, 3], marches: 2.9, throwIns: -4 } }) },
	{ name: 'stats numeric strings', raw: record({ stats: { euchresFor: ' 3 ', lonersMade: '0x10', tricks: { 0: '7', 1: 2 } } }) },
	{ name: 'stats bad value', raw: record({ stats: { euchresFor: true } }) },
	{ name: 'stats blank string', raw: record({ stats: { marches: '  ' } }) },
	{ name: 'stats not object', raw: record({ stats: 5 }) },
	{ name: 'tokens bad value', raw: record({ tokens: { calls: 'many' } }) },
	{ name: 'tokens fractional', raw: record({ tokens: { tokensIn: 10.9, tokensOut: 1e3, cacheReadTokens: '2.5' } }) }
];
const recordFixtures = recordCases.map((c) => ({
	name: c.name,
	raw: c.raw,
	owner: c.owner ?? OWNER,
	now: NOW,
	...run(() => repo.parseMatchRecord(c.raw, c.owner ?? OWNER, NOW))
}));

const handCases: { name: string; raw?: unknown; matchId?: string }[] = [
	{ name: 'not array', raw: { handNo: 0 } },
	{ name: 'missing' },
	{ name: 'too many', raw: Array.from({ length: 65 }, (_, i) => ({ handNo: i })) },
	{ name: 'sixty-four ok', raw: Array.from({ length: 64 }, (_, i) => ({ handNo: i })) },
	{ name: 'entry not object', raw: [{ handNo: 0 }, null] },
	{ name: 'match id bad', raw: [{ matchId: 'bad id', handNo: 0 }] },
	{ name: 'other match', raw: [{ matchId: 'm_other', handNo: 0 }] },
	{ name: 'match id null uses fallback', raw: [{ matchId: null, handNo: 0 }] },
	{ name: 'deck too long', raw: [{ handNo: 0, deckOrder: Array.from({ length: 25 }, () => '9S') }] },
	{ name: 'moves too long', raw: [{ handNo: 0, moves: Array.from({ length: 129 }, () => ({})) }] },
	{ name: 'handNo missing', raw: [{}] },
	{ name: 'handNo fractional', raw: [{ handNo: 0.5 }] },
	{ name: 'seed cut to 256', raw: [{ handNo: 0, seed: 'q'.repeat(300) }] },
	{ name: 'dealer bad', raw: [{ handNo: 0, dealerSeat: -1 }] },
	{ name: 'trump bad', raw: [{ handNo: 0, trump: 's' }] },
	{ name: 'maker bad', raw: [{ handNo: 0, makerSeat: '1' }] },
	{ name: 'alone bad', raw: [{ handNo: 0, aloneSeat: 9 }] },
	{ name: 'tricks null defaults', raw: [{ handNo: 0, tricksWon: null }] },
	{ name: 'tricks bad', raw: [{ handNo: 0, tricksWon: [1] }] },
	{ name: 'result bad', raw: [{ handNo: 0, result: 'win' }] },
	{ name: 'delta bad', raw: [{ handNo: 0, delta: [0, 2000] }] },
	{ name: 'endedAt bad', raw: [{ handNo: 0, endedAt: -5 }] },
	{ name: 'first fault wins', raw: [{ handNo: 0, trump: 'X' }, { handNo: 'y' }] },
	{ name: 'full hand', raw: [handsRaw[0]] }
];
const handFixtures = handCases.map((c) => ({
	name: c.name,
	raw: c.raw,
	matchId: c.matchId ?? 'm_alpha',
	...run(() => repo.parseHands(c.raw, c.matchId ?? 'm_alpha'))
}));

const episodeCases: { name: string; raw?: unknown }[] = [
	{ name: 'not array', raw: 'x' },
	{ name: 'too many', raw: Array.from({ length: 33 }, () => ({})) },
	{ name: 'entry not object', raw: [[]] },
	{ name: 'role bad', raw: [{ role: 'mar ge', matchId: 'm', handNo: 0, kind: 'bid', summary: 's' }] },
	{ name: 'match id bad', raw: [{ role: 'marge', matchId: '', handNo: 0, kind: 'bid', summary: 's' }] },
	{ name: 'handNo bad', raw: [{ role: 'marge', matchId: 'm', handNo: 10001, kind: 'bid', summary: 's' }] },
	{ name: 'kind bad', raw: [{ role: 'marge', matchId: 'm', handNo: 0, kind: 'grudge', summary: 's' }] },
	{ name: 'ts bad', raw: [{ role: 'marge', matchId: 'm', handNo: 0, kind: 'bid', ts: 'now', summary: 's' }] },
	{ name: 'summary not string', raw: [{ role: 'marge', matchId: 'm', handNo: 0, kind: 'bid', summary: null }] },
	{ name: 'expiresAt bad', raw: [{ role: 'marge', matchId: 'm', handNo: 0, kind: 'bid', summary: 's', expiresAt: 1.5 }] },
	{ name: 'salience forms', raw: [
		{ role: 'r1', matchId: 'm', handNo: 0, kind: 'bid', summary: ' a ', salience: ' 0.25 ' },
		{ role: 'r2', matchId: 'm', handNo: 1, kind: 'play', summary: 'b', salience: {} },
		{ role: 'r3', matchId: 'm', handNo: 2, kind: 'read', summary: 'c', salience: 1e-9 },
		{ role: 'r4', matchId: 'm', handNo: 3, kind: 'loner', summary: 'd', salience: 'NaN', ts: 0, expiresAt: null }
	] },
	{ name: 'realistic', raw: episodesRaw }
];
const episodeFixtures = episodeCases.map((c) => ({
	name: c.name,
	raw: c.raw,
	now: NOW,
	...run(() => repo.parseEpisodes(c.raw, NOW))
}));

const clampCases: { raw?: unknown }[] = [{}, { raw: null }, { raw: 1 }, { raw: 50 }, { raw: 51 }, { raw: 0 }, { raw: -0.5 }, { raw: 7.9 }, { raw: '7' }, { raw: true }, { raw: 1e300 }];

const utcCases = [0, NOW, NOW - 1, 951_782_400_000, 4_102_444_800_000, 253_402_300_799_999, 253_402_300_800_000, -1, -62_167_219_200_001];

write('records.json', {
	match: recordFixtures,
	hands: handFixtures,
	episodes: episodeFixtures,
	clampLimit: clampCases.map((c) => ({ ...c, ok: JSON.stringify(repo.clampLimit(c.raw)) })),
	utcDay: utcCases.map((now) => ({ now, ok: new Date(now).toISOString().slice(0, 10) })),
	episodeIds: episodesParsed.map((e: { episodeId: string }) => e.episodeId)
});
