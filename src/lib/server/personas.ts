/**
 * Default AI personas snapshotted into a new table at creation.
 *
 * M2 uses heuristic play inside the table — these names/blurbs are HUD-only
 * until the `aiSeat` actor lands. Keep three seats (1 West, 2 North, 3 East).
 */

import type { PersonaAssignment, PersonaConfig } from '#lib/protocol/index.ts';

/** Shared stub prompts — never consulted by the M2 heuristic path. */
function stubPersona(partial: Pick<PersonaConfig, 'id' | 'name' | 'blurb'>): PersonaConfig {
	return {
		id: partial.id,
		version: 1,
		name: partial.name,
		blurb: partial.blurb,
		prompt: 'Play solid euchre. Keep banter short.',
		housePrompt: 'Friendly kitchen-table game.',
		difficulty: 'casual',
		aggression: 0.45,
		risk: 0.4,
		chattiness: 0.35,
		temperature: 0.5,
		modelId: 'claude-haiku-4-5',
		bidModelId: 'claude-haiku-4-5'
	};
}

/** Three opponents for a fresh `/play` match. */
export function defaultPersonas(): readonly PersonaAssignment[] {
	return [
		{
			seat: 1,
			persona: stubPersona({
				id: 'west-rita',
				name: 'Rita',
				blurb: 'Orders up with a grin.'
			}),
			dossier: ''
		},
		{
			seat: 2,
			persona: stubPersona({
				id: 'north-ned',
				name: 'Ned',
				blurb: 'Your partner. Counts trump.'
			}),
			dossier: ''
		},
		{
			seat: 3,
			persona: stubPersona({
				id: 'east-eva',
				name: 'Eva',
				blurb: 'Quiet until the march.'
			}),
			dossier: ''
		}
	];
}
