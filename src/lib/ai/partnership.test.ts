import { afterEach, describe, expect, it, vi } from 'vitest';
import { partnershipCases, playView } from '../../../scripts/fixtures/partnership';
import { analyzePartnership } from './partnership';
import { rankMoves } from './heuristic';
import { callModel } from './decide';
import { modelFactoryFromEnv } from './model';
import { JEV_MODEL } from './jev';
import { defaultPersonas } from '../server/personas';
import { runLadder } from '../actors/ai-seat/decide';
import { setLlmProvider } from '../actors/ai-seat/llm-bridge';

const partner = defaultPersonas().find((assignment) => assignment.seat === 2)!;
const analyze = (view: ReturnType<typeof playView>) => analyzePartnership(view, view.legal)!;

describe('partnership decision evidence', () => {
	it('marks a last-seat partner winner secure and distinguishes overtaking from laying off', () => {
		const { context, options } = analyze(partnershipCases[0].view);
		expect(context).toMatchObject({
			partnerSeat: 0,
			partnerWinning: true,
			partnerTrickSecured: true,
			opponentsAfterYou: [],
			possibleOpponentBeaters: []
		});
		expect(options.find((move) => move.id === 'play:TS')).toMatchObject({
			winnerAfterPlay: { seat: 0, card: 'KS' },
			overtakesPartner: false,
			remainingHand: ['AS', '9C', 'KC', 'AD']
		});
		expect(options.find((move) => move.id === 'play:AS')?.overtakesPartner).toBe(true);
	});
	it('does not mistake a vulnerable lead for a won trick; shows how covering helps', () => {
		const { context, options } = analyze(partnershipCases[4].view);
		expect(context.partnerTrickSecured).toBe(false);
		expect(context.opponentsAfterYou).toEqual([3]);
		expect(context.possibleOpponentBeaters).toEqual(expect.arrayContaining(['AH', 'JD']));
		expect(options.find((move) => move.id === 'play:JH')).toMatchObject({
			overtakesPartner: true,
			possibleOpponentBeatersAfterPlay: []
		});
	});
	it('recognizes an unbeatable partner even with opponents left to play', () => {
		expect(analyze(partnershipCases[3].view).context).toMatchObject({
			opponentsAfterYou: [3],
			partnerTrickSecured: true,
			possibleOpponentBeaters: []
		});
	});
	it('saves the right bower when a nine already wins from the last seat', () => {
		const view = playView(
			['JH', 'AH', '9H'],
			[
				{ seat: 3, card: 'KS' },
				{ seat: 0, card: 'QS' },
				{ seat: 1, card: 'AS' }
			]
		);
		const decision = analyze(view);
		expect(decision.context.opponentsAfterYou).toEqual([]);
		expect(decision.options.find((option) => option.card === 'JH')).toMatchObject({
			cheaperEquivalentCards: expect.arrayContaining(['9H', 'AH'])
		});
	});
	it('identifies a cheaper equivalent without treating necessary cover as equivalent', () => {
		expect(
			analyze(partnershipCases[3].view).options.find((option) => option.card === 'JD')
		).toMatchObject({
			resource: 'left bower, second-highest trump',
			cheaperEquivalentCards: ['9H']
		});
		expect(
			analyze(partnershipCases[5].view).options.find((option) => option.card === 'AS')
		).toMatchObject({ cheaperEquivalentCards: ['KS'] });
		expect(
			analyze(partnershipCases[4].view).options.find((option) => option.card === 'JH')
		).toMatchObject({ cheaperEquivalentCards: [] });
	});
	it.each([0, 1, 2, 3] as const)('recognizes partners relative to seat %i', (seat) => {
		const base = partnershipCases[0].view;
		const rotate = (s: number) => ((s + seat + 2) % 4) as 0 | 1 | 2 | 3;
		const view = {
			...base,
			you: seat,
			turnSeat: seat,
			trick: {
				...base.trick,
				plays: base.trick.plays.map((play) => ({ ...play, seat: rotate(play.seat) }))
			}
		};
		expect(analyze(view).context).toMatchObject({
			partnerSeat: rotate(0),
			yourTeam: seat % 2,
			partnerTrickSecured: true
		});
	});
	it('uses the left bower as trump when determining winners and following suit', () => {
		const view = playView(
			['9H', 'AD', '9C'],
			[
				{ seat: 0, card: 'JD' },
				{ seat: 1, card: 'AH' }
			]
		);
		expect(view.legal.map((move) => move.id)).toEqual(['play:9H']);
		expect(analyze(view).context).toMatchObject({
			currentWinner: { seat: 0, card: 'JD' },
			possibleOpponentBeaters: ['JH'],
			partnerTrickSecured: false
		});
	});
	it('does not count an already played or turned-down bower as a threat', () => {
		const view = playView(
			['9H', 'AD'],
			[
				{ seat: 0, card: 'JD' },
				{ seat: 1, card: 'AH' }
			],
			{
				upCard: 'JH',
				upCardTurnedDown: true
			}
		);
		expect(analyze(view).context.partnerTrickSecured).toBe(true);
		expect(analyze({ ...view, upCardTurnedDown: false }).context.partnerTrickSecured).toBe(false);
		const played = {
			...view,
			upCardTurnedDown: false,
			trickLog: [
				{
					index: 1,
					ledSuit: 'H' as const,
					winnerSeat: 0 as const,
					plays: [{ seat: 0 as const, card: 'JH' as const }]
				}
			]
		};
		expect(analyze(played).context.partnerTrickSecured).toBe(true);
	});
	it('uses public void evidence conservatively instead of inventing enemy holdings', () => {
		const view = playView(
			['9H', 'AD'],
			[
				{ seat: 0, card: 'JD' },
				{ seat: 1, card: 'AH' }
			],
			{
				trickLog: [
					{
						index: 1,
						ledSuit: 'H',
						winnerSeat: 0,
						plays: [
							{ seat: 0, card: 'KH' },
							{ seat: 3, card: '9C' }
						]
					}
				]
			}
		);
		expect(analyze(view).context).toMatchObject({
			partnerTrickSecured: true,
			unseenCardsArePossibilitiesNotKnownHoldings: true
		});
	});
	it('skips a sitting seat and uses the acting team rather than always the human team', () => {
		const view = playView(['AH', '9C'], [{ seat: 0, card: 'KH' }], {
			you: 1,
			turnSeat: 1,
			aloneSeat: 0,
			sittingSeat: 2,
			makerSeat: 0,
			tricksWon: [2, 1]
		});
		expect(analyze(view).context).toMatchObject({
			yourTeam: 1,
			partnerSeat: 3,
			seatsAfterYou: [3],
			opponentsAfterYou: [],
			yourTeamTricks: 1,
			opponentTricks: 2,
			partnerTrickSecured: false,
			lonerSweepStopped: true
		});
	});
	it('keeps the sweep objective after makers have already made the hand', () => {
		const view = { ...partnershipCases[0].view, tricksWon: [3, 0] as const };
		expect(analyze(view).context).toMatchObject({
			yourTeamHasWonHand: true,
			makersHaveThreeTricks: true,
			makersCanStillSweep: true
		});
		expect(analyze({ ...view, tricksWon: [3, 1] }).context.makersCanStillSweep).toBe(false);
	});
	it('does not attach trick advice during bidding', () => {
		const view = { ...partnershipCases[0].view, phase: 'bid_round_1' as const };
		expect(analyzePartnership(view, view.legal)).toBeNull();
	});
});

describe('conservation prior regressions', () => {
	it.each(partnershipCases.filter((_, index) => index !== 4))('$name', ({ view, expected }) => {
		expect(rankMoves(view)[0].id).toBe(expected);
	});
	it('does not spend low trump when a plain loser can be discarded', () => {
		const view = playView(
			['9H', 'QC', 'AC'],
			[
				{ seat: 3, card: '9S' },
				{ seat: 0, card: 'AS' },
				{ seat: 1, card: 'KS' }
			]
		);
		expect(rankMoves(view)[0].id).toBe('play:QC');
	});
	it('does not overtake a partner to stop a loner sweep the partner already stopped', () => {
		const view = playView(
			['KH', 'JD', '9C'],
			[
				{ seat: 3, card: '9H' },
				{ seat: 0, card: 'AH' }
			],
			{ aloneSeat: 3, sittingSeat: 1, makerSeat: 3 }
		);
		expect(rankMoves(view)[0].id).toBe('play:KH');
		expect(analyze(view).context.partnerTrickSecured).toBe(true);
	});
});

describe('Jev integration', () => {
	afterEach(() => setLlmProvider({ decide: null, banter: null }));
	it('sends partnership facts and option consequences and preserves Jev final authority', async () => {
		const view = partnershipCases[0].view;
		// Deliberately choose the overtake: evidence must inform Jev, never replace it.
		const decision = vi.fn().mockResolvedValue({
			choice: 'move_0',
			confidence: 0.9,
			usage: { inputTokens: 1, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 }
		});
		const talk = vi.fn(() => {
			throw new Error('Text model must not make game decisions');
		});
		const outcome = await callModel(
			{
				...partner,
				nonce: 'partnership-test',
				factory: {
					...modelFactoryFromEnv({ OPENROUTER_API_KEY: 'test' })!,
					decision,
					talk
				}
			},
			{
				internalToken: '',
				gameId: 'test',
				seat: 2,
				turnId: 'test',
				kind: 'play',
				view,
				legal: view.legal,
				ranking: rankMoves(view),
				deadlineAt: Date.now() + 10000
			},
			view.legal,
			{ escalate: false, budgetMs: 5000 }
		);
		const sent = decision.mock.calls[0][0];
		expect(JSON.parse(sent.state).user).toContain('"partnerTrickSecured":true');
		expect(JSON.parse(sent.criteria.move_1)).toMatchObject({ card: 'TS', overtakesPartner: false });
		expect(outcome).toMatchObject({ moveId: 'play:AS', source: 'llm', modelId: JEV_MODEL });
		expect(talk).not.toHaveBeenCalled();
	});
	it('offers every legal card to Jev even if a casual heuristic ranks the cover last', async () => {
		const view = partnershipCases[1].view;
		const decide = vi.fn(async () => ({ moveId: 'play:JH' }));
		setLlmProvider({ decide });
		const result = await runLadder(
			{ ...partner, nonce: 'partnership-test', degraded: false, now: Date.now, log: () => {} },
			{
				internalToken: '',
				gameId: 'test',
				seat: 2,
				turnId: 'test',
				kind: 'play',
				view,
				legal: view.legal,
				deadlineAt: Date.now() + 10000
			}
		);
		expect(decide).toHaveBeenCalledWith(expect.objectContaining({ candidates: view.legal }));
		expect(result).toMatchObject({ moveId: 'play:JH', source: 'llm', candidates: 5, llmCalls: 1 });
	});
});
