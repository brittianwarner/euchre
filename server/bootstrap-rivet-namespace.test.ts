/**
 * Tests for the self-hosted engine bootstrap. All engine responses are
 * mocked through the fetchImpl seam — the empirical shapes (403 without
 * token, `name_not_unique`, minimal normal-pool PUT) come from the Phase 0
 * probes against rivetdev/engine:2.3.10 (2026-08-10).
 */
import { describe, expect, test } from "bun:test";
import {
  assertRivetEnvSafety,
  ENVOY_RUNNER_POOL_CONFIG,
  ensureRivetNamespace,
  RivetBootstrapFatalError,
} from "./bootstrap-rivet-namespace.js";

const OPTS = {
  endpoint: "http://engine:6420",
  token: "admin-token",
  namespace: "layerr-dev",
  runnerPool: "default",
  retryDelayMs: 1,
  sleep: () => Promise.resolve(),
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

/** Mock fetch over a table of method+path → response, recording calls. */
function mockFetch(table: Record<string, Response | (() => Response)>) {
  const calls: Array<{ method: string; url: string; body?: string }> = [];
  const impl = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const path = new URL(url).pathname;
    const key = `${method} ${path}`;
    calls.push({ method, url, body: init?.body as string | undefined });
    const entry = table[key];
    if (!entry) throw new Error(`unexpected call: ${key} (${url})`);
    return typeof entry === "function" ? entry() : entry;
  }) as typeof fetch;
  return { impl, calls };
}

describe("ensureRivetNamespace", () => {
  test("namespace exists → skips POST, asserts runner config", async () => {
    const { impl, calls } = mockFetch({
      "GET /namespaces": jsonResponse({
        namespaces: [{ name: "layerr-dev" }],
        pagination: { cursor: null },
      }),
      "PUT /runner-configs/default": jsonResponse({
        endpoint_config_changed: true,
      }),
    });
    const result = await ensureRivetNamespace({ ...OPTS, fetchImpl: impl });
    expect(result.namespaceCreated).toBe(false);
    expect(result.runnerConfigEnsured).toBe(true);
    expect(result.attempts).toBe(1);
    expect(calls.map((c) => `${c.method} ${new URL(c.url).pathname}`)).toEqual([
      "GET /namespaces",
      "PUT /runner-configs/default",
    ]);
    // Admin token rides as Bearer, never in the URL.
    for (const call of calls) expect(call.url).not.toContain("admin-token");
    const put = calls.find((c) => c.method === "PUT");
    expect(JSON.parse(put!.body!)).toEqual({
      datacenters: { default: { normal: ENVOY_RUNNER_POOL_CONFIG } },
    });
    expect(ENVOY_RUNNER_POOL_CONFIG.drain_on_version_upgrade).toBe(true);
    expect(ENVOY_RUNNER_POOL_CONFIG.actor_eviction_period).toBe(60);
    expect(ENVOY_RUNNER_POOL_CONFIG.actor_eviction_rate).toBe(1);
  });

  test("asserts the fixed services pool when Durable Streams is enabled", async () => {
    const { impl, calls } = mockFetch({
      "GET /namespaces": jsonResponse({
        namespaces: [{ name: "layerr-dev" }],
        pagination: { cursor: null },
      }),
      "PUT /runner-configs/default": jsonResponse({
        endpoint_config_changed: true,
      }),
      "PUT /runner-configs/services": jsonResponse({
        endpoint_config_changed: true,
      }),
    });
    const result = await ensureRivetNamespace({
      ...OPTS,
      additionalRunnerPools: ["services", "default", "services"],
      fetchImpl: impl,
    });

    expect(result.runnerConfigEnsured).toBe(true);
    expect(
      calls
        .filter((call) => call.method === "PUT")
        .map((call) => new URL(call.url).pathname),
    ).toEqual(["/runner-configs/default", "/runner-configs/services"]);
  });

  test("namespace missing → POSTs create, then runner config", async () => {
    const { impl, calls } = mockFetch({
      "GET /namespaces": jsonResponse({
        namespaces: [{ name: "default" }],
        pagination: { cursor: null },
      }),
      "POST /namespaces": jsonResponse({
        namespace: { name: "layerr-dev" },
      }),
      "PUT /runner-configs/default": jsonResponse({
        endpoint_config_changed: true,
      }),
    });
    const result = await ensureRivetNamespace({ ...OPTS, fetchImpl: impl });
    expect(result.namespaceCreated).toBe(true);
    const post = calls.find((c) => c.method === "POST");
    expect(JSON.parse(post!.body!)).toEqual({
      name: "layerr-dev",
      display_name: "layerr-dev",
    });
  });

  test("POST name_not_unique race is treated as success", async () => {
    const { impl } = mockFetch({
      "GET /namespaces": jsonResponse({
        namespaces: [],
        pagination: { cursor: null },
      }),
      "POST /namespaces": jsonResponse(
        {
          group: "namespace",
          code: "name_not_unique",
          message: "Namespace name must be unique.",
        },
        400,
      ),
      "PUT /runner-configs/default": jsonResponse({
        endpoint_config_changed: true,
      }),
    });
    const result = await ensureRivetNamespace({ ...OPTS, fetchImpl: impl });
    expect(result.runnerConfigEnsured).toBe(true);
  });

  test("follows namespace pagination cursor", async () => {
    let getCalls = 0;
    const { impl } = mockFetch({
      "GET /namespaces": () => {
        getCalls++;
        return getCalls === 1
          ? jsonResponse({
              namespaces: [{ name: "default" }],
              pagination: { cursor: "abc" },
            })
          : jsonResponse({
              namespaces: [{ name: "layerr-dev" }],
              pagination: { cursor: null },
            });
      },
      "PUT /runner-configs/default": jsonResponse({
        endpoint_config_changed: true,
      }),
    });
    const result = await ensureRivetNamespace({ ...OPTS, fetchImpl: impl });
    expect(result.namespaceCreated).toBe(false);
    expect(getCalls).toBe(2);
  });

  test("403 throws fatally without retrying", async () => {
    const { impl, calls } = mockFetch({
      "GET /namespaces": jsonResponse(
        { group: "api", code: "forbidden", message: "Access denied" },
        403,
      ),
    });
    await expect(
      ensureRivetNamespace({ ...OPTS, fetchImpl: impl }),
    ).rejects.toBeInstanceOf(RivetBootstrapFatalError);
    expect(calls).toHaveLength(1);
  });

  test("transient 5xx retries until success", async () => {
    let getCalls = 0;
    const { impl } = mockFetch({
      "GET /namespaces": () => {
        getCalls++;
        if (getCalls < 3) return jsonResponse({ message: "booting" }, 503);
        return jsonResponse({
          namespaces: [{ name: "layerr-dev" }],
          pagination: { cursor: null },
        });
      },
      "PUT /runner-configs/default": jsonResponse({
        endpoint_config_changed: true,
      }),
    });
    const result = await ensureRivetNamespace({ ...OPTS, fetchImpl: impl });
    expect(result.attempts).toBe(3);
  });

  test("deadline exhaustion throws with attempt count", async () => {
    const { impl } = mockFetch({
      "GET /namespaces": jsonResponse({ message: "down" }, 502),
    });
    await expect(
      ensureRivetNamespace({
        ...OPTS,
        fetchImpl: impl,
        timeoutMs: 5,
      }),
    ).rejects.toThrow(/did not accept .* within 5ms/);
  });
});

describe("assertRivetEnvSafety", () => {
  test("cloud mode allows userinfo endpoints (today's config)", () => {
    expect(() =>
      assertRivetEnvSafety({
        RIVET_ENDPOINT: "https://ns:sk_secret@api.rivet.dev",
        RIVET_PUBLIC_ENDPOINT: "https://ns:pk_public@api.rivet.dev",
        selfhosted: false,
      }),
    ).not.toThrow();
  });

  test("always-on: RIVET_TOKEN value inside RIVET_PUBLIC_ENDPOINT throws", () => {
    expect(() =>
      assertRivetEnvSafety({
        RIVET_TOKEN: "super-secret",
        RIVET_PUBLIC_ENDPOINT: "https://ns:super-secret@api.rivet.dev",
        selfhosted: false,
      }),
    ).toThrow(/RIVET_PUBLIC_ENDPOINT contains the RIVET_TOKEN value/);
  });

  test("selfhost: missing token/namespace/endpoint throws naming all three", () => {
    expect(() =>
      assertRivetEnvSafety({ selfhosted: true }),
    ).toThrow(/RIVET_TOKEN.*RIVET_NAMESPACE.*RIVET_ENDPOINT/);
  });

  test("selfhost: RIVET_ENDPOINT userinfo throws", () => {
    expect(() =>
      assertRivetEnvSafety({
        RIVET_ENDPOINT: "http://default:admin@engine:6420",
        RIVET_TOKEN: "admin",
        RIVET_NAMESPACE: "default",
        selfhosted: true,
      }),
    ).toThrow(/RIVET_ENDPOINT embeds URL userinfo/);
  });

  test("selfhost: RIVET_PUBLIC_ENDPOINT userinfo throws on non-Cloud hosts", () => {
    expect(() =>
      assertRivetEnvSafety({
        RIVET_ENDPOINT: "http://engine:6420",
        RIVET_PUBLIC_ENDPOINT: "https://u:p@api.layerr.ai/rivet",
        RIVET_TOKEN: "admin",
        RIVET_NAMESPACE: "default",
        selfhosted: true,
      }),
    ).toThrow(/RIVET_PUBLIC_ENDPOINT embeds URL userinfo/);
  });

  test("selfhost: Cloud userinfo public endpoint allowed (Phase 2 transition)", () => {
    expect(() =>
      assertRivetEnvSafety({
        RIVET_ENDPOINT: "http://engine:6420",
        RIVET_PUBLIC_ENDPOINT: "https://my-namespace:pk_public@api.rivet.dev",
        RIVET_TOKEN: "admin",
        RIVET_NAMESPACE: "default",
        selfhosted: true,
      }),
    ).not.toThrow();
    expect(() =>
      assertRivetEnvSafety({
        RIVET_ENDPOINT: "http://engine:6420",
        RIVET_PUBLIC_ENDPOINT:
          "https://my-namespace:pk_public@api-us-east-1.rivet.dev",
        RIVET_TOKEN: "admin",
        RIVET_NAMESPACE: "default",
        selfhosted: true,
      }),
    ).not.toThrow();
  });

  test("selfhost: clean config passes", () => {
    expect(() =>
      assertRivetEnvSafety({
        RIVET_ENDPOINT: "http://rivet-engine.railway.internal:6420",
        RIVET_PUBLIC_ENDPOINT: "https://api.layerr.ai/rivet",
        RIVET_TOKEN: "admin",
        RIVET_NAMESPACE: "default",
        selfhosted: true,
      }),
    ).not.toThrow();
  });
});
