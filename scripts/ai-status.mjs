import { createClient } from 'rivetkit/client';
const client = createClient({ endpoint: 'http://127.0.0.1:6420', devtools: false });
const statuses = await Promise.all(
	[1, 2, 3].map((seat) =>
		client.aiSeat.get(['table', process.argv[2], 'seat', String(seat)]).getStatus()
	)
);
console.log(JSON.stringify(statuses, null, 2));
