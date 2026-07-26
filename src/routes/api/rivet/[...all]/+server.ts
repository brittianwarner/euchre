/**
 * Rivet serverless mount point.
 *
 * Every request Rivet Cloud makes to this deployment lands here:
 *   - `GET /api/rivet/metadata` — validates config, returns the public
 *     endpoint/namespace/token the browser uses to reach Rivet Cloud
 *   - `GET|POST /api/rivet/start` — runs an actor for the lifetime of the request
 *
 * In local `vite dev`, `vite-plugin-rivet-dev.ts` intercepts this path so the
 * binary start payload is not dropped by SvelteKit's empty-body quirk. This
 * route remains the production path on Vercel.
 *
 * @see https://rivet.dev/docs/connect/vercel
 */

import { registry } from '$lib/actors/registry';
import type { RequestHandler } from './$types';

const handle: RequestHandler = async ({ request }) => {
	return registry.handler(request);
};

export const GET = handle;
export const POST = handle;
export const PUT = handle;
export const DELETE = handle;
export const PATCH = handle;
export const OPTIONS = handle;
export const fallback = handle;
