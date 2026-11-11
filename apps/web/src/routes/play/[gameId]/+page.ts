/**
 * Play table is client-only: Rivet WebSocket + useActor need the browser.
 *
 * Actor resolution is retried by the client, so a temporary engine outage never
 * replaces a saved game with an error page. `POST /api/table-session/{gameId}`
 * refreshes the recent-tables cookie for this table.
 */

import type { PageLoad } from './$types';

export const ssr = false;

export const load: PageLoad = ({ params }) => ({ gameId: params.gameId });
