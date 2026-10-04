/**
 * `tempo.ts` — the millisecond constants for the table's choreography.
 *
 * Values come from `docs/04-FRONTEND-UX.md` §9.2 ("the tempo table"). This
 * module owns only the numbers and the reduced-motion collapse rule; it has no
 * opinion about *what* animates, which stays with the component that owns the
 * thing being animated (`Hand`/`OpponentHand` for the deal, `TrickPile` for
 * plays and the sweep, `TableScene` for the hand-score beat).
 *
 * ## Why the "read pause" and "score delay" aren't client sleeps here
 *
 * `docs/04-FRONTEND-UX.md` §9.1 describes `trick_resolve` and `hand_score` as
 * *server*-held phases (`c.schedule.after(…, 'onTempoGate', …)` in
 * `$lib/actors/euchre-table`): the table actor keeps broadcasting the
 * trick-just-resolved / hand-just-scored view for that many real milliseconds
 * before advancing. By the time this client's `sync` for the *next* trick or
 * hand arrives, that pause has already elapsed on the wire — there is nothing
 * for this module to sleep for. What *is* this module's job is the cosmetic
 * sequencing that happens once that next view lands: how long the sweep takes,
 * how long the score celebration plays, how a card's flight is eased. None of
 * it gates when the next view is allowed to apply — see the "never gate or
 * delay" comment on `TrickPile.svelte` and `TableScene.svelte`.
 */

export const TEMPO = {
	/** Deal: per-card flight duration once its packet's turn comes up. */
	dealFlightMs: 180,
	/** Deal: stagger between successive cards' flight start, in the engine's packet order (approximated — see `Hand.svelte`). */
	dealStaggerMs: 50,

	/** Card play: the card's flight from hand to trick slot. */
	cardPlayFlightMs: 560,
	/** Card play: height of the mid-flight arc, world metres. */
	cardPlayArcM: 0.03,

	/** Trick sweep: winner's ring pulse fires this long after the trick reads as resolved. */
	trickWinnerPulseDelayMs: 150,
	/** Trick sweep: flight duration from trick slot to the winner's pile. */
	trickSweepMs: 680,
	/** Trick sweep: stagger between the (up to four) cascading cards. */
	trickSweepStaggerMs: 80,

	/** Hand score: delay before the counter starts — comprehension time, preserved under reduced motion. */
	handScoreDelayMs: 600,
	/** Hand score: counter run time — comprehension time, preserved under reduced motion. */
	handScoreCounterMs: 450,
	/** Hand score: celebration length for a plain point or march. */
	celebratePointMarchMs: 1400,
	/** Hand score: celebration length for a euchre or a loner (either side). */
	celebrateEuchreLonerMs: 2400,

	/** Up-card reveal: stands in for the flip `Card.svelte` cannot perform (see `Kitty.svelte`). */
	upCardRevealMs: 260,

	/** Game over: celebration length — the biggest beat on the table, per doc §9.2's dim/banner/confetti sequence. */
	gameWonMs: 2400
} as const;

/**
 * Collapses a *flight* duration under reduced motion (`docs/04-FRONTEND-UX.md`
 * §9.2: "sets every flight to 0.001s and kills rotation, arc and confetti").
 * Never call this on `handScoreDelayMs`/`handScoreCounterMs` — those are
 * comprehension time, not decoration, and stay at full length regardless (the
 * same section: "reduced motion must not mean reduced comprehension time").
 */
export function flightMs(ms: number, reducedMotion: boolean): number {
	return reducedMotion ? 1 : ms;
}
