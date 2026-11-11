/**
 * Converts the TS seat-visible regression positions (`scripts/fixtures/*.ts`)
 * into JSON for the Rust tests, with a TS parity record per case.
 *
 * Output (`crates/euchre-ai/tests/fixtures/`):
 *   bidding.json             10 `biddingCases`
 *   bidding-acceptance.json  15 `acceptanceBiddingCases`
 *   partnership.json          7 `partnershipCases`
 *   strategy.json             6 `strategyCases`
 *
 * Each case is `{ name, view, expected, rank, jev }`: `rank` is
 * `id|String(score)|why;` per move of `rankMoves(view)` followed by
 * `top=<topMove>`, and `jev` is the full Jev request body for
 * `buildDecisionQuestion(view, view.legal, Rita)`, so the Rust side checks both
 * byte for byte. `playView` and `bidView` themselves are ported to
 * `tests/common/mod.rs`.
 *
 * Usage (from the repo root): bun run crates/euchre-ai/scripts/fixtures.ts
 * Override the TS source location with EUCHRE_TS_SRC (default ~/goods/euchre).
 */

import { mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

const SRC = process.env.EUCHRE_TS_SRC ?? join(homedir(), 'goods/euchre');
const H = await import(join(SRC, 'src/lib/ai/heuristic.ts'));
const DC = await import(join(SRC, 'src/lib/ai/decision-context.ts'));
const J = await import(join(SRC, 'src/lib/ai/jev.ts'));
const P = await import(join(SRC, 'src/lib/server/personas.ts'));
const bidding = await import(join(SRC, 'scripts/fixtures/bidding.ts'));
const acceptance = await import(join(SRC, 'scripts/fixtures/bidding-acceptance.ts'));
const partnership = await import(join(SRC, 'scripts/fixtures/partnership.ts'));
const strategy = await import(join(SRC, 'scripts/fixtures/strategy.ts'));

const OUT = join(dirname(new URL(import.meta.url).pathname), '..', 'tests', 'fixtures');
const RITA = P.defaultPersonas()[0].persona;

function rank(view: any): string {
	const ranked = H.rankMoves(view);
	return `${ranked.map((r: any) => `${r.id}|${String(r.score)}|${r.why};`).join('')}top=${H.topMove(view)}`;
}

function jev(view: any): string {
	const q = DC.buildDecisionQuestion(view, view.legal, RITA);
	return JSON.stringify({
		model: J.JEV_MODEL,
		state: q.state,
		questions: {
			...q.additionalQuestions,
			move: { type: 'choice', instructions: q.instructions, criteria: q.criteria }
		}
	});
}

/**
 * The hand-built fixture views list a trick's keys in a different order
 * (`winnerSeat` before `plays`) from the engine projection the server builds.
 * The Jev body passes `trick` and `trickLog` through verbatim, so put them in projection order
 * first: the typed Rust view always serializes that order.
 */
function projectionOrder(view: any): any {
	const trick = (t: any) =>
		t && { index: t.index, ledSuit: t.ledSuit, plays: t.plays, winnerSeat: t.winnerSeat };
	const out = { ...view };
	if ('trick' in view) out.trick = trick(view.trick);
	if (Array.isArray(view.trickLog)) out.trickLog = view.trickLog.map(trick);
	return out;
}

function write(file: string, cases: readonly any[]): void {
	const body = cases.map((c) => {
		const view = projectionOrder(c.view);
		return { name: c.name, view, expected: c.expected, rank: rank(view), jev: jev(view) };
	});
	writeFileSync(join(OUT, file), JSON.stringify(body, null, '\t') + '\n');
	console.log(`${file}: ${body.length} cases`);
}

mkdirSync(OUT, { recursive: true });
write('bidding.json', bidding.biddingCases);
write('bidding-acceptance.json', acceptance.acceptanceBiddingCases);
write('partnership.json', partnership.partnershipCases);
write('strategy.json', strategy.strategyCases);
