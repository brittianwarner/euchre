/** What does gemini-3.6-flash actually return through OpenRouter? */
import { generateText, generateObject } from 'ai';
import { z } from 'zod';
import { createOpenRouter } from '@openrouter/ai-sdk-provider';

const or = createOpenRouter({ apiKey: process.env.OPENROUTER_API_KEY! });
const model = or.chat('google/gemini-3.6-flash');
const legal = ['play:JS', 'play:AH', 'play:9C'] as const;
const schema = z.object({ move: z.enum(legal), why: z.string().max(120) });
const sys = 'You are a euchre player. Hearts are trump. Choose exactly one legal move.';
const usr = 'Legal moves: play:JS, play:AH, play:9C. You are on lead. Reply with JSON only.';

console.log('--- A: generateText (see raw shape) ---');
try {
  const r = await generateText({ model, system: sys, messages: [{role:'user',content:usr}], maxOutputTokens: 400 });
  console.log('text:', JSON.stringify(r.text).slice(0, 300));
  console.log('reasoning len:', (r as any).reasoningText?.length ?? 0);
  console.log('finishReason:', r.finishReason);
} catch (e) { console.log('ERR', String(e).slice(0,200)); }

console.log('\n--- B: generateObject, no providerOptions ---');
try {
  const r = await generateObject({ model, schema, system: sys, messages: [{role:'user',content:usr}], maxOutputTokens: 800, maxRetries: 0 });
  console.log('OK ->', r.object);
} catch (e) { console.log('ERR', String(e).slice(0,200)); }

console.log('\n--- C: generateObject + repairText + bigger budget ---');
try {
  const r = await generateObject({
    model, schema, system: sys, messages: [{role:'user',content:usr}],
    maxOutputTokens: 1200, maxRetries: 1,
    experimental_repairText: async ({ text }) => { const m=/\{[\s\S]*\}/.exec(text); return m ? m[0] : null; }
  });
  console.log('OK ->', r.object);
} catch (e) { console.log('ERR', String(e).slice(0,200)); }
