/**
 * `euchre-table/tempo.ts` — the table's timing policy, in one place.
 *
 * Every number here is **policy, not physics**. Invariant 22 ("no number is
 * frozen before it is measured") applies to all of them: they are starting
 * values chosen to feel like a card table, they are all in one object so a
 * measurement session can move them together, and none of them is load-bearing
 * for correctness. Correctness comes from the schedule + queue machinery in
 * `index.ts`; these constants only decide *when* a message is enqueued, never
 * *whether* the game can proceed.
 *
 * Two of them are different in kind and are called out below:
 *
 * - {@link TEMPO.aiHardCapMs} is a **safety cap**, not a feel knob. It is the
 *   watchdog delay after which the table force-plays a legal move regardless of
 *   whether the AI seat ever answered. Raising it lengthens the worst-case stall;
 *   lowering it starves a slow model. It must always exceed the AI seat's own
 *   internal budget, which is why `AIDecideRequest.deadlineAt` is derived from it
 *   and sent along — the seat clamps itself against our cap rather than the two
 *   being kept in sync by hand.
 * - The `trickResolve*` and `handScore*` holds are a **security property**, not
 *   decoration (`docs/01-ARCHITECTURE.md` §9.4). While the table sits in
 *   `trick_resolve` the next card has not been computed, so a hacked client that
 *   skips its local animation still cannot see it early. Setting these to zero
 *   would not "make the game faster", it would delete that guarantee.
 *
 * Presentation timing — the `{ brisk 0.7, table 1.0, slow 1.3 }` multiplier — is
 * entirely client-side and deliberately absent from this file.
 *
 * Nothing here reads a clock or a random number generator: {@link thinkFloorMs}
 * derives its jitter from the turn nonce, so two runs of the same match pace
 * identically and a replay is reproducible (V20).
 */

import { fnv1a } from '$lib/euchre';
import type { AIDecisionKind } from '$lib/protocol';

/**
 * Fast tempo compresses every pause so a full match is testable in minutes. It is
 * OPT-IN via `EUCHRE_FAST_TEMPO=1`.
 *
 * It used to be inferred from `RIVET_ENDPOINT` being unset, which meant anyone
 * running `bun run dev` locally got 100 ms think-floors instead of 700 ms. The
 * opponents answered instantly, cards vanished the moment you tapped them, and
 * the game read as broken rather than fast — which is exactly how it was
 * reported. Local play now feels like production by default, because the tempo
 * IS the feel; a developer who wants to grind a match quickly can ask for it.
 */
const LOCAL_FAST = process.env.EUCHRE_FAST_TEMPO === '1';

/** The table's whole timing policy. Frozen so a caller cannot drift one seat. */
export const TEMPO = Object.freeze({
	/** Server-held read pause after a trick completes. */
	trickResolveMs: LOCAL_FAST ? 120 : 900,
	/** Longer hold when the trick just made a euchre certain — the moment deserves it. */
	trickResolveSealsEuchreMs: LOCAL_FAST ? 180 : 1100,
	/** Server-held pause on the hand recap before the next deal. */
	handScoreMs: LOCAL_FAST ? 250 : 1800,

	/**
	 * Minimum time a seat appears to deliberate, by decision kind. A 380 ms
	 * answer reads as a robot, so an early decision is parked until `revealAt`
	 * (see `PendingDecision`) rather than played immediately.
	 */
	thinkFloorMs: Object.freeze(
		LOCAL_FAST
			? { cut: 80, bid1: 120, bid2: 120, discard: 120, play: 100 }
			: { cut: 500, bid1: 900, bid2: 900, discard: 900, play: 700 }
	) satisfies Record<AIDecisionKind, number>,

	/** Width of the deterministic jitter added on top of the floor. */
	thinkJitterMs: LOCAL_FAST ? 40 : 400,

	/**
	 * **Safety cap.** Watchdog delay from dispatch; past it the table force-plays.
	 * Also what `AIDecideRequest.deadlineAt` is computed from.
	 */
	aiHardCapMs: LOCAL_FAST ? 1500 : 4000,

	/** When `thinking { extended: true }` is raised so the UI escalates rather than freezing. */
	aiExtendedAtMs: LOCAL_FAST ? 800 : 2200,

	/** Partner speaks a nudge line. No state change; there is deliberately no human turn timer. */
	nudgeMs: LOCAL_FAST ? 30_000 : 90_000,

	/** Auto-play so a walked-away solo match terminates instead of pinning an actor forever. */
	abandonMs: LOCAL_FAST ? 90_000 : 240_000,

	/** Retry delay for the durable `playerProfile` write. */
	profileRetryMs: 30_000,

	/** Self-reap delay after `game_over`, and only once the durable copy exists. */
	reapMs: 600_000
});

/**
 * How long this seat must appear to think before its decision may be revealed.
 *
 * Deterministic: the jitter is a hash of the turn nonce, never `Math.random()`,
 * so the same match paces the same way on every replay and the actor keeps its
 * "no RNG outside the seeded engine" property.
 */
export function thinkFloorMs(kind: AIDecisionKind, turnId: string): number {
	const jitter = fnv1a(`${turnId}#pace`) % TEMPO.thinkJitterMs;
	return TEMPO.thinkFloorMs[kind] + jitter;
}

/** The server-held hold for a completed trick, longer when the trick sealed a euchre. */
export function trickResolveMs(sealsEuchre: boolean): number {
	return sealsEuchre ? TEMPO.trickResolveSealsEuchreMs : TEMPO.trickResolveMs;
}
