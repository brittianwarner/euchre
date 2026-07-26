/**
 * Shared RivetKit helpers for provider-level setup and mixed reactive/raw usage.
 *
 * These helpers codify the shared-client patterns common in SvelteKit apps:
 * one transport, one RivetKit wrapper, many reactive and raw consumers.
 *
 * @module
 */

import type { ActorOptions, AnyActorRegistry } from "@rivetkit/framework-base";
import type {
  ActorConn,
  ActorConnStatus,
  AnyActorDefinition,
  Client,
  ExtractActorsFromRegistry,
} from "rivetkit/client";
import {
  createRivetKitWithClient,
  type RivetKit,
  type SvelteRivetKitOptions,
} from "./rivetkit.svelte.js";
import type { MaybeGetter } from "./internal/types.js";
import { extract } from "./internal/extract.js";

/** Lazily create and reuse a single RivetKit wrapper for a shared client factory. */
export function createSharedRivetKit<Registry extends AnyActorRegistry>(
  getClient: () => Client<Registry>,
  opts?: SvelteRivetKitOptions<Registry>,
): () => RivetKit<Registry> {
  let rivet: RivetKit<Registry> | undefined;

  return () => {
    if (!rivet) {
      rivet = createRivetKitWithClient<Registry>(
        getClient(),
        opts,
      ) as RivetKit<Registry>;
    }
    return rivet!;
  };
}

/**
 * Merge static actor options with static or reactive params.
 *
 * Useful for auth tokens and Svelte-derived params while keeping actor config
 * assembly declarative and easy to reuse.
 */
export function withActorParams<
  Registry extends AnyActorRegistry,
  ActorName extends keyof ExtractActorsFromRegistry<Registry> & string,
>(
  base: MaybeGetter<ActorOptions<Registry, ActorName>>,
  params: MaybeGetter<Record<string, unknown> | undefined>,
): () => ActorOptions<Registry, ActorName> {
  return () => {
    const resolvedBase = extract(base);
    const resolvedParams = extract(params);
    const mergedParams = {
      ...(resolvedBase.params ?? {}),
      ...(resolvedParams ?? {}),
    };

    return {
      ...resolvedBase,
      ...(Object.keys(mergedParams).length > 0 ? { params: mergedParams } : {}),
    };
  };
}

export interface ReactiveConnectionSource {
  connect(): ActorConn<AnyActorDefinition>;
}

export interface ReactiveConnection {
  readonly connection: ActorConn<AnyActorDefinition> | null;
  readonly connStatus: ActorConnStatus;
  readonly error: Error | null;
  readonly isConnected: boolean;
  connect(): ActorConn<AnyActorDefinition>;
  disconnect(): Promise<void>;
  dispose(): Promise<void>;
  onEvent(eventName: string, handler: (...args: unknown[]) => void): () => void;
  /**
   * Returns a promise that resolves to `true` when the connection is
   * established, or `false` if the timeout elapses first.
   *
   * Resolves immediately if already connected.
   *
   * @param timeout - Maximum time to wait in milliseconds (default: 30000).
   */
  whenConnected(timeout?: number): Promise<boolean>;
}

/**
 * Create a reactive wrapper around an existing raw Rivet connection source.
 *
 * This is intended for low-level `handle.connect()` consumers that still want a
 * Svelte-friendly `connStatus` / `error` bridge without adopting `useActor`.
 */
export function createReactiveConnection(
  source: ReactiveConnectionSource,
): ReactiveConnection {
  let _connection = $state.raw<ActorConn<AnyActorDefinition> | null>(null);
  let _connStatus = $state<ActorConnStatus>("idle");
  let _error = $state.raw<Error | null>(null);

  const listeners = new Set<{
    eventName: string;
    handler: (...args: unknown[]) => void;
    unsubscribe?: () => void | Promise<unknown>;
  }>();

  const _onConnectedCallbacks = new Set<(connected: boolean) => void>();

  /** Cancel all pending whenConnected promises, resolving each with the given value. */
  function cancelPendingConnections(connected: boolean): void {
    if (_onConnectedCallbacks.size === 0) return;
    const snapshot = [..._onConnectedCallbacks];
    _onConnectedCallbacks.clear();
    for (const cb of snapshot) cb(connected);
  }

  let cleanupStatus: (() => void) | null = null;
  let cleanupError: (() => void) | null = null;

  function bindConnection(conn: ActorConn<AnyActorDefinition>): void {
    cleanupStatus?.();
    cleanupError?.();

    _connection = conn;
    _connStatus = conn.connStatus;
    _error = null;

    cleanupStatus = conn.onStatusChange((status) => {
      _connStatus = status;
      if (status === "connected") {
        _error = null;
        cancelPendingConnections(true);
      }
    });

    cleanupError = conn.onError((error) => {
      _error = error instanceof Error ? error : new Error(String(error));
    });

    for (const listener of listeners) {
      listener.unsubscribe?.();
      listener.unsubscribe = conn.on(
        listener.eventName,
        listener.handler,
      ) as unknown as () => void | Promise<unknown>;
    }
  }

  function connect(): ActorConn<AnyActorDefinition> {
    if (_connection) return _connection;
    const conn = source.connect();
    bindConnection(conn);
    return conn;
  }

  async function disconnect(): Promise<void> {
    const conn = _connection;
    if (!conn) return;

    cleanupStatus?.();
    cleanupStatus = null;
    cleanupError?.();
    cleanupError = null;

    // Cancel any pending whenConnected promises before tearing down
    cancelPendingConnections(false);

    for (const listener of listeners) {
      listener.unsubscribe?.();
      listener.unsubscribe = undefined;
    }

    await conn.dispose();
    _connection = null;
    _connStatus = "disconnected";
  }

  return {
    get connection() {
      return _connection;
    },
    get connStatus() {
      return _connStatus;
    },
    get error() {
      return _error;
    },
    get isConnected() {
      return _connStatus === "connected";
    },
    connect,
    disconnect,
    dispose() {
      return disconnect();
    },
    whenConnected(timeout = 30_000): Promise<boolean> {
      if (_connStatus === "connected") return Promise.resolve(true);

      return new Promise<boolean>((resolve) => {
        let settled = false;

        const cb = (connected: boolean) => {
          if (settled) return;
          settled = true;
          clearTimeout(timeoutId);
          _onConnectedCallbacks.delete(cb);
          resolve(connected);
        };

        _onConnectedCallbacks.add(cb);

        const timeoutId = setTimeout(() => {
          if (settled) return;
          settled = true;
          _onConnectedCallbacks.delete(cb);
          resolve(false);
        }, timeout);
      });
    },
    onEvent(
      eventName: string,
      handler: (...args: unknown[]) => void,
    ): () => void {
      const listener: {
        eventName: string;
        handler: (...args: unknown[]) => void;
        unsubscribe?: () => void;
      } = { eventName, handler };
      listeners.add(listener);

      if (_connection) {
        // Cast: the actor action surface has grown large enough that
        // rivetkit's generic `on` overload resolution hits TypeScript's
        // recursion limit. The runtime shape is correct (string name,
        // handler callback) — we just erase the deep union at the type
        // boundary.
        listener.unsubscribe = (
          _connection as unknown as {
            on: (e: string, h: (...a: unknown[]) => void) => () => void;
          }
        ).on(eventName, handler);
      }

      return () => {
        listener.unsubscribe?.();
        listeners.delete(listener);
      };
    },
  };
}
