/**
 * Invariant-message fixture generator for the `euchre-engine` Rust port.
 *
 * Builds real states with the TS engine, breaks one invariant in each, and
 * records the exact `InvariantError` message `assertInvariants` (or
 * `assertLegalNonEmpty`) throws. `tests/invariants.rs` checks the Rust port
 * produces the same message for the same state.
 *
 * Usage (from the repo root):  bun run crates/euchre-engine/scripts/invariants.ts
 *
 * Override the TS source location with EUCHRE_TS_SRC (default ~/goods/euchre).
 */

import { mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

const SRC = process.env.EUCHRE_TS_SRC ?? join(homedir(), 'goods/euchre');
const E = await import(join(SRC, 'src/lib/euchre/index.ts'));

const OUT = join(dirname(new URL(import.meta.url).pathname), '..', 'tests', 'fixtures');

const clone = (s: any) => structuredClone(s);
const step = (s: any, move: any) => E.apply(s, s.hand.turnSeat, move).state;

function opened(seed: string): any {
	return E.advance(E.createGame({ gameId: `g-${seed}`, seed, firstDealer: 0 })).state;
}

const cutting = opened('inv');
const bidding = step(cutting, { t: 'cut', cut: false });
const alone = step(bidding, { t: 'orderUp', alone: true }); // maker 1, sitting 3, dealer discard
const discarded = step(alone, { t: 'discard', card: alone.hand.hands[0][0] });
const played = step(discarded, { t: 'play', card: E.legalMoves(discarded, 1)[0].move.card });

/** A finished hand at `hand_score`, and one trick resolved mid-hand. */
function playUntil(s: any, pred: (s: any) => boolean): any {
	let state = s;
	while (!pred(state)) {
		const seat = state.hand.turnSeat;
		state =
			seat === null ? E.advance(state).state : E.apply(state, seat, E.legalMoves(state, seat)[0].move).state;
	}
	return state;
}
const twoTricks = playUntil(
	discarded,
	(s) => s.hand.trickLog.length === 2 && s.hand.phase === 'trick_play'
);
const scored = playUntil(discarded, (s) => s.hand.phase === 'hand_score');

interface Case {
	label: string;
	state: any;
	legal?: boolean;
}

const cases: Case[] = [];
const add = (label: string, base: any, patch: (s: any) => void, legal = false) => {
	const s = clone(base);
	patch(s);
	cases.push({ label, state: s, legal });
};

add('valid cutting', cutting, () => {});
add('valid trick play', played, () => {});
add('valid hand score', scored, () => {});
add('V4 alone != maker', played, (s) => (s.hand.makerSeat = 2));
add('V4 sitting != partner', played, (s) => (s.hand.sittingSeat = 0));
add('V4 alone without trump', played, (s) => (s.hand.trump = null));
add('V4 sitting without alone', bidding, (s) => (s.hand.sittingSeat = 2));
add('V5 dealer holds six', discarded, (s) => {
	s.hand.hands[0] = [...s.hand.hands[0], s.hand.kitty[1]];
});
add('V8 sitting seat played', played, (s) => {
	s.hand.trick.plays = [...s.hand.trick.plays, { seat: 3, card: s.hand.hands[3][0] }];
	s.hand.hands[3] = s.hand.hands[3].slice(1);
});
add('V9 wrong leader', twoTricks, (s) => {
	const t = s.hand.trickLog[1];
	s.hand.trickLog[1] = { ...t, plays: [...t.plays.slice(1), t.plays[0]] };
});
add('V9 sitting seat leads the next trick (V8 fires first)', twoTricks, (s) => {
	// The V9 "sitting seat won a trick" branch is unreachable: it needs the
	// sitting seat to lead the next trick, and V8 rejects that play first.
	const [t0, t1] = s.hand.trickLog;
	s.hand.trickLog[0] = { ...t0, winnerSeat: 3 };
	s.hand.trickLog[1] = {
		...t1,
		plays: [{ ...t1.plays[0], seat: 3 }, ...t1.plays.slice(1)]
	};
});
add('V10 short deck', bidding, (s) => (s.hand.deckOrder = s.hand.deckOrder.slice(1)));
add('V10 duplicate deck', bidding, (s) => {
	s.hand.deckOrder = [s.hand.deckOrder[1], ...s.hand.deckOrder.slice(1)];
});
add('V10 lost card', bidding, (s) => (s.hand.hands[2] = s.hand.hands[2].slice(1)));
add('V10 live duplicate', bidding, (s) => {
	s.hand.hands[2] = [...s.hand.hands[2], s.hand.hands[1][0]];
});
add('V10 played count', scored, (s) => {
	const last = s.hand.trickLog[4];
	s.hand.trickLog[4] = { ...last, plays: last.plays.slice(0, 2) };
	s.hand.trick = s.hand.trickLog[4];
	s.hand.hands[0] = [...s.hand.hands[0], ...last.plays.slice(2).map((p: any) => p.card)];
});
add('V11 score over gameTo', bidding, (s) => (s.score = [11, 0]));
add('V11 both teams', scored, (s) => (s.hand.delta = [1, 2]));
add('V11 illegal award', scored, (s) => {
	s.hand.delta = [3, 0];
	s.hand.result = 'march';
});
add('V11 clamped loner ok', scored, (s) => {
	s.hand.delta = [3, 0];
	s.hand.result = 'lone_march';
	s.score = [10, 0];
	s.winnerTeam = 0;
});
add('V14 null up-card', bidding, (s) => (s.hand.upCard = null));
add('V14 up-card mismatch', bidding, (s) => (s.hand.upCard = s.hand.kitty[1]));
add('V17 tie', cutting, (s) => {
	s.hand.trump = 'H';
	s.hand.trick = {
		index: 0,
		ledSuit: 'H',
		plays: [
			{ seat: 0, card: 'AH' },
			{ seat: 1, card: 'AH' },
			{ seat: 2, card: '9C' }
		],
		winnerSeat: null
	};
});
add('V18 passes', cutting, (s) => (s.hand.passes = 5));
add('V18 trick plays', cutting, (s) => {
	s.hand.trick = {
		index: 0,
		ledSuit: null,
		plays: [0, 1, 2, 3, 0].map((seat, i) => ({ seat, card: ['9S', 'TS', 'QS', 'KS', 'AS'][i] })),
		winnerSeat: null
	};
});
add('V18 trickLog length', scored, (s) => {
	s.hand.trickLog = [...s.hand.trickLog, { index: 5, ledSuit: null, plays: [], winnerSeat: null }];
});
add('V18 bids length', alone, (s) => {
	s.hand.bids = Array.from({ length: 9 }, () => s.hand.bids[0]);
});
add('V19 empty legal set', played, (s) => (s.hand.hands[s.hand.turnSeat] = []), true);
add('V19 sitting turn is skipped', played, (s) => (s.hand.turnSeat = 3), true);

const out = cases.map(({ label, state, legal }) => {
	let message: string | null = null;
	try {
		if (legal) E.assertLegalNonEmpty(state, E.legalMoves);
		else E.assertInvariants(state);
	} catch (e) {
		message = (e as Error).message;
	}
	return { label, legal: legal ?? false, message, state };
});

mkdirSync(OUT, { recursive: true });
const body = JSON.stringify({ generatedBy: 'scripts/invariants.ts', cases: out });
writeFileSync(join(OUT, 'invariants.json'), body + '\n');
for (const c of out) console.log(`${c.label}: ${c.message}`);
console.log(`invariants.json: ${out.length} cases, ${body.length} bytes`);
