import { describe, expect, it, vi } from 'vitest';
import { euchreTable, type TableState } from './index';
import { advance, apply, legalMoves, type GameState } from '#lib/euchre/index.ts';
import type { ReviewMsg, TickMsg } from './queues';

type Message = { name: 'review'; body: ReviewMsg } | { name: 'tick'; body: TickMsg };

/** Real actor run loop + real reducer, with only transport, persistence and clocks stubbed. */
async function fixture(trickIndex = 0) {
	const config = euchreTable.config;
	if (!('createState' in config) || typeof config.createState !== 'function')
		throw new Error('Expected state factory');
	const state = (await config.createState({ key: ['table', 'review-test'] } as never, {
		seed: 'review-test'
	})) as TableState;
	let game: GameState = state.game;
	for (let i = 0; i < 200; i++) {
		if (game.hand.phase === 'trick_resolve' && game.hand.trick.index === trickIndex) break;
		const seat = game.hand.turnSeat;
		const moves = seat === null ? [] : legalMoves(game, seat);
		game = seat !== null && moves[0] ? apply(game, seat, moves[0].move).state : advance(game).state;
	}
	if (game.hand.phase !== 'trick_resolve') throw new Error('Fixture failed to finish a trick');
	state.game = game;
	const inbox: Message[] = [];
	const sent: unknown[] = [];
	const handle = { send: vi.fn(async () => {}) };
	const archive = new Map<string, string>();
	const context = {
		kv: {
			put: vi.fn(async (key: string, value: string) => {
				archive.set(key, value);
			}),
			get: vi.fn(async (key: string) => archive.get(key) ?? null)
		},
		state,
		key: ['table', 'review-test'],
		vars: {},
		conn: { state: { role: 'player', seat: 0, userId: 'review-player' } },
		conns: new Map([
			[
				'test',
				{
					state: { role: 'player', seat: 0 },
					send: (_name: string, event: unknown) => sent.push(event)
				}
			]
		]),
		log: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
		schedule: { after: vi.fn(async () => 'timer'), cancel: vi.fn(async () => true) },
		saveState: vi.fn(async () => {}),
		broadcast: vi.fn(),
		destroy: vi.fn(),
		client: () => ({
			aiSeat: { getOrCreate: () => handle },
			playerProfile: { getOrCreate: () => handle }
		}),
		queue: {
			send: vi.fn(async (name: Message['name'], body: ReviewMsg | TickMsg) => {
				inbox.push({ name, body } as Message);
			}),
			async *iter() {
				while (inbox.length) yield { ...inbox.shift()!, complete: async () => {} };
			}
		}
	};
	const turnId = game.turnId;
	const envelope = { internalToken: game.internalToken, gameId: game.gameId, turnId };
	const run = async (...messages: Message[]) => {
		inbox.push(...messages);
		const handler = euchreTable.config.run;
		if (typeof handler !== 'function') throw new Error('Expected actor run function');
		await handler(context as never);
	};
	const review = (action: 'set' | 'continue', enabled?: boolean): Message => ({
		name: 'review',
		body: { ...envelope, action, enabled }
	});
	const tempo: Message = { name: 'tick', body: { ...envelope, kind: 'tempo' } };
	return { context, state, run, review, tempo, turnId, sent, handle };
}

describe('durable trick review', () => {
	it('pauses after the server read gate, and stays paused across wake/reconnect', async () => {
		const f = await fixture();
		await f.run(f.review('set', true), f.tempo);
		expect(f.state.game.hand.phase).toBe('trick_resolve');
		expect(f.state.reviewReadyTurnId).toBe(f.turnId);
		f.context.schedule.after.mockClear();
		await f.run();
		expect(f.context.schedule.after).not.toHaveBeenCalled();
		expect(euchreTable.config.actions!.snapshot(f.context as never).awaitingTrickReview).toBe(true);
	});
	it('ignores an early Continue; it cannot bypass the server read pause', async () => {
		const f = await fixture();
		await f.run(f.review('set', true), f.review('continue'));
		expect(f.state.game.turnId).toBe(f.turnId);
		expect(f.state.reviewReadyTurnId).toBeUndefined();
	});
	it('continues once and ignores a duplicate or stale Continue', async () => {
		const f = await fixture();
		await f.run(f.review('set', true), f.tempo, f.review('continue'), f.review('continue'));
		expect(f.state.game.hand.trick.index).toBe(1);
		expect(f.state.game.hand.phase).toBe('trick_play');
		expect(f.state.reviewReadyTurnId).toBeNull();
	});
	it('releases a paused trick when review is switched off', async () => {
		const f = await fixture();
		await f.run(f.review('set', true), f.tempo, f.review('set', false));
		expect(f.state.reviewTricks).toBe(false);
		expect(f.state.game.hand.trick.index).toBe(1);
	});
	it('automatically advances existing tables without the new preference', async () => {
		const f = await fixture();
		await f.run(f.tempo);
		expect(f.state.game.hand.phase).toBe('trick_play');
		expect(f.state.game.hand.trick.index).toBe(1);
	});
	it('holds the fifth trick until Continue, then scores the hand once', async () => {
		const f = await fixture(4);
		await f.run(f.review('set', true), f.tempo);
		expect(f.state.game.hand.phase).toBe('trick_resolve');
		await f.run(f.review('continue'), f.review('continue'));
		expect(f.state.game.hand.phase).toBe('hand_score');
		expect(f.state.handsPlayed).toBe(1);
		const review = await euchreTable.config.actions!.getHandReview(
			f.context as never,
			f.state.game.hand.handNo
		);
		expect(review?.tricks).toHaveLength(5);
		expect(review?.buried).toHaveLength(4);
	});
	it('finishes and archives the fifth trick even when AI lifecycle delivery hangs', async () => {
		const f = await fixture(4);
		await f.run(f.review('set', true), f.tempo);
		vi.useFakeTimers();
		try {
			f.handle.send.mockImplementation(() => new Promise(() => {}));
			const finished = f.run(f.review('continue'));
			await vi.advanceTimersByTimeAsync(4000);
			await finished;
			expect(f.state.game.hand.phase).toBe('hand_score');
			expect(f.state.handsPlayed).toBe(1);
			const review = await euchreTable.config.actions!.getHandReview(
				f.context as never,
				f.state.game.hand.handNo
			);
			expect(review?.tricks).toHaveLength(5);
			expect(review?.tricksWon).toEqual(f.state.game.hand.tricksWon);
			expect(review?.delta).toEqual(f.state.game.hand.delta);
		} finally {
			vi.useRealTimers();
		}
	});
	it('keeps buried cards inaccessible until the hand has finished and rejects non-player reads', async () => {
		const f = await fixture();
		expect(
			await euchreTable.config.actions!.getHandReview(f.context as never, f.state.game.hand.handNo)
		).toBeNull();
		f.context.conn.state.role = 'internal';
		await expect(euchreTable.config.actions!.getHandReview(f.context as never, 0)).rejects.toThrow(
			'Not a player connection'
		);
	});
	it('ignores old human timeout messages without playing or abandoning', async () => {
		const f = await fixture();
		await f.run(f.tempo);
		const before = JSON.stringify(f.state.game);
		await f.run({
			name: 'tick',
			body: { ...f.tempo.body, turnId: f.state.game.turnId, kind: 'abandon' }
		});
		expect(JSON.stringify(f.state.game)).toBe(before);
		expect(f.state.abandonStrikes).toBe(0);
	});
	it('rejects forged internal review messages', async () => {
		const f = await fixture();
		const forged = f.review('set', true);
		await f.run({ ...forged, body: { ...forged.body, internalToken: 'forged' } } as Message);
		expect(f.state.reviewTricks).toBeUndefined();
	});
	it('authorizes the player action and queues without mutating state in the action lane', async () => {
		const f = await fixture();
		await euchreTable.config.actions!.reviewTrick(f.context as never, {
			action: 'set',
			enabled: true
		});
		expect(f.state.reviewTricks).toBeUndefined();
		await f.run();
		expect(f.state.reviewTricks).toBe(true);
		f.context.conn.state.role = 'internal';
		await expect(
			euchreTable.config.actions!.reviewTrick(f.context as never, { action: 'set', enabled: false })
		).rejects.toThrow('Not a player connection');
	});
});
