import { afterEach, describe, expect, it, vi } from 'vitest';
import { euchreTable, type TableState } from './index';
import { advance, apply, legalMoves, type GameState } from '#lib/euchre/index.ts';
import { defaultPersonas } from '#lib/server/personas.ts';
import type { AIDecideRequest, AIDecision } from '#lib/protocol/index.ts';
import type { TickMsg, AiSayMsg } from './queues';

type Message =
	| { name: 'aiDecision'; body: AIDecision }
	| { name: 'tick'; body: TickMsg }
	| { name: 'aiSay'; body: AiSayMsg };

async function fixture() {
	let now = 100_000;
	vi.spyOn(Date, 'now').mockImplementation(() => now);
	const config = euchreTable.config;
	if (!('createState' in config) || typeof config.createState !== 'function')
		throw new Error('No state factory');
	const state = (await config.createState({ key: ['table', 'delivery-test'] } as never, {
		seed: 'delivery-test',
		personas: defaultPersonas()
	})) as TableState;
	let game: GameState = state.game;
	for (let i = 0; i < 300; i++) {
		const seat = game.hand.turnSeat;
		const moves = seat === null ? [] : legalMoves(game, seat);
		if (game.hand.phase === 'trick_play' && seat !== null && seat !== 0 && moves.length >= 2) break;
		game = seat !== null && moves[0] ? apply(game, seat, moves[0].move).state : advance(game).state;
	}
	if (game.hand.phase !== 'trick_play' || game.hand.turnSeat === 0)
		throw new Error('No AI play fixture');
	state.game = game;
	const inbox: Message[] = [];
	const requests: AIDecideRequest[] = [];
	const handle = {
		send: vi.fn(async (name: string, body: AIDecideRequest) => {
			if (name === 'decide') requests.push(body);
		})
	};
	const context = {
		state,
		key: ['table', game.gameId],
		vars: {},
		conns: new Map(),
		kv: { put: vi.fn(async () => {}), get: vi.fn(async () => null) },
		log: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
		saveState: vi.fn(async () => {}),
		broadcast: vi.fn(),
		destroy: vi.fn(),
		schedule: { after: vi.fn(async () => 'timer'), cancel: vi.fn(async () => true) },
		client: () => ({
			aiSeat: { getOrCreate: () => handle },
			playerProfile: { getOrCreate: () => handle }
		}),
		queue: {
			async *iter() {
				while (inbox.length) yield { ...inbox.shift()!, complete: async () => {} };
			}
		}
	};
	const run = async (...messages: Message[]) => {
		inbox.push(...messages);
		if (typeof config.run !== 'function') throw new Error('No run loop');
		await config.run(context as never);
	};
	await run();
	const request = requests[0];
	if (!request) throw new Error('No AI request');
	const reply = (moveId: string, overrides: Partial<AIDecision> = {}): Message => ({
		name: 'aiDecision',
		body: {
			internalToken: game.internalToken,
			gameId: game.gameId,
			seat: request.seat,
			turnId: request.turnId,
			moveId,
			source: 'llm',
			...overrides
		} as AIDecision
	});
	const tick = (kind: TickMsg['kind']): Message => ({
		name: 'tick',
		body: {
			internalToken: game.internalToken,
			gameId: game.gameId,
			seat: request.seat,
			turnId: request.turnId,
			kind
		}
	});
	return {
		state,
		run,
		request,
		reply,
		tick,
		context,
		setTime: (value: number) => {
			now = value;
		}
	};
}
afterEach(() => vi.restoreAllMocks());

describe('Jev answer delivery through the real table loop', () => {
	it('replaces the parked fallback and applies the actual model choice after the floor', async () => {
		const f = await fixture();
		const pending = f.state.game.pending!;
		const chosen = f.request.legal.find((move) => move.id !== pending.decision)!;
		const count = f.state.game.hand.trick.plays.length;
		f.setTime(pending.revealAt + 1);
		await f.run(f.reply(chosen.id));
		expect(f.state.game.hand.trick.plays).toContainEqual({
			seat: f.request.seat,
			card: chosen.move.t === 'play' ? chosen.move.card : null
		});
		expect(f.state.game.hand.trick.plays).toHaveLength(count + 1);
		expect(f.context.log.info).toHaveBeenCalledWith(
			'AI decision accepted',
			expect.objectContaining({ source: 'llm' })
		);
	});
	it('does not release the safety move at the think floor while the model is outstanding', async () => {
		const f = await fixture();
		f.setTime(f.state.game.pending!.revealAt + 1);
		const before = f.state.game.hand.trick.plays;
		await f.run(f.tick('releaseAi'));
		expect(f.state.game.turnId).toBe(f.request.turnId);
		expect(f.state.game.hand.trick.plays).toEqual(before);
		expect(f.state.game.pending?.answered).toBe(false);
	});
	it('keeps an early answer behind the presentation floor and ignores a duplicate', async () => {
		const f = await fixture();
		const floor = f.state.game.pending!.revealAt;
		const [first, second] = f.request.legal;
		await f.run(f.reply(first.id), f.reply(second.id));
		expect(f.state.game.pending).toMatchObject({
			decision: first.id,
			answered: true,
			source: 'llm'
		});
		f.setTime(floor + 1);
		await f.run(f.tick('releaseAi'));
		expect(f.state.game.turnId).not.toBe(f.request.turnId);
	});
	it('keeps a pre-upgrade fallback pending after wake and still accepts the model answer', async () => {
		const f = await fixture();
		const pending = f.state.game.pending!;
		delete pending.answered;
		const chosen = f.request.legal.find((move) => move.id !== pending.decision)!;
		f.setTime(pending.revealAt + 1);
		await f.run();
		expect(f.state.game.turnId).toBe(f.request.turnId);
		await f.run(f.reply(chosen.id));
		expect(f.state.game.hand.trick.plays).toContainEqual({
			seat: f.request.seat,
			card: chosen.move.t === 'play' ? chosen.move.card : null
		});
	});
	it('releases a pre-upgrade forced move at its floor without waiting for a model reply', async () => {
		const f = await fixture();
		const pending = f.state.game.pending!;
		delete pending.answered;
		pending.source = 'forced';
		f.setTime(pending.revealAt + 1);
		await f.run();
		expect(f.state.game.turnId).not.toBe(f.request.turnId);
	});
	it('releases exactly one durable fallback on timeout and rejects late/forged answers', async () => {
		const f = await fixture();
		const pending = f.state.game.pending!;
		const fallback = f.request.legal.find((move) => move.id === pending.decision)!;
		await f.run(f.reply(f.request.legal.at(-1)!.id, { internalToken: 'forged' }));
		expect(f.state.game.pending?.answered).toBe(false);
		f.setTime(f.request.deadlineAt + 1);
		await f.run(f.tick('aiTimeout'), f.reply(f.request.legal.at(-1)!.id));
		expect(f.state.game.hand.trick.plays).toContainEqual({
			seat: f.request.seat,
			card: fallback.move.t === 'play' ? fallback.move.card : null
		});
		expect(
			f.state.game.hand.trick.plays.filter((play) => play.seat === f.request.seat)
		).toHaveLength(1);
	});
	it('allows post-move chatter in this hand but drops chatter from a previous hand', async () => {
		const f = await fixture();
		const say = (handNo: number): Message => ({
			name: 'aiSay',
			body: {
				internalToken: f.request.internalToken,
				gameId: f.request.gameId,
				seat: f.request.seat,
				turnId: 'previous-turn',
				handNo,
				msgId: 'talk',
				text: 'Now that was a hand.',
				final: true
			}
		});
		await f.run(say(f.state.game.hand.handNo), say(f.state.game.hand.handNo - 1));
		expect(f.context.broadcast.mock.calls.filter(([name]) => name === 'chat')).toHaveLength(1);
	});
});
