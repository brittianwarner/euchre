import { createClient } from 'rivetkit/client';
const origin = process.argv[2] ?? 'http://localhost:5280';
const metadata = await (await fetch(origin + '/api/rivet/metadata')).json();
if (metadata.clientEndpoint !== origin + '/rivet') {
	throw new Error('Gateway discovery changed the browser origin');
}
const first = await fetch(origin + '/play/__data.json', { redirect: 'manual' });
const location = ((await first.json()) as any).location;
const cookie = first.headers.get('set-cookie')?.split(';')[0];
if (!location || !cookie) throw new Error(`Create failed ${first.status}`);
const gameId = location.split('/').pop();
const response = await fetch(origin + '/api/table-session/' + gameId, {
	method: 'POST',
	headers: { Origin: origin, Cookie: cookie }
});
if (!response.ok)
	throw new Error(`Session failed ${response.status}: ${(await response.text()).slice(0, 200)}`);
const session = (await response.json()) as any;
console.log('Session authorized', {
	gameId,
	durationSeconds: (session.expiresAt - Date.now()) / 1000
});
const pageData = (await (
	await fetch(origin + location + '/__data.json', { headers: { Cookie: cookie } })
).json()) as any;
let id;
for (const node of pageData.nodes ?? []) {
	if (node?.data?.[0]?.actorId) id = node.data[node.data[0].actorId];
}
if (!id) throw new Error('No actor ID in page data');
console.log('Resolved actor:', id);
const client = createClient({
	endpoint: origin + '/api/rivet',
	token: session.gatewayToken,
	headers: { Origin: origin, Cookie: cookie },
	devtools: false,
	gateway: { skipReadyWait: true }
});
const handle = client.euchreTable.getForId(id, { params: { token: session.token } });

const NativeSocket = globalThis.WebSocket;
globalThis.WebSocket = class extends NativeSocket {
	constructor(url: string, protocols: string[]) {
		const target = new URL(url);
		if (target.protocol === 'wss:') target.protocol = 'https:';
		else if (target.protocol === 'ws:') target.protocol = 'http:';
		if (target.origin !== origin) {
			throw new Error(`WebSocket changed the browser origin to ${target.origin}`);
		}
		super(url, { protocols, headers: { Origin: origin, Cookie: cookie } } as any);
	}
} as any;
const conn = handle.connect();
const snapshot = await Promise.race([
	conn.action({ name: 'snapshot', args: [] }),
	new Promise((_, rej) => setTimeout(() => rej(new Error('Timed out connecting')), 15000))
]);
console.log('WebSocket snapshot', { handNo: snapshot.handNo, phase: snapshot.phase });
if (process.argv.includes('--play-hand')) {
	let view = snapshot;
	let submittedTurn: string | undefined;
	let humanMoves = 0;
	const handNo = Math.max(1, snapshot.handNo);
	const deadline = Date.now() + 180_000;
	while (Date.now() < deadline) {
		if (view.result || view.handNo > handNo) {
			console.log('Production hand completed', { gameId, humanMoves, score: view.score });
			break;
		}
		if (view.legal.length && view.turnId !== submittedTurn) {
			const move = view.legal.find((m) => m.id === 'pass') ?? view.legal[0];
			const ack = await conn.action({
				name: 'submitMove',
				args: [
					{
						moveId: move.id,
						turnId: view.turnId,
						clientMoveId: crypto.randomUUID()
					}
				]
			});
			if (!ack.ok) throw new Error(`Production move rejected: ${ack.code}`);
			submittedTurn = view.turnId;
			humanMoves++;
			console.log('Human move accepted', { phase: view.phase, humanMoves });
		}
		await Bun.sleep(1000);
		view = await conn.action({ name: 'snapshot', args: [] });
	}
	if (!view.result && view.handNo <= handNo) throw new Error('Production hand timed out');
}
await conn.dispose();
const outsider = await fetch(origin + '/api/table-session/' + gameId, {
	method: 'POST',
	headers: { Origin: origin }
});
if (outsider.status !== 401) throw new Error('Anonymous session was not denied');
const stranger = await fetch(origin + '/play/__data.json');
const strangerCookie = stranger.headers.get('set-cookie')?.split(';')[0];
if (!strangerCookie) throw new Error('Missing second guest');
const stolen = await fetch(origin + '/api/table-session/' + gameId, {
	method: 'POST',
	headers: { Origin: origin, Cookie: strangerCookie }
});
if (stolen.status !== 403) throw new Error('Another guest could obtain table access');
const crossOrigin = await fetch(origin + '/api/table-session/' + gameId, {
	method: 'POST',
	headers: { Origin: 'https://untrusted.example', Cookie: cookie }
});
if (crossOrigin.status !== 403) throw new Error('Cross-origin request was not denied');
console.log('Session protection: anonymous 401; other guest 403; cross-origin 403');
for (const path of ['/rivet/actors', '/rivet/namespaces', '/api/rivet/start']) {
	const status = (await fetch(origin + path)).status;
	if (status !== 404) throw new Error(path + ' was exposed');
	console.log(path, status);
}
process.exit(0);
