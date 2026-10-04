/**
 * `aiSeat/memory` — card counting and cross-hand recall, both bounded.
 *
 * Everything here is derived from the **redacted view** and nothing else. There is
 * no argument on any function in this file capable of carrying a foreign hand, the
 * buried kitty or the dealer's discard, so "the AI counts only what a human in
 * that chair could count" is a property of the types rather than a rule someone
 * has to remember.
 *
 * Two stores, two lifetimes:
 *
 * - **Per hand** ({@link AiSeatHandMemory}) — trump seen, publicly shown voids,
 *   leads, the auction. Cleared by the table's `resetHand` message at every deal,
 *   and defensively re-cleared here if a view arrives for a hand number we have
 *   not seen (a lost `resetHand` must not poison the count).
 * - **Cross hand** (`state.notes`) — at most {@link MEM_NOTE_CAP} short lines about
 *   how the humans have been playing, each clamped to {@link NOTE_MAX_CHARS}.
 *   Bounded at ~1.1 KiB regardless of how long the match runs.
 *
 * A note is only written when the hand's outcome is **determined by what this seat
 * actually saw**. The `handEnd` message carries a hand number and nothing else, so
 * inventing an outcome would mean guessing; a hand that ended after this seat's
 * last view simply produces no note.
 *
 * Pure except for the deliberate in-place mutation of the memory object it is
 * handed — this runs inside the actor's serialized loop, on `c.state`.
 *
 * Normative source: `docs/03-AI-AGENTS.md` §10.
 */

import { effectiveSuit, partnerOf, teamOf } from '#lib/euchre/index.ts';
import { voidReads } from '#lib/ai/heuristic.ts';
import type { CardId, EpisodeKind, PublicGameView, Seat, Suit, Team } from '#lib/protocol/index.ts';
import {
	MEM_BID_CAP,
	MEM_LEAD_CAP,
	MEM_NOTE_CAP,
	MEM_TRUMP_SEEN_CAP,
	MEM_VOID_SUITS_CAP,
	NOTE_MAX_CHARS,
	type AiSeatEpisode,
	type AiSeatHandMemory,
	type HandSnapshot
} from './types';

/** Append with an explicit cap, dropping the oldest entries. Deduplicates. */
function pushCapped<T>(arr: T[], value: T, cap: number): void {
	if (arr.includes(value)) return;
	arr.push(value);
	while (arr.length > cap) arr.shift();
}

/** A fresh, empty per-hand memory. */
export function emptyMemory(handNo: number): AiSeatHandMemory {
	return {
		handNo,
		trumpSeen: [],
		voids: { 0: [], 1: [], 2: [], 3: [] },
		leadHistory: [],
		bidHistory: [],
		snapshot: null
	};
}

/** Clear the per-hand memory in place. Called by the `resetHand` consumer. */
export function resetMemory(memory: AiSeatHandMemory, handNo: number): void {
	const fresh = emptyMemory(handNo);
	memory.handNo = fresh.handNo;
	memory.trumpSeen = fresh.trumpSeen;
	memory.voids = fresh.voids;
	memory.leadHistory = fresh.leadHistory;
	memory.bidHistory = fresh.bidHistory;
	memory.snapshot = fresh.snapshot;
}

/** Every card face up this hand, deduping the live trick against its journal entry. */
function tableCards(view: PublicGameView): CardId[] {
	const out: CardId[] = [];
	const live = view.trick.index;
	for (const t of view.trickLog) {
		if (t.index === live) continue;
		for (const p of t.plays) out.push(p.card);
	}
	for (const p of view.trick.plays) out.push(p.card);
	return out;
}

/**
 * Fold one redacted view into the per-hand memory.
 *
 * Idempotent: absorbing the same view twice changes nothing, because every store
 * is a deduplicated capped set or a recomputed snapshot. That matters — a durable
 * queue may redeliver the same `decide` message.
 */
export function absorbPublicState(memory: AiSeatHandMemory, view: PublicGameView): void {
	if (memory.handNo !== view.handNo) resetMemory(memory, view.handNo);

	const trump = view.trump;

	// Trump census. Seeded with the up-card even after it is turned down —
	// precisely why the projection keeps a turned-down up-card identified (V14).
	if (trump !== null) {
		if (view.upCard !== null && effectiveSuit(view.upCard, trump) === trump) {
			pushCapped(memory.trumpSeen, view.upCard, MEM_TRUMP_SEEN_CAP);
		}
		for (const c of tableCards(view)) {
			if (effectiveSuit(c, trump) === trump) pushCapped(memory.trumpSeen, c, MEM_TRUMP_SEEN_CAP);
		}
	}

	// Publicly shown voids: every failure to follow the led suit, bower-aware.
	const reads = voidReads(view);
	for (const seat of [0, 1, 2, 3] as const) {
		for (const suit of reads[seat]) {
			pushCapped(memory.voids[seat], suit, MEM_VOID_SUITS_CAP);
		}
	}

	// Leads.
	const live = view.trick.index;
	for (const t of view.trickLog) {
		if (t.index === live) continue;
		const first = t.plays[0];
		if (first !== undefined) pushCapped(memory.leadHistory, first.card, MEM_LEAD_CAP);
	}
	const liveFirst = view.trick.plays[0];
	if (liveFirst !== undefined) pushCapped(memory.leadHistory, liveFirst.card, MEM_LEAD_CAP);

	// The auction.
	for (const b of view.bids) {
		pushCapped(memory.bidHistory, `${b.seat}:${b.say}`, MEM_BID_CAP);
	}

	memory.snapshot = {
		handNo: view.handNo,
		trump: view.trump,
		makerSeat: view.makerSeat,
		aloneSeat: view.aloneSeat,
		tricksWon: [view.tricksWon[0], view.tricksWon[1]],
		result: view.result
	};
}

/* ========================================================================== */
/* Cross-hand notes                                                            */
/* ========================================================================== */

/**
 * A one-line memory of how the last hand went, or `null` when this seat did not
 * see enough of it to know.
 *
 * Only two outcomes are *certain* from a partial view: a side that has already
 * taken three tricks has made the hand, and a maker's side that cannot reach three
 * has been euchred. Anything less definite is not written down, because a note
 * that later re-enters a prompt as fact must not be a guess.
 *
 * Seats are named by number rather than by persona name so a note can never carry
 * user-authored text back into a prompt.
 */
export function deriveHandNote(snapshot: HandSnapshot | null, seat: Seat): string | null {
	if (snapshot === null || snapshot.makerSeat === null) return null;

	const makerTeam: Team = teamOf(snapshot.makerSeat);
	const defenderTeam: Team = makerTeam === 0 ? 1 : 0;
	const makerTricks = snapshot.tricksWon[makerTeam] ?? 0;
	const defTricks = snapshot.tricksWon[defenderTeam] ?? 0;
	const alone = snapshot.aloneSeat;
	const mine: Team = teamOf(seat);
	const who = (s: Seat): string =>
		s === seat ? 'I' : s === partnerOf(seat) ? 'my partner' : `seat ${s}`;
	const hand = `h${snapshot.handNo}`;

	if (alone !== null) {
		if (defTricks >= 3) return clampNote(`${hand}: ${who(alone)} went alone and got euchred`);
		if (makerTricks === 5) return clampNote(`${hand}: ${who(alone)} went alone and marched it`);
		if (makerTricks >= 3) return clampNote(`${hand}: ${who(alone)} went alone and made it`);
		return null;
	}

	if (defTricks >= 3) {
		const verb = makerTeam === mine ? 'we got euchred' : 'we euchred them';
		return clampNote(`${hand}: ${who(snapshot.makerSeat)} called it and ${verb}`);
	}
	if (makerTricks === 5)
		return clampNote(`${hand}: ${who(snapshot.makerSeat)} called it and marched`);
	if (makerTricks >= 3)
		return clampNote(`${hand}: ${who(snapshot.makerSeat)} called it and made the point`);
	return null;
}

function clampNote(s: string): string {
	return s.length > NOTE_MAX_CHARS ? s.slice(0, NOTE_MAX_CHARS) : s;
}

/**
 * Record a cross-hand note, bounded at {@link MEM_NOTE_CAP} and written at most
 * once per hand. Returns the note when it was new, `null` otherwise.
 */
export function rememberHand(
	notes: string[],
	notedHands: number[],
	snapshot: HandSnapshot | null,
	seat: Seat
): string | null {
	if (snapshot === null) return null;
	if (notedHands.includes(snapshot.handNo)) return null;
	const note = deriveHandNote(snapshot, seat);
	notedHands.push(snapshot.handNo);
	while (notedHands.length > MEM_NOTE_CAP) notedHands.shift();
	if (note === null) return null;
	notes.push(note);
	while (notes.length > MEM_NOTE_CAP) notes.shift();
	return note;
}

/** Classify a note for the episode store. */
export function episodeKindFor(snapshot: HandSnapshot | null): EpisodeKind {
	if (snapshot === null) return 'play';
	if (snapshot.aloneSeat !== null) return 'loner';
	const makerSeat = snapshot.makerSeat;
	if (makerSeat === null) return 'play';
	const makerTeam: Team = teamOf(makerSeat);
	const defTricks = snapshot.tricksWon[makerTeam === 0 ? 1 : 0] ?? 0;
	return defTricks >= 3 ? 'euchre' : 'bid';
}

/**
 * Turn a finished hand into a buffered episode, or `null`. `ts` is supplied by the
 * caller so this stays clock-free and testable.
 */
export function episodeFor(
	snapshot: HandSnapshot | null,
	seat: Seat,
	ts: number
): AiSeatEpisode | null {
	const summary = deriveHandNote(snapshot, seat);
	if (summary === null || snapshot === null) return null;
	return {
		kind: episodeKindFor(snapshot),
		summary,
		salience: snapshot.aloneSeat !== null ? 0.9 : 0.5,
		handNo: snapshot.handNo,
		ts
	};
}

/* ========================================================================== */
/* Salience — how much a moment is worth talking about                         */
/* ========================================================================== */

/**
 * `0`–`1`. A banter line is attempted only when this clears `1 − chattiness`, and
 * a handful of moments force it on regardless of the dial: a declared loner, a
 * euchre, a march, trump being named, and 9–9.
 *
 * Computed from the public view only, so a persona cannot manufacture salience out
 * of information it should not have.
 */
export function momentSalience(view: PublicGameView, gameTo: number): number {
	let s = 0.2;

	if (view.aloneSeat !== null) s = Math.max(s, 1);
	if (view.result === 'euchre') s = Math.max(s, 0.95);
	if (view.result === 'march' || view.result === 'lone_march') s = Math.max(s, 0.9);
	if (view.phase === 'trick_resolve') s = Math.max(s, 0.35);

	// Trump was just named: the last bid in the log is a call, not a pass.
	const lastBid = view.bids[view.bids.length - 1];
	if (lastBid !== undefined && lastBid.move.t !== 'pass') s = Math.max(s, 0.6);

	// Both sides in the barn.
	const a = view.score[0] ?? 0;
	const b = view.score[1] ?? 0;
	if (a >= gameTo - 1 && b >= gameTo - 1) s = Math.max(s, 0.9);
	else if (a >= gameTo - 1 || b >= gameTo - 1) s = Math.max(s, 0.55);

	return s > 1 ? 1 : s;
}

/**
 * A short, card-free description of the moment for the banter prompt. Public facts
 * only — it is built from the same view the model is given.
 */
export function situationLine(view: PublicGameView, seat: Seat): string {
	const parts: string[] = [
		`seat ${seat}`,
		`hand ${view.handNo}`,
		`score ${view.score[0]}-${view.score[1]}`
	];
	if (view.trump !== null) parts.push(`trump ${view.trump}`);
	if (view.makerSeat !== null) parts.push(`maker seat ${view.makerSeat}`);
	if (view.aloneSeat !== null) parts.push(`seat ${view.aloneSeat} is alone`);
	parts.push(`tricks ${view.tricksWon[0]}-${view.tricksWon[1]}`);
	if (view.result !== null) parts.push(`result ${view.result}`);
	return parts.join(', ');
}

/** Suits this seat has publicly shown a void in. Convenience for prompt building. */
export function knownVoids(memory: AiSeatHandMemory, seat: Seat): readonly Suit[] {
	return memory.voids[seat] ?? [];
}
