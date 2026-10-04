/**
 * Table talk.
 *
 * Banter is a **separate generation on public state only**, and that separation is
 * a security control rather than a code-organisation preference. Folding the quip
 * into the decision call is forbidden: that prompt contains the seat's hand, and
 * an LLM asked to be charming while looking at five cards will eventually mention
 * one of them. Here the prompt physically does not contain the hand
 * (`publicStateOnly` omits `HAND` and `LEGAL`), so the worst a hostile persona can
 * extract is an invention — which then fails the output screen.
 *
 * Three gates, in order:
 *
 * 1. **Salience.** A line is attempted only when the moment clears
 *    `1 - persona.chattiness`, or when the caller forces it (a declared loner, a
 *    euchre, a march, trump called, 9-9). A silent persona costs nothing.
 * 2. **Budget.** One short streamed call with its own abort. Fire-and-forget from
 *    the caller's perspective — banter must never be on the critical path of a
 *    move, and this function never throws.
 * 3. **Screen.** `screenBanter` runs on the accumulated buffer at every flush
 *    *and* at final. A rejected line is replaced by a phrasebook line, not
 *    regenerated: regeneration costs latency the tempo budget does not have, and
 *    it hands an adversarial persona a retry loop.
 */

import { streamText } from 'ai';
import type { PublicGameView } from '#lib/protocol/index.ts';
import { resolveBudget } from './config';
import { modelParams } from './model';
import { publicStateOnly } from './notation';
import { buildLayers, toInstructions } from './prompt';
import { phrasebookLine, screenBanter } from './screen';
import { ZERO_USAGE, noopLog, type DecideDeps, type DecideUsage } from './types';

export interface BanterRequest {
	/** The redacted view. Only its public slice is rendered into the prompt. */
	readonly view: PublicGameView;
	/** A phrasebook key and a prompt hint: `bid`, `loner`, `euchre`, `march`, … */
	readonly situation: string;
	/** `0..1`. Compared against `1 - persona.chattiness`. */
	readonly salience: number;
	/** Bypass the salience gate for a moment that always deserves a line. */
	readonly force?: boolean;
	/** Deterministic phrasebook index. A seeded counter, never `Math.random()`. */
	readonly seq: number;
	/** Called with each raw delta. Batching for the wire is the table's job. */
	readonly onDelta?: (delta: string) => void;
	readonly signal?: AbortSignal;
}

export type BanterOutcome =
	| { readonly spoke: false; readonly reason: 'not_salient' }
	| {
			readonly spoke: true;
			readonly text: string;
			readonly source: 'llm' | 'phrasebook';
			/** Set when a generated line was rejected; the reason it was rejected. */
			readonly screenedReason?: string;
			readonly usage: DecideUsage;
	  };

/**
 * Generate one line, or fall back to the phrasebook.
 *
 * Never throws and never rejects: every failure path — no key, timeout, provider
 * error, screened line — resolves to a phrasebook line. A quip is not worth an
 * unhandled rejection inside an actor's workflow step.
 */
export async function generateBanter(deps: DecideDeps, req: BanterRequest): Promise<BanterOutcome> {
	const log = deps.log ?? noopLog;
	const budget = resolveBudget(deps.budget);
	const chattiness = clamp01(deps.persona.chattiness);

	if (req.force !== true && req.salience < 1 - chattiness) {
		log('banter_skipped', { situation: req.situation, salience: req.salience });
		return { spoke: false, reason: 'not_salient' };
	}

	const canned = (reason: string | undefined): BanterOutcome => ({
		spoke: true,
		text: phrasebookLine(req.situation, req.seq),
		source: 'phrasebook',
		...(reason === undefined ? {} : { screenedReason: reason }),
		usage: ZERO_USAGE
	});

	if (deps.factory === null) {
		log('ai_no_provider', { where: 'banter' });
		return canned(undefined);
	}

	const modelId = deps.persona.modelId;
	const layers = buildLayers({
		kind: 'play',
		persona: deps.persona,
		dossier: deps.dossier,
		nonce: deps.nonce,
		seat: req.view.you,
		body: `${publicStateOnly(req.view)}\nSITUATION=${req.situation}`,
		...(deps.log === undefined ? {} : { log: deps.log })
	});

	const abort = new AbortController();
	const timeout = AbortSignal.timeout(budget.banterMs);
	const signals = [abort.signal, timeout, ...(req.signal === undefined ? [] : [req.signal])];
	const signal = AbortSignal.any(signals);

	let buffer = '';
	let usage: DecideUsage = ZERO_USAGE;

	try {
		const res = streamText({
			model: deps.factory.talk(modelId),
			instructions: [
				...toInstructions(layers),
				{
					role: 'system' as const,
					content:
						'One line of table talk, at most 90 characters. Public events only. ' +
						'Never name a card, never describe your own holdings, never hint at what ' +
						'you hold, never speculate about another player’s cards. No markup.'
				}
			],
			messages: [{ role: 'user', content: layers.user }],
			maxOutputTokens: budget.maxOutputTokensBanter,
			maxRetries: 0,
			abortSignal: signal,
			...modelParams(
				deps.factory?.slugFor(modelId) ?? modelId,
				deps.persona.temperature,
				deps.factory?.provider
			),
			onError: ({ error }) => log('banter_error', { e: String(error) })
		});

		for await (const delta of res.textStream) {
			buffer += delta;
			// Screen the *settled* prefix on every flush. The trailing partial word is
			// held back because a half-streamed token can transiently look like a card
			// code ("AS" inside "ASTONISHING") and a false reject costs a real line.
			const settled = buffer.slice(0, buffer.lastIndexOf(' ') + 1);
			if (settled.length > 0) {
				const mid = screenBanter(settled, req.view);
				if (!mid.ok) {
					abort.abort();
					log('banter_screened', {
						persona: deps.persona.id,
						reason: mid.reason,
						where: 'stream'
					});
					return canned(mid.reason);
				}
			}
			req.onDelta?.(delta);
		}

		usage = await res.usage.then(
			(u) => ({
				inputTokens: u.inputTokens ?? 0,
				outputTokens: u.outputTokens ?? 0,
				cacheReadTokens: u.inputTokenDetails.cacheReadTokens ?? 0,
				cacheWriteTokens: u.inputTokenDetails.cacheWriteTokens ?? 0
			}),
			() => ZERO_USAGE
		);
	} catch (e) {
		log('banter_error', { e: e instanceof Error ? e.name : String(e) });
		return canned(undefined);
	}

	const final = screenBanter(buffer, req.view);
	if (!final.ok) {
		log('banter_screened', { persona: deps.persona.id, reason: final.reason, where: 'final' });
		// The tokens were still spent, so the caller still meters them.
		return {
			spoke: true,
			text: phrasebookLine(req.situation, req.seq),
			source: 'phrasebook',
			screenedReason: final.reason,
			usage
		};
	}

	return { spoke: true, text: final.text, source: 'llm', usage };
}

function clamp01(v: number): number {
	return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
}
