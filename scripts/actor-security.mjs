/** Against a local running match: unauthenticated and forged internal calls must fail. */
import { createClient } from 'rivetkit/client';
import assert from 'node:assert/strict';
const client = createClient({ endpoint: 'http://127.0.0.1:6420', devtools: false });
const key = ['table', process.argv[2]];
for (const params of [{}, { token: 'dev' }, { internalToken: 'forged-match-token' }]) {
	const table = client.euchreTable.get(key, { params });
	await assert.rejects(table.snapshot());
	await assert.rejects(
		table.send('aiDecision', { internalToken: 'forged-match-token', moveId: 'pass' })
	);
}
console.log('Six unauthorized table reads/publishes rejected.');
