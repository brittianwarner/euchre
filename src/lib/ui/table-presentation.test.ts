import { describe, expect, it } from 'vitest';
import { createGame, project } from '#lib/euchre/index.ts';
import type { PublicGameView } from '#lib/protocol/index.ts';
import {
	reconcileHandSlots,
	TRICK_PRESENTATION_MS,
	TABLE_MOTION,
	turnInstruction
} from './table-presentation';
import { TEMPO } from '#lib/actors/euchre-table/tempo.ts';

const base = project(createGame({ gameId: 'layout', seed: 'layout', internalToken: 'test' }), 0);
const view = { ...base, hand: ['9H', 'TH', 'JH', 'QH', 'KH'], handNo: 1 } as PublicGameView;

describe('stable hand presentation', () => {
	it('does not move remaining cards after a play or a repeated sync', () => {
		const slots = reconcileHandSlots(null, view);
		const after = { ...view, hand: view.hand.filter((card) => card !== 'TH'), v: view.v + 1 };
		expect(reconcileHandSlots(slots, after)).toBe(slots);
		expect(reconcileHandSlots(slots, { ...after, v: after.v + 1 })).toBe(slots);
	});
	it('appends a dealer pickup without moving the five cards, and resets on a new deal', () => {
		const slots = reconcileHandSlots(null, view);
		expect(reconcileHandSlots(slots, { ...view, hand: [...view.hand, 'AH'] }).cards).toEqual([
			...view.hand,
			'AH'
		]);
		expect(reconcileHandSlots(slots, { ...view, handNo: 2, hand: ['9C', 'AC'] }).cards).toEqual([
			'9C',
			'AC'
		]);
	});
	it('does not hide the sitting-out explanation behind another player’s turn', () => {
		expect(
			turnInstruction({
				...view,
				status: 'active',
				phase: 'trick_play',
				sittingSeat: 0,
				turnSeat: 1
			})
		).toBe('Your partner is going alone. Enjoy the hand.');
	});
	it('reserves enough server time for the final card, reading, and all four collected cards', () => {
		expect(TRICK_PRESENTATION_MS).toBeGreaterThan(
			TABLE_MOTION.play + TABLE_MOTION.read + TABLE_MOTION.collect + 3 * TABLE_MOTION.stagger
		);
		if (process.env.EUCHRE_FAST_TEMPO !== '1')
			expect(TEMPO.trickResolveMs).toBe(TRICK_PRESENTATION_MS);
	});
});
