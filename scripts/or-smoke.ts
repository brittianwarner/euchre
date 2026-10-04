/** Live provider smoke: only Jev selects a move. */
import { modelFactoryFromEnv } from '../src/lib/ai/model';
import { JEV_MODEL } from '../src/lib/ai/jev';
const factory = modelFactoryFromEnv(process.env);
if (!factory?.decision) throw new Error('Set OPENROUTER_API_KEY');
const started = Date.now();
const result = await factory.decision({
	state: 'Euchre. Hearts trump. Lead a new trick. Your hand: JS, AH, 9C. Choose a legal lead.',
	criteria: {
		move_0: 'Play jack of spades',
		move_1: 'Play ace of hearts',
		move_2: 'Play nine of clubs'
	},
	signal: AbortSignal.timeout(5000)
});
console.log({ model: JEV_MODEL, choice: result.choice, elapsedMs: Date.now() - started });
