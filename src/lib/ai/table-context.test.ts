import { describe, expect, it, vi } from 'vitest';
import { playView } from '../../../scripts/fixtures/partnership';
import { tableContext } from './table-context';
import { encodeForLlm } from './notation';
import { callModel } from './decide';
import { buildLayers } from './prompt';
import { defaultPersonas, personasFromSettings } from '../server/personas';
import { defaultSettings } from '../actors/player-profile/settings';
import type { ModelFactory } from './types';

const base = playView(
	['JD', '9H', 'AC'],
	[
		{ seat: 0, card: 'AS' },
		{ seat: 1, card: '9C' }
	]
);

describe('human table evidence', () => {
	it('counts the left bower as trump and records proved voids from attributed plays', () => {
		const context = tableContext(base);
		expect(context.cardsInEachEffectiveSuit).toMatchObject({ H: 2, D: 0, C: 1 });
		expect(context.publicReads.find((read) => read.seat === 1)).toMatchObject({
			provedVoids: ['S'],
			cardsPlayed: ['9C']
		});
		expect(context.unseenTrump).not.toContain('JD');
		expect(context.unseenTrump).toContain('JH');
	});
	it('keeps buried trump possibilities uncertain and never assumes a picked-up card is retained', () => {
		const context = tableContext({
			...base,
			upCard: 'JH',
			upCardTurnedDown: false,
			turnedDownSuit: null
		});
		expect(context.unseenTrump).toContain('JH');
		expect(context.auction.upCardLocation).toContain('discarded');
		expect(
			tableContext({ ...base, upCard: 'JH', upCardTurnedDown: true }).unseenTrump
		).not.toContain('JH');
	});
	it('remembers who played each past card without doubling a resolved trick', () => {
		const trick = {
			index: 0,
			ledSuit: 'S' as const,
			winnerSeat: 0 as const,
			plays: [
				{ seat: 0 as const, card: 'AS' as const },
				{ seat: 1 as const, card: '9C' as const }
			]
		};
		const view = { ...base, phase: 'trick_resolve' as const, trick, trickLog: [trick] };
		expect(tableContext(view).publicReads.find((read) => read.seat === 1)?.cardsPlayed).toEqual([
			'9C'
		]);
		expect(encodeForLlm(view, [])).toContain('trick1:led=S,winner=0<0:AS 1:9C');
	});
	it('changes the objective after making the hand and when defending a loner', () => {
		expect(tableContext({ ...base, tricksWon: [3, 0] }).objective).toContain('sweep');
		expect(
			tableContext({ ...base, makerSeat: 1, aloneSeat: 1, tricksWon: [0, 0] }).objective
		).toContain('four-point');
	});
	it('sends observations and their limitations through the actual Jev request, without private game fields', async () => {
		const decision = vi.fn(
			async (_input: Parameters<NonNullable<ModelFactory['decision']>>[0]) => ({
				choice: 'move_0',
				confidence: 0.7,
				usage: { inputTokens: 10, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 }
			})
		);
		const view = {
			...base,
			observations: [
				{
					handNo: base.handNo,
					seat: 0 as const,
					kind: 'play' as const,
					action: 'Ace of spades',
					elapsedMs: 3250,
					timing: 'human-observed' as const
				}
			]
		};
		const factory: ModelFactory = {
			provider: 'openrouter',
			decision,
			slugFor: () => 'unused',
			talk: () => {
				throw new Error('Conversation cannot make decisions');
			}
		};
		await callModel(
			{ factory, ...defaultPersonas()[1], nonce: 'context-test' },
			{
				internalToken: '',
				gameId: '',
				seat: 2,
				turnId: 'test',
				kind: 'play',
				view,
				legal: view.legal,
				deadlineAt: Date.now() + 5000
			},
			view.legal,
			{ escalate: false, budgetMs: 4000 }
		);
		const state = decision.mock.calls[0][0].state;
		expect(state).toContain('human-observed');
		expect(state).toContain('Computer timing');
		expect(state).not.toMatch(/deckOrder|internalToken|dealerDiscard|"kitty"|seed=/);
	});
});

describe('distinct players and saved settings', () => {
	it('keeps conversational prompts focused on voice instead of move selection', () => {
		const layers = buildLayers({
			kind: 'play',
			conversation: true,
			...defaultPersonas()[2],
			nonce: 'conversation-test',
			seat: 3,
			body: 'A public trick just ended.'
		});
		const instructions = layers.system.map((layer) => layer.text).join('\n');
		expect(instructions).toContain('voice and tone only');
		expect(instructions).toContain('Do not choose a move');
		expect(instructions).not.toContain('Dials bias your preference');
		expect(instructions).not.toContain('Choose the supplied legal option');
	});
	it('gives Grandpa calculated risk without degrading partnership play', () => {
		const personas = defaultPersonas();
		expect(personas.map((p) => p.persona.name)).toEqual(['Rita', 'Ned', 'Grandpa']);
		expect(personas[2].persona.risk).toBeGreaterThan(personas[0].persona.risk);
		expect(personas[2].persona.prompt).toContain('never deliberately misplay');
	});
	it('snapshots custom profiles and house context without aliasing mutable settings', () => {
		const settings = defaultSettings(1);
		settings.housePrompt = 'A quiet Sunday game.';
		settings.personas[2] = { ...settings.personas[2], name: 'Grandpa', risk: 0.9 };
		const snapshot = personasFromSettings(settings);
		settings.personas[2] = { ...settings.personas[2], name: 'Changed later' };
		expect(snapshot[2]).toMatchObject({
			seat: 3,
			persona: { name: 'Grandpa', risk: 0.9, housePrompt: 'A quiet Sunday game.' }
		});
	});
});
