/**
 * Self-hosted rivet-engine bootstrap + env containment guards.
 *
 * When `RIVET_ENGINE_SELFHOSTED=true` the api boots against a Layerr-owned
 * rivet-engine (Railway private networking) instead of Rivet Cloud. Two
 * things must be true before the envoy runner dials out:
 *
 *   1. The configured namespace exists on the engine. A fresh engine
 *      auto-creates only `default` (verified rivetdev/engine:2.3.10).
 *   2. Runner-pool configs exist for the API runner (`RIVET_POOL` ??
 *      "default") and any enabled service runners. Rivet Services registers
 *      its durableStream actors in the fixed `services` pool; without that
 *      second config every create fails with `no_runner_config_configured`.
 *
 * Rivet Cloud managed both through the portal — the portal-only runner-pool
 * config was a past dev-outage root cause, and a previous attempt to push
 * pool config from app code (`serverless.configureRunnerPool`) failed
 * SILENTLY on token permissions and left every actor scheduling
 * `{"crashed":{"message":null}}`. That lesson shapes this module:
 *
 *   - Idempotent: GET-before-POST, `name_not_unique` treated as success —
 *      both api replicas boot through this concurrently.
 *   - Fail-loud: transient errors retry until a deadline (engine cold start
 *      races api boot on deploy), then THROW and kill boot. Auth failures
 *      (401/403) throw immediately — retrying a bad token is pointless.
 *      A silent bootstrap failure is strictly worse than a crashed deploy.
 *
 * Admin API shapes verified against rivetdev/engine:2.3.10 (Phase 0,
 * 2026-08-10): all admin routes require `Authorization: Bearer <admin
 * token>` (403 without); POST /namespaces takes `{name, display_name}`;
 * PUT /runner-configs/{pool}?namespace={ns} takes
 * `{datacenters: {default: {normal: ENVOY_RUNNER_POOL_CONFIG}}}` (the
 * envoy/normal pool variant — request_lifespan/slots_per_runner are
 * serverless-pool concepts and do not apply to envoy runners). An empty
 * `normal: {}` is a full replace back to unthrottled version-upgrade drain.
 */

/** Default ceiling for the whole bootstrap (engine cold-start race). */
const DEFAULT_TIMEOUT_MS = 60_000;
/** Delay between attempts after a transient failure. */
const DEFAULT_RETRY_DELAY_MS = 1_000;

/**
 * Envoy (`normal`) runner-pool body asserted on every API boot.
 *
 * `PUT /runner-configs/{pool}` is a full replace. An empty `normal: {}`
 * restores Rivet defaults: `drain_on_version_upgrade: true` with
 * `actor_eviction_period: 0`, so the first replica of a new
 * `RIVET_ENVOY_VERSION` immediately drains every older runner — before
 * Railway has marked the new deployment healthy and before the other
 * replicas have finished `startAndWait()`. Rate-limit eviction so the
 * new fleet can finish registering before the old one is emptied.
 *
 * HTTP API is snake_case. Fields match `RegistryConfigRequest.normal` in
 * the vendored engine client (`api-endpoints.ts`). Do not send
 * `actor_eviction_delay`: 2.3.13's normal-pool schema does not include it.
 *
 * @see https://rivet.dev/actors/docs/general/pool-configuration/
 */
export const ENVOY_RUNNER_POOL_CONFIG = {
  drain_on_version_upgrade: true,
  actor_eviction_period: 60,
  actor_eviction_rate: 1,
} as const;

export interface EnsureRivetNamespaceOptions {
  /** Bare engine URL, e.g. `http://rivet-engine.railway.internal:6420`. No userinfo. */
  endpoint: string;
  /** Engine admin token (`RIVET__AUTH__ADMIN_TOKEN` on the engine service). */
  token: string;
  /** Namespace to ensure, e.g. `default` or `layerr-dev`. */
  namespace: string;
  /** Runner pool the envoy runner registers under (`RIVET_POOL` ?? "default"). */
  runnerPool: string;
  /** Additional fixed service-runner pools to assert in the same namespace. */
  additionalRunnerPools?: readonly string[];
  /** Overall deadline for the retry loop. */
  timeoutMs?: number;
  /** Delay between attempts. */
  retryDelayMs?: number;
  /** Test seam. */
  fetchImpl?: typeof fetch;
  /** Test seam. */
  sleep?: (ms: number) => Promise<void>;
}

export interface EnsureRivetNamespaceResult {
  /** True when the namespace had to be created this boot. */
  namespaceCreated: boolean;
  /** Runner-pool config was (re)asserted — always true on success. */
  runnerConfigEnsured: boolean;
  /** How many attempts the retry loop took. */
  attempts: number;
}

/** Bootstrap failed in a way retrying cannot fix (auth, schema). */
export class RivetBootstrapFatalError extends Error {}

interface EngineErrorBody {
  group?: string;
  code?: string;
  message?: string;
}

function authHeaders(token: string): Record<string, string> {
  return { Authorization: `Bearer ${token}` };
}

async function readEngineError(res: Response): Promise<string> {
  try {
    const body = (await res.json()) as EngineErrorBody;
    return `${res.status} ${body.group ?? "api"}.${body.code ?? "error"}: ${body.message ?? res.statusText}`;
  } catch {
    return `${res.status} ${res.statusText}`;
  }
}

async function ensureNamespaceOnce(
  opts: Required<Pick<EnsureRivetNamespaceOptions, "endpoint" | "token" | "namespace">> &
    Pick<EnsureRivetNamespaceOptions, "runnerPool" | "additionalRunnerPools"> &
    Pick<EnsureRivetNamespaceOptions, "fetchImpl">,
): Promise<{ namespaceCreated: boolean }> {
  const doFetch = opts.fetchImpl ?? fetch;
  const base = opts.endpoint.replace(/\/+$/, "");
  const headers = authHeaders(opts.token);

  // --- Namespace: list, create only if missing. Pages are bounded — a
  // self-hosted engine holds a handful of namespaces — but follow the cursor
  // so a name landing on page 2 is never "re-created".
  let found = false;
  let cursor: string | null = null;
  for (let page = 0; page < 20 && !found; page++) {
    const url = cursor
      ? `${base}/namespaces?cursor=${encodeURIComponent(cursor)}`
      : `${base}/namespaces`;
    const res = await doFetch(url, { headers });
    if (res.status === 401 || res.status === 403) {
      throw new RivetBootstrapFatalError(
        `engine admin auth rejected the token on GET /namespaces (${await readEngineError(res)}) — check RIVET_TOKEN matches the engine's RIVET__AUTH__ADMIN_TOKEN`,
      );
    }
    if (!res.ok) {
      throw new Error(`GET /namespaces failed: ${await readEngineError(res)}`);
    }
    const body = (await res.json()) as {
      namespaces?: Array<{ name: string }>;
      pagination?: { cursor?: string | null };
    };
    found = (body.namespaces ?? []).some((ns) => ns.name === opts.namespace);
    cursor = body.pagination?.cursor ?? null;
    if (!cursor) break;
  }

  let namespaceCreated = false;
  if (!found) {
    const res = await doFetch(`${base}/namespaces`, {
      method: "POST",
      headers: { ...headers, "Content-Type": "application/json" },
      body: JSON.stringify({
        name: opts.namespace,
        display_name: opts.namespace,
      }),
    });
    if (res.ok) {
      namespaceCreated = true;
    } else {
      const body = (await res.json().catch(() => ({}))) as EngineErrorBody;
      // Concurrent replica boot (or a list/create race) — the namespace now
      // exists, which is all we needed.
      if (body.code !== "name_not_unique") {
        if (res.status === 401 || res.status === 403) {
          throw new RivetBootstrapFatalError(
            `engine admin auth rejected the token on POST /namespaces (${res.status}) — check RIVET_TOKEN`,
          );
        }
        throw new Error(
          `POST /namespaces failed: ${res.status} ${body.message ?? res.statusText}`,
        );
      }
    }
  }

  // --- Runner pools: envoy ("normal") config with paced version-upgrade
  // eviction. Verified shape for rivetdev/engine:2.3.13 and 2.3.17 (the
  // engine api-types runner-config schema is identical between the two); serverless-only
  // fields (request_lifespan, slots_per_runner, min/max runners) are Rivet
  // Cloud portal concepts and deliberately NOT pushed here.
  const runnerPools = [opts.runnerPool, ...(opts.additionalRunnerPools ?? [])]
    .map((pool) => pool.trim())
    .filter((pool, index, pools) => pool.length > 0 && pools.indexOf(pool) === index);
  for (const runnerPool of runnerPools) {
    const put = await doFetch(
      `${base}/runner-configs/${encodeURIComponent(runnerPool)}?namespace=${encodeURIComponent(opts.namespace)}`,
      {
        method: "PUT",
        headers: { ...headers, "Content-Type": "application/json" },
        body: JSON.stringify({
          datacenters: { default: { normal: ENVOY_RUNNER_POOL_CONFIG } },
        }),
      },
    );
    if (put.status === 401 || put.status === 403) {
      throw new RivetBootstrapFatalError(
        `engine admin auth rejected the token on PUT /runner-configs/${runnerPool} (${await readEngineError(put)}) — check RIVET_TOKEN`,
      );
    }
    if (!put.ok) {
      throw new Error(
        `PUT /runner-configs/${runnerPool} failed: ${await readEngineError(put)}`,
      );
    }
  }

  return { namespaceCreated };
}

/**
 * Ensure the namespace + runner pool exist on the self-hosted engine,
 * retrying transient failures until `timeoutMs`, then throwing. Call once at
 * boot AFTER the HTTP server starts (so /healthz keeps answering during an
 * engine cold start) and BEFORE `registry.startEnvoy()` (so the runner never
 * dials an unbootstrapped namespace).
 */
export async function ensureRivetNamespace(
  opts: EnsureRivetNamespaceOptions,
): Promise<EnsureRivetNamespaceResult> {
  const timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const retryDelayMs = opts.retryDelayMs ?? DEFAULT_RETRY_DELAY_MS;
  const sleep =
    opts.sleep ?? ((ms: number) => new Promise((r) => setTimeout(r, ms)));
  const deadline = Date.now() + timeoutMs;

  let attempts = 0;
  let lastError: Error | null = null;
  while (Date.now() < deadline) {
    attempts++;
    try {
      const { namespaceCreated } = await ensureNamespaceOnce({
        endpoint: opts.endpoint,
        token: opts.token,
        namespace: opts.namespace,
        runnerPool: opts.runnerPool,
        additionalRunnerPools: opts.additionalRunnerPools,
        fetchImpl: opts.fetchImpl,
      });
      return { namespaceCreated, runnerConfigEnsured: true, attempts };
    } catch (err) {
      if (err instanceof RivetBootstrapFatalError) throw err;
      lastError = err instanceof Error ? err : new Error(String(err));
      await sleep(retryDelayMs);
    }
  }
  throw new Error(
    `rivet bootstrap: engine at ${opts.endpoint} did not accept namespace/runner-config provisioning within ${timeoutMs}ms (${attempts} attempts): ${lastError?.message ?? "unknown"}`,
  );
}

/**
 * Boot-time env containment guards for the Rivet surface.
 *
 *   - ALWAYS (both Cloud and self-host): the secret RIVET_TOKEN value must
 *     never appear inside RIVET_PUBLIC_ENDPOINT — that string is shipped
 *     verbatim to every browser via /api/client-config. This is the one
 *     config mistake that would leak the engine admin token platform-wide.
 *   - SELF-HOST only: RIVET_TOKEN + RIVET_NAMESPACE are required and
 *     RIVET_ENDPOINT must be a bare URL (no userinfo) — the vendored
 *     endpoint-parser hard-errors when credentials appear in BOTH the URL
 *     and the separate vars, and bare endpoints keep logged endpoint strings
 *     token-free. RIVET_PUBLIC_ENDPOINT must also be userinfo-free once it
 *     points at the self-hosted /rivet path (that path carries no
 *     Rivet-native token at all — the OSS engine validates nothing; Layerr's
 *     HMAC actor tokens remain the browser boundary). Userinfo is tolerated
 *     ONLY while it still points at a rivet.dev host — the documented
 *     Phase 2 transition state (server self-hosted, browsers still on the
 *     Cloud pk_ endpoint).
 *
 * Cloud mode deliberately keeps allowing userinfo in BOTH endpoints — the
 * `namespace:token@api.rivet.dev` style is the documented Cloud config.
 */
export function assertRivetEnvSafety(env: {
  RIVET_ENDPOINT?: string;
  RIVET_PUBLIC_ENDPOINT?: string;
  RIVET_TOKEN?: string;
  RIVET_NAMESPACE?: string;
  selfhosted: boolean;
}): void {
  if (
    env.RIVET_TOKEN &&
    env.RIVET_PUBLIC_ENDPOINT &&
    env.RIVET_PUBLIC_ENDPOINT.includes(env.RIVET_TOKEN)
  ) {
    throw new Error(
      "api boot: RIVET_PUBLIC_ENDPOINT contains the RIVET_TOKEN value — this string is shipped verbatim to every browser via /api/client-config; refusing to start",
    );
  }

  if (!env.selfhosted) return;

  const missing: string[] = [];
  if (!env.RIVET_TOKEN) missing.push("RIVET_TOKEN");
  if (!env.RIVET_NAMESPACE) missing.push("RIVET_NAMESPACE");
  if (!env.RIVET_ENDPOINT) missing.push("RIVET_ENDPOINT");
  if (missing.length > 0) {
    throw new Error(
      `api boot: RIVET_ENGINE_SELFHOSTED=true but ${missing.join(", ")} ${missing.length === 1 ? "is" : "are"} unset — the self-hosted engine requires the admin token, an explicit namespace, and the private engine URL`,
    );
  }

  const endpoint = new URL(env.RIVET_ENDPOINT as string);
  if (endpoint.username || endpoint.password) {
    throw new Error(
      "api boot: RIVET_ENGINE_SELFHOSTED=true but RIVET_ENDPOINT embeds URL userinfo — keep the endpoint bare and pass the admin token via RIVET_TOKEN only (the endpoint-parser rejects credentials in both places, and bare endpoints keep logs token-free)",
    );
  }

  if (env.RIVET_PUBLIC_ENDPOINT) {
    const publicEndpoint = new URL(env.RIVET_PUBLIC_ENDPOINT);
    // Userinfo is allowed ONLY while the public endpoint still points at
    // Rivet Cloud (api.rivet.dev / api-<region>.rivet.dev) — the documented
    // transitional Phase 2 state (server-side self-hosted, browsers still on
    // Cloud with the pk_ endpoint). Once the public path flips to the
    // self-hosted /rivet proxy it must be bare: the OSS engine validates
    // nothing, so the public path carries no Rivet-native credential at
    // all — Layerr HMAC actor tokens are the browser boundary.
    const isCloudHost =
      publicEndpoint.hostname === "api.rivet.dev" ||
      publicEndpoint.hostname.endsWith(".rivet.dev");
    if ((publicEndpoint.username || publicEndpoint.password) && !isCloudHost) {
      throw new Error(
        "api boot: RIVET_ENGINE_SELFHOSTED=true but RIVET_PUBLIC_ENDPOINT embeds URL userinfo on a non-Rivet-Cloud host — the self-hosted public path (https://<api-origin>/rivet) must be bare; userinfo is only tolerated while the endpoint still points at Rivet Cloud during the Phase 2 transition",
      );
    }
  }
}
