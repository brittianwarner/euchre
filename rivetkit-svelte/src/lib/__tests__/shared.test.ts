import "./runes-shim.js";
import { describe, expect, test, vi } from "vitest";
import type { AnyActorRegistry } from "../index.js";
import {
  createReactiveConnection,
  createSharedRivetKit,
  withActorParams,
  getActionError,
} from "../index.js";
import { createMockConnection } from "./helpers.js";

describe("shared helpers", () => {
  test("createSharedRivetKit reuses a single wrapper", () => {
    const client = { id: "client" } as never;
    let clientCalls = 0;

    const getRivet = createSharedRivetKit<AnyActorRegistry>(() => {
      clientCalls += 1;
      return client;
    });

    const a = getRivet();
    const b = getRivet();

    expect(a).toBe(b);
    expect(clientCalls).toBe(1);
  });

  test("withActorParams merges static and getter params", () => {
    let token = "first";

    const getOpts = withActorParams<AnyActorRegistry, never>(
      {
        name: "chat" as never,
        key: ["room-1"],
        params: { organizationId: "org-1" },
      },
      () => ({ token }),
    );

    expect(getOpts()).toEqual({
      name: "chat",
      key: ["room-1"],
      params: { organizationId: "org-1", token: "first" },
    });

    token = "second";

    expect(getOpts().params).toEqual({
      organizationId: "org-1",
      token: "second",
    });
  });

  test("withActorParams omits params when both inputs are undefined", () => {
    const getOpts = withActorParams<AnyActorRegistry, never>(
      {
        name: "chat" as never,
        key: ["room-1"],
      },
      () => undefined,
    );

    expect(getOpts()).toEqual({
      name: "chat",
      key: ["room-1"],
    });
  });

  test("createReactiveConnection reflects status, errors, and events", async () => {
    const mock = createMockConnection();
    const reactive = createReactiveConnection({
      connect: () => mock.connection,
    });

    expect(reactive.connStatus).toBe("idle");
    expect(reactive.isConnected).toBe(false);

    reactive.connect();
    mock.setStatus("connected");

    expect(reactive.connStatus).toBe("connected");
    expect(reactive.isConnected).toBe(true);

    let payload: string | null = null;
    const unsubscribe = reactive.onEvent("message", (value) => {
      payload = value as string;
    });

    mock.emit("message", "hello");
    expect(payload).toBe("hello");

    mock.emitError("boom");
    expect(reactive.error?.message).toBe("boom");

    unsubscribe();
    await reactive.dispose();

    expect(reactive.connStatus).toBe("disconnected");
    expect(reactive.connection).toBe(null);
  });

  test("whenConnected resolves true when status becomes connected", async () => {
    const mock = createMockConnection();
    const reactive = createReactiveConnection({
      connect: () => mock.connection,
    });

    reactive.connect();

    const promise = reactive.whenConnected(5_000);
    mock.setStatus("connected");

    const result = await promise;
    expect(result).toBe(true);
  });

  test("whenConnected resolves false on timeout", async () => {
    vi.useFakeTimers();

    const mock = createMockConnection();
    const reactive = createReactiveConnection({
      connect: () => mock.connection,
    });

    reactive.connect();

    const promise = reactive.whenConnected(100);
    vi.advanceTimersByTime(150);

    const result = await promise;
    expect(result).toBe(false);

    vi.useRealTimers();
  });

  test("disconnect cancels pending whenConnected with false", async () => {
    const mock = createMockConnection();
    const reactive = createReactiveConnection({
      connect: () => mock.connection,
    });

    reactive.connect();

    const promise = reactive.whenConnected(30_000);
    await reactive.disconnect();

    const result = await promise;
    expect(result).toBe(false);
  });
});

describe("getActionError", () => {
  test("returns null when no error", () => {
    const result = getActionError({ lastActionError: null });
    expect(result).toBe(null);
  });

  test("returns null for undefined error", () => {
    const result = getActionError({ lastActionError: undefined });
    expect(result).toBe(null);
  });

  test("extracts message from plain Error", () => {
    const result = getActionError({
      lastActionError: new Error("something broke"),
    });
    expect(result).not.toBe(null);
    expect(result!.message).toBe("something broke");
    expect(result!.code).toBeUndefined();
    expect(result!.isActorError).toBe(false);
  });

  test("extracts code and message from ActorError-like object", () => {
    const err = Object.assign(new Error("rate limited"), {
      __type: "ActorError" as const,
      group: "client",
      code: "RATE_LIMITED",
    });
    const result = getActionError({ lastActionError: err });
    expect(result).not.toBe(null);
    expect(result!.message).toBe("rate limited");
    expect(result!.code).toBe("RATE_LIMITED");
    expect(result!.isActorError).toBe(true);
  });
});
