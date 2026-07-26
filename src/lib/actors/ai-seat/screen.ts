/**
 * `aiSeat/screen` — the output screen and the untrusted-text sanitiser.
 *
 * Two directions of defence live here, and both are pure functions of their
 * arguments: no clock, no randomness, no I/O, no state.
 *
 * **Inbound.** {@link sanitizePersona} is applied to the user's editable persona
 * prompt, the shared house prompt and the LLM-authored dossier before any of them
 * is fenced into a system message. The SvelteKit form action clamps the same text,
 * but the form action is not the trust boundary — this is, and it runs again here
 * whatever the form did.
 *
 * **Outbound.** {@link screenBanter} is the last thing that touches a model-authored
 * line before it reaches a human. A persona is free to instruct its seat to
 * announce its holdings; the seat's *prompt* does not contain them (banter is
 * generated from a hand-stripped view), and if a line names a card anyway it is
 * rejected here rather than repaired. Rejection is terminal — the caller ships a
 * phrasebook line instead. Regeneration would cost latency the tempo budget does
 * not have, and would hand a prompt-injection a retry loop.
 *
 * `AIDecision.rationale` is a leak surface too: it is journalled and shown in the
 * post-game "why did it do that" panel, so it passes the same screen via
 * {@link cleanRationale}.
 *
 * Normative source: `docs/03-AI-AGENTS.md` §5, §6.
 */

import { BANTER_MAX_CHARS, RATIONALE_MAX_CHARS } from '$lib/protocol';
import type { CardId, PublicGameView, Rank, Suit } from '$lib/protocol';

/** C0/C1 control characters — the classic prompt-smuggling channel. */
const CTRL = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F]/g;

/** Anything that looks like markup. Models with thinking disabled sometimes leak tags. */
const MARKUP = /<[^>]*>/g;

const RANK_WORD: Readonly<Record<string, Rank>> = Object.freeze({
	nine: '9',
	ten: 'T',
	jack: 'J',
	queen: 'Q',
	king: 'K',
	ace: 'A'
});

const SUIT_WORD: Readonly<Record<string, Suit>> = Object.freeze({
	spade: 'S',
	spades: 'S',
	heart: 'H',
	hearts: 'H',
	diamond: 'D',
	diamonds: 'D',
	club: 'C',
	clubs: 'C'
});

const SUIT_GLYPH: Readonly<Record<string, Suit>> = Object.freeze({
	'♠': 'S',
	'♥': 'H',
	'♦': 'D',
	'♣': 'C'
});

/** `ace of spades`, `nine of hearts`, `jack diamonds`. */
const WORD_CARD = /\b(nine|ten|jack|queen|king|ace)\s+(?:of\s+)?(spades?|hearts?|diamonds?|clubs?)\b/gi;

/** `A♠`, `ace ♥`, `9 ♦`. */
const GLYPH_CARD = /\b(nine|ten|jack|queen|king|ace|[9TJQKA])\s?([♠♥♦♣])/gi;

/** The engine's own two-character card ids: `JS`, `9C`, `TD`. */
const CODE_CARD = /\b([9TJQKA])([SHDC])\b/g;

/**
 * Every card the given text names, as engine card ids. Unrecognised text yields
 * an empty set; the function never throws.
 */
export function cardsNamedIn(text: string): Set<CardId> {
	const found = new Set<CardId>();
	const push = (rank: Rank | undefined, suit: Suit | undefined): void => {
		if (rank === undefined || suit === undefined) return;
		found.add(`${rank}${suit}` as CardId);
	};
	const rankOfToken = (t: string): Rank | undefined => {
		const lower = t.toLowerCase();
		if (lower in RANK_WORD) return RANK_WORD[lower];
		const upper = t.toUpperCase();
		return upper === '9' || upper === 'T' || upper === 'J' || upper === 'Q' || upper === 'K' || upper === 'A'
			? (upper as Rank)
			: undefined;
	};

	for (const m of text.matchAll(WORD_CARD)) {
		push(rankOfToken(m[1] ?? ''), SUIT_WORD[(m[2] ?? '').toLowerCase()]);
	}
	for (const m of text.matchAll(GLYPH_CARD)) {
		push(rankOfToken(m[1] ?? ''), SUIT_GLYPH[m[2] ?? '']);
	}
	for (const m of text.matchAll(CODE_CARD)) {
		push(rankOfToken(m[1] ?? ''), suitOfCode(m[2] ?? ''));
	}
	return found;
}

function suitOfCode(c: string): Suit | undefined {
	return c === 'S' || c === 'H' || c === 'D' || c === 'C' ? c : undefined;
}

/**
 * The cards a speaker is allowed to name: everything already face up on the table
 * this hand, plus the up-card, which every seat saw and which stays publicly
 * identified for the whole hand (V14).
 *
 * Derived from the redacted view alone, so the allow-list can never itself be a
 * leak. `view.hand` is deliberately **not** included: a seat naming its own
 * unplayed card is exactly the failure this screen exists to catch.
 */
export function publiclyNamedCards(view: PublicGameView): Set<CardId> {
	const allowed = new Set<CardId>();
	if (view.upCard !== null) allowed.add(view.upCard);
	const live = view.trick.index;
	for (const t of view.trickLog) {
		if (t.index === live) continue; // same trick as `view.trick` during trick_resolve
		for (const p of t.plays) allowed.add(p.card);
	}
	for (const p of view.trick.plays) allowed.add(p.card);
	return allowed;
}

/** Strip control characters and markup, collapse whitespace, trim. */
function scrub(raw: string): string {
	return raw.replace(CTRL, '').replace(MARKUP, '').replace(/\s+/g, ' ').trim();
}

/**
 * Screen a model-authored table-talk line.
 *
 * Returns the cleaned line, or `null` when the line must not be shown. A `null`
 * is terminal: replace it with a phrasebook line, never regenerate.
 *
 * @param raw     The accumulated buffer (screened at every flush *and* at final).
 * @param allowed Cards it is legitimate to name — see {@link publiclyNamedCards}.
 */
export function screenBanter(raw: string, allowed: ReadonlySet<CardId>): string | null {
	if (typeof raw !== 'string') return null;
	const text = scrub(raw);
	if (text.length === 0) return null;
	for (const card of cardsNamedIn(text)) {
		if (!allowed.has(card)) return null;
	}
	return text.length > BANTER_MAX_CHARS ? text.slice(0, BANTER_MAX_CHARS).trimEnd() : text;
}

/**
 * Screen a decision rationale. Same rules as banter, but it falls back to the
 * engine's own `RankedMove.why` instead of being dropped, because the post-game
 * panel should still be able to explain the move.
 */
export function cleanRationale(
	raw: string | undefined,
	fallback: string,
	allowed: ReadonlySet<CardId>
): string {
	const fb = scrub(fallback).slice(0, RATIONALE_MAX_CHARS);
	if (typeof raw !== 'string') return fb;
	const text = scrub(raw);
	if (text.length === 0) return fb;
	for (const card of cardsNamedIn(text)) {
		if (!allowed.has(card)) return fb;
	}
	return text.slice(0, RATIONALE_MAX_CHARS);
}

/**
 * Sanitise untrusted persona text before it is fenced into a system message.
 *
 * Strips any literal occurrence of the fence nonce (the fence-escape defence),
 * strips control characters, collapses newline runs, and hard-clamps the length.
 * The nonce is per-game and unguessable, so a persona cannot close the fence it is
 * inside; stripping literal matches closes the remaining hole where a persona is
 * edited *during* a match by someone who has seen the nonce.
 */
export function sanitizePersona(raw: string, nonce: string, cap: number): string {
	if (typeof raw !== 'string') return '';
	let out = raw;
	if (nonce.length > 0) out = out.split(nonce).join('');
	return out
		.replace(CTRL, '')
		.replace(/\n{3,}/g, '\n\n')
		.slice(0, cap)
		.trim();
}

/**
 * The view with this seat's own cards removed.
 *
 * Banter is a **separate generation on public state only** — folding a quip into
 * the decision call is forbidden, because that prompt contains the hand. Passing
 * this instead of `view` makes "never describe your own holdings" structural: the
 * banter prompt does not contain them to describe.
 */
export function publicOnlyView(view: PublicGameView): PublicGameView {
	return { ...view, hand: [], legal: [] };
}
