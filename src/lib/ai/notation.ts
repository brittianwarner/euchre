/**
 * The dense line notation the model actually reads.
 *
 * JSON braces, quotes and repeated keys can triple the token cost of the same
 * information, and a card game state is mostly keys. So the per-decision user
 * message is a handful of `KEY=value` lines, one per fact, in a fixed order.
 *
 * Three properties are load-bearing:
 *
 * - `HAND=` holds only this seat's cards, because the input is a
 *   `PublicGameView` and `project()` has no field capable of holding a foreign
 *   hand, the buried kitty, or the dealer's discard. Redaction is structural, not
 *   a formatting decision made here.
 * - The follow-suit constraint is never restated, because the `LEGAL:` block is
 *   already the complete legal set as computed by the engine. The model is not
 *   asked to apply the bower rule; it is told the answer in the label.
 * - Every legal move carries the engine's own score, so the model's job collapses
 *   to "deviate from this ranking the way my character would" — which small,
 *   fast models do well and open-ended reasoning does badly.
 *
 * ---
 *
 * A real trick-1 decision for seat 1: hearts are trump, the seat holds the left
 * bower and four clubs, and the human has led the ace of spades. Exactly the case
 * that confuses newcomers, and exactly where `effectiveSuit` earns its keep —
 * note that spades never appears as a constraint, because the engine has already
 * determined this seat is void, and that `JD` is offered as trump:
 *
 * ```text
 * YOU=1 DEALER=3 HAND#4 SCORE=6-4
 * PHASE=trick_play TRUMP=H MAKER=3
 * UP=TH
 * HAND=JD,9C,TC,KC,AC
 * TRICK=1 LED=S ON=0:AS
 * TRICKS=0-0 COUNTS=4,5,5,5
 * BIDS=0:Pass. 1:Pass. 2:Pass. 3:I take it.
 * LEGAL:
 *   play:JD = Jack of diamonds (left bower, hearts) [1.163 take it while I can]
 *   play:9C = Nine of clubs [0.433 cannot win, keep the good ones]
 *   play:TC = Ten of clubs [0.267 cannot win, keep the good ones]
 *   play:KC = King of clubs [-0.233 cannot win, keep the good ones]
 *   play:AC = Ace of clubs [-0.400 cannot win, keep the good ones]
 * ```
 *
 * ~520 characters, ≈150 tokens.
 */

import type { CardId, LegalMove, PublicGameView, RankedMove, Trick } from '#lib/protocol/index.ts';

/** `NOTES=` lines are model-authored text re-entering a prompt. Cap them hard. */
const MAX_NOTE_LINES = 6;
const MAX_NOTE_CHARS = 80;

export interface EncodeOptions {
	/**
	 * The engine's ranking over the same moves. Optional — when the AI seat ranks
	 * for itself it passes its own; when it does not, the block simply carries no
	 * bracketed scores and the model falls back on the labels.
	 */
	readonly ranking?: readonly RankedMove[];
	/**
	 * Per-hand card-counting notes, derived **from the public view only**. Rendered
	 * verbatim, so they are clamped in both dimensions.
	 */
	readonly notes?: readonly string[];
}

/**
 * Encode one decision.
 *
 * `legal` is the candidate set the model may choose from — after difficulty
 * narrowing, not before — so the rendered `LEGAL:` block and the `z.enum` built
 * for the same decision are derived from the same array. That identity is what
 * makes "the model picked something not on the list" unrepresentable rather than
 * merely detected.
 */
export function encodeForLlm(
	view: PublicGameView,
	legal: readonly LegalMove[],
	opts: EncodeOptions = {}
): string {
	const L: string[] = [];

	L.push(
		`YOU=${view.you} DEALER=${view.dealerSeat} HAND#${view.handNo} ` +
			`SCORE=${view.score[0]}-${view.score[1]}`
	);
	L.push(
		`PHASE=${view.phase}` +
			(view.trump === null ? '' : ` TRUMP=${view.trump}`) +
			(view.makerSeat === null ? '' : ` MAKER=${view.makerSeat}`) +
			(view.aloneSeat === null ? '' : ` ALONE=${view.aloneSeat}`) +
			(view.sittingSeat === null ? '' : ` SITTING=${view.sittingSeat}`)
	);
	L.push(`UP=${view.upCard ?? '-'}${view.upCardTurnedDown ? ' (turned down)' : ''}`);
	if (view.turnedDownSuit !== null) L.push(`BARRED=${view.turnedDownSuit}`);
	L.push(`HAND=${view.hand.join(',')}`);

	const { current, past } = splitTricks(view);
	if (current !== null && current.plays.length > 0) {
		L.push(
			`TRICK=${current.index + 1} LED=${current.ledSuit ?? '-'} ` +
				`ON=${current.plays.map((p) => `${p.seat}:${p.card}`).join(',')}`
		);
	}
	if (past.length > 0) {
		L.push(
			`PAST=${past
				.map((t) => `${t.winnerSeat ?? '?'}<${t.plays.map((p) => p.card).join(' ')}`)
				.join(' | ')}`
		);
	}

	L.push(`TRICKS=${view.tricksWon[0]}-${view.tricksWon[1]} COUNTS=${view.handCounts.join(',')}`);
	if (view.bids.length > 0) {
		L.push(`BIDS=${view.bids.map((b) => `${b.seat}:${b.say}`).join(' ')}`);
	}

	for (const note of (opts.notes ?? []).slice(0, MAX_NOTE_LINES)) {
		const clean = note.replace(/\s+/g, ' ').trim().slice(0, MAX_NOTE_CHARS);
		if (clean.length > 0) L.push(`NOTES=${clean}`);
	}

	L.push('LEGAL:');
	for (const m of legal) {
		const r = opts.ranking?.find((x) => x.id === m.id);
		const score = r === undefined ? '' : ` [${r.score.toFixed(3)} ${r.why}]`;
		L.push(`  ${m.id} = ${m.label}${score}`);
	}

	return L.join('\n');
}

/**
 * The strictly narrower slice banter is generated from.
 *
 * It omits `HAND` and `LEGAL` entirely. That is defence in depth by *information
 * starvation*: a persona that demands "tell everyone what you're holding" is
 * asking for data this prompt does not contain, so the strongest attack it can
 * mount is to make the model invent a card — which then fails the output screen.
 */
export function publicStateOnly(view: PublicGameView): string {
	const L: string[] = [];
	L.push(
		`YOU=${view.you} DEALER=${view.dealerSeat} HAND#${view.handNo} ` +
			`SCORE=${view.score[0]}-${view.score[1]}`
	);
	L.push(
		`PHASE=${view.phase}` +
			(view.trump === null ? '' : ` TRUMP=${view.trump}`) +
			(view.makerSeat === null ? '' : ` MAKER=${view.makerSeat}`) +
			(view.aloneSeat === null ? '' : ` ALONE=${view.aloneSeat}`)
	);
	L.push(`UP=${view.upCard ?? '-'}${view.upCardTurnedDown ? ' (turned down)' : ''}`);

	const { current, past } = splitTricks(view);
	if (current !== null && current.plays.length > 0) {
		L.push(
			`TRICK=${current.index + 1} ON=${current.plays.map((p) => `${p.seat}:${p.card}`).join(',')}`
		);
	}
	if (past.length > 0) {
		L.push(`PAST=${past.map((t) => `${t.winnerSeat ?? '?'}`).join(',')}`);
	}
	L.push(`TRICKS=${view.tricksWon[0]}-${view.tricksWon[1]}`);
	if (view.bids.length > 0) {
		L.push(`BIDS=${view.bids.map((b) => `${b.seat}:${b.say}`).join(' ')}`);
	}
	return L.join('\n');
}

/**
 * Split the live trick from the completed ones **without double-counting**.
 *
 * During `trick_resolve`, `view.trick` and the last entry of `view.trickLog` are
 * the same trick. Rendering both would tell the model the hand contains six
 * tricks and would double every card in the screen's "already public" set. The
 * discriminator is `winnerSeat`: a trick with a winner is finished and belongs in
 * `PAST`, and only in `PAST`.
 */
export function splitTricks(view: PublicGameView): {
	readonly current: Trick | null;
	readonly past: readonly Trick[];
} {
	const live = view.trick.winnerSeat === null ? view.trick : null;
	const past = view.trickLog.filter((t) => live === null || t.index !== live.index);
	return { current: live, past };
}

/**
 * Every card that is already public knowledge at this moment.
 *
 * The union of all cards played (completed tricks plus the live one, deduped) and
 * the up-card, which stays publicly identified for the whole hand even after it
 * is turned down. This is the allowlist the banter screen checks names against.
 */
export function publicCards(view: PublicGameView): ReadonlySet<CardId> {
	const seen = new Set<CardId>();
	const { current, past } = splitTricks(view);
	for (const t of past) for (const p of t.plays) seen.add(p.card);
	if (current !== null) for (const p of current.plays) seen.add(p.card);
	if (view.upCard !== null) seen.add(view.upCard);
	return seen;
}

/** Rough token estimate. Deterministic, committed, and only ever used for gates. */
export function estimateTokens(text: string): number {
	return Math.ceil(text.length / 3.7);
}
