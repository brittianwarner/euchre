import { describe, expect, it } from 'vitest';
import { screenBanter } from './screen';
import { createGame, project } from '#lib/euchre/index.ts';
const view = project(createGame({ gameId: 'banter', seed: 'banter' }), 0);
describe('no table coaching', () => {
	it.each([
		'Your call, partner.',
		'Your choice, partner.',
		'You should take it.',
		'Play your trump.',
		'Save the ace.',
		'Go alone!'
	])('blocks advice: %s', (text) => expect(screenBanter(text, view).ok).toBe(false));
	it('allows a public social reaction', () =>
		expect(screenBanter('Good game. Well played.', view).ok).toBe(true));
});
