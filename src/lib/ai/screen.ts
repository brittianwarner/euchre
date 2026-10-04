/**
 * Output screening — the last line of defence.
 *
 * Two kinds of generated free text reach a human: banter on the chat channel, and
 * the decision `rationale` shown in the post-game "why did it do that" panel. Both
 * are written by a model that has just been shown a hand, so both are leak
 * surfaces, and both pass through here.
 *
 * The rule: **a line may not name a specific card unless that card is already
 * public.** Public means played to a completed or in-progress trick, or the
 * up-card, which stays publicly identified for the whole hand even after it is
 * turned down. Naming a *suit* alone is fine — trump is public and "I like
 * hearts" reveals nothing.
 *
 * A rejected line is **not regenerated**. Regeneration costs latency the tempo
 * budget does not have and gives an adversarial persona a retry loop; instead a
 * phrasebook line ships and the rejection is logged with the persona id.
 *
 * The screen is intentionally trigger-happy. A false positive costs one canned
 * quip. A false negative is an opponent telling the human what it holds.
 */

import {
	BANTER_MAX_CHARS,
	RATIONALE_MAX_CHARS,
	type CardId,
	type PublicGameView
} from '#lib/protocol/index.ts';
import { publicCards } from './notation';

/* ========================================================================== */
/* Card-name detection                                                        */
/* ========================================================================== */

const RANK_WORDS: Readonly<Record<string, string>> = {
	nine: '9',
	ten: 'T',
	jack: 'J',
	queen: 'Q',
	king: 'K',
	ace: 'A'
};

const SUIT_WORDS: Readonly<Record<string, string>> = {
	spade: 'S',
	spades: 'S',
	heart: 'H',
	hearts: 'H',
	diamond: 'D',
	diamonds: 'D',
	club: 'C',
	clubs: 'C'
};

const SUIT_GLYPHS: Readonly<Record<string, string>> = {
	'♠': 'S',
	'♥': 'H',
	'♦': 'D',
	'♣': 'C'
};

/** `AS`, `9C`, `JD` — the engine's own two-character code, upper case only. */
const CODE_RE = /\b([9TJQKA])([SHDC])\b/g;

/** "ace of spades", "nine of clubs", and the lazy "jack diamonds". */
const WORDS_RE =
	/\b(nine|ten|jack|queen|king|ace)s?\b(?:\s+of)?\s+\b(spades?|hearts?|diamonds?|clubs?)\b/gi;

/** "A♠", "ace ♥", "J of ♦". */
const GLYPH_RE = /\b(nine|ten|jack|queen|king|ace|[9TJQKA])\b\s*(?:of\s*)?([♠♥♦♣])/gi;

/**
 * "the right bower" / "the left bower".
 *
 * Once trump is known — and it always is by the time anyone is talking about
 * bowers — "the right bower" names exactly one card. A seat that says "I've still
 * got the right bower" has told the table its holding as precisely as if it had
 * said `JS`. Treated as a card reference and screened against the same allowlist.
 */
const BOWER_RE = /\b(right|left)\s+bower\b/gi;

/** Markup of any kind, including leaked `<thinking>` from a thinking-disabled model. */
const MARKUP_RE = /<[^>]*>/g;

const CTRL_RE = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F]/g;

const BLACK: Readonly<Record<string, string>> = { S: 'C', C: 'S', H: 'D', D: 'H' };

/**
 * Every card name the text refers to, normalised to `CardId`.
 *
 * Bower references resolve through `trump`; if trump is not yet known they resolve
 * to `null`, which is treated as "unresolvable card reference" and rejected — a
 * seat talking about bowers before trump exists is either confused or fishing.
 */
export function cardsNamedIn(
	text: string,
	trump: string | null
): {
	readonly cards: readonly CardId[];
	readonly unresolvedBower: boolean;
} {
	const cards: CardId[] = [];
	let unresolvedBower = false;

	for (const m of text.matchAll(CODE_RE)) cards.push(`${m[1]}${m[2]}` as CardId);

	for (const m of text.matchAll(WORDS_RE)) {
		const rank = RANK_WORDS[m[1].toLowerCase()];
		const suit = SUIT_WORDS[m[2].toLowerCase()];
		if (rank !== undefined && suit !== undefined) cards.push(`${rank}${suit}` as CardId);
	}

	for (const m of text.matchAll(GLYPH_RE)) {
		const raw = m[1].toLowerCase();
		const rank = RANK_WORDS[raw] ?? (raw.length === 1 ? raw.toUpperCase() : undefined);
		const suit = SUIT_GLYPHS[m[2]];
		if (rank !== undefined && suit !== undefined) cards.push(`${rank}${suit}` as CardId);
	}

	for (const m of text.matchAll(BOWER_RE)) {
		if (trump === null) {
			unresolvedBower = true;
			continue;
		}
		const suit = m[1].toLowerCase() === 'right' ? trump : BLACK[trump];
		if (suit === undefined) unresolvedBower = true;
		else cards.push(`J${suit}` as CardId);
	}

	return { cards, unresolvedBower };
}

/* ========================================================================== */
/* The screen                                                                 */
/* ========================================================================== */

export type ScreenReject =
	'private_card' | 'unresolved_bower' | 'markup' | 'empty' | 'too_long_after_clamp';

export type ScreenResult =
	| { readonly ok: true; readonly text: string }
	| { readonly ok: false; readonly reason: ScreenReject; readonly offending?: string };

export interface ScreenOptions {
	readonly maxChars: number;
	/**
	 * Cards that may be named. Defaults to `publicCards(view)`. Passing a wider set
	 * is a deliberate act and should be justified at the call site — there is no
	 * legitimate reason for a *speaker's own hand* to be in here.
	 */
	readonly allow?: ReadonlySet<CardId>;
}

/**
 * Screen one line of generated text against a public view.
 *
 * Order: strip markup and control characters, collapse whitespace, clamp, then
 * check card references on the **clamped** text — because that is the text that
 * ships. Screening before the clamp would let a half-sentence reveal survive.
 */
export function screenText(raw: string, view: PublicGameView, opts: ScreenOptions): ScreenResult {
	const hadMarkup = MARKUP_RE.test(raw);
	MARKUP_RE.lastIndex = 0;

	const text = raw
		.replace(MARKUP_RE, ' ')
		.replace(CTRL_RE, '')
		.replace(/\s+/g, ' ')
		.trim()
		.slice(0, opts.maxChars)
		.trim();

	if (text.length === 0) return { ok: false, reason: hadMarkup ? 'markup' : 'empty' };

	const allow = opts.allow ?? publicCards(view);
	const { cards, unresolvedBower } = cardsNamedIn(text, view.trump);
	if (unresolvedBower) return { ok: false, reason: 'unresolved_bower' };

	for (const c of cards) {
		if (!allow.has(c)) return { ok: false, reason: 'private_card', offending: c };
	}

	return { ok: true, text };
}

/** Banter: public state only, clamped to {@link BANTER_MAX_CHARS}. */
export function screenBanter(raw: string, view: PublicGameView): ScreenResult {
	return screenText(raw, view, { maxChars: BANTER_MAX_CHARS });
}

/**
 * The decision rationale.
 *
 * It reaches the human in the post-game panel, so it gets the same screen as
 * banter — the spec is explicit that `why` is a leak surface too. On rejection it
 * falls back to the engine's own `RankedMove.why`, which is authored by us and is
 * safe by construction. Never used for control flow.
 */
export function cleanRationale(raw: string, fallback: string, view: PublicGameView): string {
	const screened = screenText(raw, view, { maxChars: RATIONALE_MAX_CHARS });
	if (screened.ok) return screened.text;
	return fallback.replace(MARKUP_RE, ' ').replace(/\s+/g, ' ').trim().slice(0, RATIONALE_MAX_CHARS);
}

/* ========================================================================== */
/* The phrasebook                                                             */
/* ========================================================================== */

/**
 * Canned lines, used when a generated one is rejected or when there is no model.
 *
 * Every entry is card-free by construction, so it needs no screen. Selection is by
 * an index the caller supplies (a seeded counter), never `Math.random()` — a
 * replayed hand must produce the same table talk.
 */
export const PHRASEBOOK: Readonly<Record<string, readonly string[]>> = {
	generic: ['Hm.', 'All right then.', 'We go on.', 'Fair enough.', "That's the way of it."],
	bid: ['I like my chances.', 'Somebody has to.', "I'll take a look at it.", 'Worth a try.'],
	pass: ['Not from here.', 'Pass.', "It's not for me.", 'Someone else can have it.'],
	loner: ['On my own, then.', 'Stay out of it, partner.', "I'll handle this one."],
	win_trick: ['Mine.', 'That one travels.', "I'll take it."],
	lose_trick: ['Yours.', 'Take it.', 'Go on then.'],
	euchre: ['Well. That happened.', "Didn't hold up.", "We'll get the next one."],
	march: ['All five.', "That'll do nicely.", 'Clean sweep.'],
	game: ['Good game.', "That's the game.", 'Well played.']
};

/** Deterministic phrasebook selection. `n` is any non-negative integer. */
export function phrasebookLine(situation: string, n: number): string {
	const lines = PHRASEBOOK[situation] ?? PHRASEBOOK.generic;
	const safe = lines.length > 0 ? lines : PHRASEBOOK.generic;
	const i = Math.abs(Math.trunc(n)) % safe.length;
	return safe[i];
}
