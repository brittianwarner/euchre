/** Live smoke test: does our OpenRouter factory actually produce a legal euchre move? */
import { generateObject } from 'ai';
import { z } from 'zod';
import { modelFactoryFromEnv, modelParams } from '../src/lib/ai/model';

const factory = modelFactoryFromEnv(process.env);
if (!factory) { console.log('FAIL: no factory built — no key found'); process.exit(1); }
console.log('provider tag :', factory.provider);

// The real mechanism: a z.enum rebuilt from the legal set makes an illegal move
// structurally impossible.
const legal = ['play:JS', 'play:AH', 'play:9C'] as const;
const schema = z.object({
  move: z.enum(legal),
  why: z.string().max(120)
});

for (const [tier, id] of [['play','claude-haiku-4-5'],['bid','claude-opus-5']] as const) {
  const model = tier === 'play' ? factory.play(id as any) : factory.bid(id as any);
  const params = modelParams(id, 0.5, factory.provider);
  console.log(`\n[${tier}] ${id} -> params keys: ${JSON.stringify(Object.keys(params))} providerOptions: ${JSON.stringify(params.providerOptions)}`);
  const t0 = Date.now();
  try {
    const res = await generateObject({
      model,
      schema,
      system: 'You are a euchre player. Hearts are trump. Choose one legal move.',
      messages: [{ role: 'user', content: 'Your legal moves: play:JS, play:AH, play:9C. You are on lead. Pick one.' }],
      maxOutputTokens: tier === 'play' ? 1200 : 1600,
      maxRetries: 0,
      ...params
    });
    const ok = (legal as readonly string[]).includes(res.object.move);
    console.log(`  -> ${res.object.move}  (${Date.now()-t0}ms)  LEGAL=${ok}`);
    console.log(`  why: ${res.object.why.slice(0,90)}`);
  } catch (e) {
    console.log(`  ERROR after ${Date.now()-t0}ms: ${String(e).slice(0,300)}`);
  }
}
