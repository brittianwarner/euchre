import "./runes-shim.js";
import { afterEach, describe, expect, test, vi, beforeEach } from "vitest";
import type { ActorConnStatus } from "rivetkit/client";

// ---------------------------------------------------------------------------
// Mock — identical shape to reactive-actor.test.ts, but with async actions
// ---------------------------------------------------------------------------

const frameworkMock = vi.hoisted(() => {
  type Listener = (...args: unknown[]) => void;
  type Subscriber = (value: { currentVal: MockActorState }) => void;
  type MockConnection = {
    id: string;
    ping: () => string;
    increment: (amount: number) => Promise<number>;
    failAction: () => Promise<never>;
    slowAction: () => Promise<string>;
    syncValue: () => number;
    syncThrow: () => never;
    on: (eventName: string, handler: Listener) => () => void;
    emit: (eventName: string, ...args: unknown[]) => void;
  };
  type MockActorState = {
    connection: MockConnection;
    handle: { id: string };
    connStatus: ActorConnStatus;
    error: Error | null;
    hash: string;
  };

  const subscribers = new Set<Subscriber>();

  function createConnection(id: string): MockConnection {
    const listeners = new Map<string, Set<Listener>>();

    return {
      id,
      ping: () => `pong:${id}`,
      increment: vi.fn(async (amount: number) => amount + 1),
      failAction: vi.fn(async () => {
        throw new Error("action failed");
      }),
      slowAction: vi.fn(
        () =>
          new Promise<string>((resolve) =>
            setTimeout(() => resolve("done"), 5_000),
          ),
      ),
      syncValue: vi.fn(() => 42),
      syncThrow: vi.fn(() => {
        throw new Error("sync action failed");
      }),
      on(eventName: string, handler: Listener) {
        let eventListeners = listeners.get(eventName);
        if (!eventListeners) {
          eventListeners = new Set();
          listeners.set(eventName, eventListeners);
        }
        eventListeners.add(handler);
        return () => eventListeners?.delete(handler);
      },
      emit(eventName: string, ...args: unknown[]) {
        listeners.get(eventName)?.forEach((listener) => listener(...args));
      },
    };
  }

  let currentState: MockActorState;

  const getOrCreateActor = vi.fn(() => ({
    mount: vi.fn(() => vi.fn()),
    state: {
      get state() {
        return currentState;
      },
      subscribe(callback: Subscriber) {
        subscribers.add(callback);
        return () => subscribers.delete(callback);
      },
    },
  }));

  function push(next: Partial<MockActorState>): void {
    currentState = { ...currentState, ...next };
    subscribers.forEach((subscriber) =>
      subscriber({ currentVal: currentState }),
    );
  }

  function reset(): void {
    subscribers.clear();
    currentState = {
      connection: createConnection("one"),
      handle: { id: "handle-one" },
      connStatus: "connected",
      error: null,
      hash: "hash-one",
    };
    getOrCreateActor.mockClear();
  }

  reset();

  return {
    getOrCreateActor,
    currentState: () => currentState,
    push,
    reset,
    createConnection,
  };
});

vi.mock("@rivetkit/framework-base", () => ({
  createRivetKit: vi.fn(() => ({
    getOrCreateActor: frameworkMock.getOrCreateActor,
  })),
}));

import { createRivetKitWithClient } from "../rivetkit.svelte.js";

// ---------------------------------------------------------------------------
// Tests — action middleware via actionDefaults
// ---------------------------------------------------------------------------

describe("action middleware (createReactiveActor)", () => {
  beforeEach(() => {
    frameworkMock.reset();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  test("without actionDefaults, actions are plain pass-through (no tracking)", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
    });
    actor.mount();

    // Action tracking state has defaults but no interceptor
    expect(actor.isMutating).toBe(false);
    expect(actor.pendingActions).toBe(0);
    expect(actor.lastActionError).toBe(null);
    expect(actor.lastAction).toBe(null);

    // Actions pass through directly — no interception
    const result = await actor.increment(5);
    expect(result).toBe(6);

    // No tracking occurred (no actionDefaults configured)
    expect(actor.isMutating).toBe(false);
    expect(actor.lastAction).toBe(null);
  });

  test("with actionDefaults, tracks isMutating and pendingActions", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {},
    });
    actor.mount();

    expect(actor.isMutating).toBe(false);
    expect(actor.pendingActions).toBe(0);

    const promise = actor.increment(5);
    // Synchronously after calling, state is updated
    expect(actor.isMutating).toBe(true);
    expect(actor.pendingActions).toBe(1);
    expect(actor.lastAction).toBe("increment");

    await promise;

    expect(actor.isMutating).toBe(false);
    expect(actor.pendingActions).toBe(0);
  });

  test("captures errors to lastActionError (throwOnError: false default)", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {},
    });
    actor.mount();

    // Call an action that throws
    const result = await actor.failAction();

    // Error captured reactively, not thrown
    expect(result).toBeUndefined();
    expect(actor.lastActionError).toBeInstanceOf(Error);
    expect(actor.lastActionError?.message).toBe("action failed");
    expect(actor.lastAction).toBe("failAction");
    expect(actor.isMutating).toBe(false);
  });

  test("captures synchronous action throws and clears pending state", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {},
    });
    actor.mount();

    const result = await actor.syncThrow();

    expect(result).toBeUndefined();
    expect(actor.lastActionError?.message).toBe("sync action failed");
    expect(actor.pendingActions).toBe(0);
    expect(actor.isMutating).toBe(false);
  });

  test("supports synchronous non-Promise action results", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {},
    });
    actor.mount();

    const result = await actor.syncValue();

    expect(result).toBe(42);
    expect(actor.pendingActions).toBe(0);
    expect(actor.isMutating).toBe(false);
  });

  test("clears lastActionError on next successful action", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {},
    });
    actor.mount();

    await actor.failAction();
    expect(actor.lastActionError).not.toBe(null);

    await actor.increment(1);
    expect(actor.lastActionError).toBe(null);
  });

  test("throwOnError: true re-throws the error", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: { throwOnError: true },
    });
    actor.mount();

    await expect(actor.failAction()).rejects.toThrow("action failed");
    // Error is still captured reactively even when thrown
    expect(actor.lastActionError?.message).toBe("action failed");
  });

  test("throwOnError as function — called per error to decide", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {
        throwOnError: (_err: Error, actionName: string) =>
          actionName === "failAction",
      },
    });
    actor.mount();

    // failAction should throw (function returns true for it)
    await expect(actor.failAction()).rejects.toThrow("action failed");
  });

  test("timeout causes action to fail", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: { timeout: 100 },
    });
    actor.mount();

    const promise = actor.slowAction();

    // Advance past the timeout
    vi.advanceTimersByTime(150);

    const result = await promise;
    expect(result).toBeUndefined();
    expect(actor.lastActionError?.message).toContain("timed out");
    expect(actor.isMutating).toBe(false);
  });

  test("resetActionState clears error and lastAction", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {},
    });
    actor.mount();

    await actor.failAction();
    expect(actor.lastActionError).not.toBe(null);
    expect(actor.lastAction).toBe("failAction");

    actor.resetActionState();
    expect(actor.lastActionError).toBe(null);
    expect(actor.lastAction).toBe(null);
  });

  test("lifecycle callbacks fire in order", async () => {
    const log: string[] = [];
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {
        onActionStart: (name: string) => log.push(`start:${name}`),
        onActionSuccess: (name: string) => log.push(`success:${name}`),
        onActionError: (_err: Error, name: string) => log.push(`error:${name}`),
        onActionSettled: (name: string) => log.push(`settled:${name}`),
      },
    });
    actor.mount();

    await actor.increment(5);
    expect(log).toEqual([
      "start:increment",
      "success:increment",
      "settled:increment",
    ]);

    log.length = 0;
    await actor.failAction();
    expect(log).toEqual([
      "start:failAction",
      "error:failAction",
      "settled:failAction",
    ]);
  });

  test("connection guard rejects when disconnected", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: { guardConnection: true },
    });
    actor.mount();

    // Simulate disconnection
    frameworkMock.push({
      connection: null as never,
      connStatus: "disconnected",
    });

    const result = await actor.increment(5);
    expect(result).toBeUndefined();
    expect(actor.lastActionError?.message).toContain("disconnected");
  });

  test("connection guard checks status even when connection object exists", async () => {
    const rivet = createRivetKitWithClient({} as never);
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: { guardConnection: true },
    });
    actor.mount();

    frameworkMock.push({ connStatus: "connecting" });

    const result = await actor.increment(5);
    expect(result).toBeUndefined();
    expect(actor.lastActionError?.message).toContain("not yet connected");
    expect((actor.lastActionError as { code?: string } | null)?.code).toBe(
      "ACTOR_NOT_YET_CONNECTED",
    );
  });

  test("client-level actionDefaults cascade to actor-level", async () => {
    const clientLog: string[] = [];
    const rivet = createRivetKitWithClient({} as never, {
      actionDefaults: {
        onActionStart: (name: string) => clientLog.push(`client:${name}`),
        timeout: 60_000,
      },
    });

    const actorLog: string[] = [];
    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {
        // Override onActionStart (actor-level wins)
        onActionStart: (name: string) => actorLog.push(`actor:${name}`),
      },
    });
    actor.mount();

    await actor.increment(5);

    // Actor-level overrode onActionStart
    expect(clientLog).toEqual([]);
    expect(actorLog).toEqual(["actor:increment"]);
  });

  test("concurrent actions track pendingActions correctly", async () => {
    const rivet = createRivetKitWithClient({} as never);

    // Replace increment with a delayed mock
    const conn = frameworkMock.currentState().connection;
    let resolveFirst: ((v: number) => void) | undefined;
    let resolveSecond: ((v: number) => void) | undefined;
    let callCount = 0;

    conn.increment = vi.fn(
      () =>
        new Promise<number>((resolve) => {
          callCount++;
          if (callCount === 1) resolveFirst = resolve;
          else resolveSecond = resolve;
        }),
    );

    const actor = rivet.createReactiveActor({
      name: "chat" as never,
      key: ["room-1"],
      actionDefaults: {},
    });
    actor.mount();

    const p1 = actor.increment(1);
    expect(actor.pendingActions).toBe(1);

    const p2 = actor.increment(2);
    expect(actor.pendingActions).toBe(2);
    expect(actor.isMutating).toBe(true);

    resolveFirst!(2);
    await p1;
    expect(actor.pendingActions).toBe(1);
    expect(actor.isMutating).toBe(true);

    resolveSecond!(3);
    await p2;
    expect(actor.pendingActions).toBe(0);
    expect(actor.isMutating).toBe(false);
  });
});
