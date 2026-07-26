/**
 * `aiSeat/difficulty` — how wide a choice the model is given.
 *
 * Difficulty narrows the **candidate set**, never legality, never latency, never
 * token count. All three settings issue exactly one API call with the same prompt
 * size; what changes is how many of the engine's ranked moves the model is allowed
 * to pick between.
 *
 * - `expert` — the full legal set. The model may do anything the rules allow.
 * - `casual` — the heuristic's top three. Competent, still capable of character.
 * - `rookie` — the top three *after seeded noise*, so a beginner opponent makes
 *   beginner mistakes and makes the same ones again in a replay.
 *
 * The noise stream is derived from the turn id, not from `Math.random()`: the same
 * turn always narrows to the same three moves, on the first run and on a redelivery
 * of the same durable message. Randomness never enters this actor from the runtime.
 */

import { fnv1a, mulberry32 } from '$lib/euchre';
import type { Difficulty, LegalMove, LegalMoveId, RankedMove, Seat } from '$lib/protocol';

/** How many moves `casual` and `rookie` see. */
const NARROW_TO = 3;

/** How much a `rookie`'s judgement wobbles, in raw score units. */
const ROOKIE_NOISE = 1.2;

/**
 * A deterministic `[0, 1)` stream for one turn at one seat.
 *
 * Seeded from the server turn nonce, so it is reproducible in replay and identical
 * across a crash-and-redeliver of the same `decide` message. This is the only
 * source of randomness anywhere in the AI seat, and it is not random.
 */
export function turnRng(turnId: string, seat: Seat): () => number {
	return mulberry32(fnv1a(`${turnId}#${seat}`));
}

/**
 * The moves the model is allowed to choose between.
 *
 * Always a subset of `legal`, always non-empty when `legal` is non-empty, and
 * always in the caller's own `LegalMove` objects — ids are never synthesised, so a
 * narrowed set cannot introduce an illegal move.
 */
export function narrowByDifficulty(
	ranking: readonly RankedMove[],
	legal: readonly LegalMove[],
	difficulty: Difficulty,
	rnd: () => number
): LegalMove[] {
	if (legal.length === 0) return [];
	if (difficulty === 'expert') return [...legal];

	const byId = new Map<LegalMoveId, LegalMove>();
	for (const m of legal) byId.set(m.id, m);

	const ordered =
		difficulty === 'casual'
			? [...ranking]
			: ranking
					.map((r) => ({ id: r.id, s: r.score + (rnd() - 0.5) * ROOKIE_NOISE }))
					.sort((a, b) => (b.s !== a.s ? b.s - a.s : a.id < b.id ? -1 : 1));

	const out: LegalMove[] = [];
	for (const r of ordered) {
		const m = byId.get(r.id);
		if (m !== undefined && !out.includes(m)) out.push(m);
		if (out.length >= NARROW_TO) break;
	}

	// A ranking that did not cover the legal set must never shrink it to nothing.
	return out.length > 0 ? out : [...legal];
}
