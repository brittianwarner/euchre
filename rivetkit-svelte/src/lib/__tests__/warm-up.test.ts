import "./runes-shim.js";
import { describe, expect, test, vi } from "vitest";

vi.mock("@rivetkit/framework-base", () => ({
  createRivetKit: vi.fn(() => ({
    getOrCreateActor: vi.fn(),
  })),
}));

vi.mock("esm-env", () => ({
  BROWSER: true,
  DEV: false,
}));

import { createRivetKitWithClient } from "../rivetkit.svelte.js";

function createClient(
  resolveImpl: () => Promise<string> = () => Promise.resolve("actor-id"),
) {
  const resolve = vi.fn(resolveImpl);
  const handle = { resolve };
  const get = vi.fn(() => handle);
  const getOrCreate = vi.fn(() => handle);
  const client = {
    document: { get, getOrCreate },
  };

  return { client, get, getOrCreate, resolve };
}

describe("warmUp", () => {
  test("resolves actor with getOrCreate without opening a connection", () => {
    const { client, get, getOrCreate, resolve } = createClient();
    const rivet = createRivetKitWithClient(client as never);

    rivet.warmUp({ name: "document" as never, key: "doc-1" });

    expect(getOrCreate).toHaveBeenCalledWith(["doc-1"], {});
    expect(get).not.toHaveBeenCalled();
    expect(resolve).toHaveBeenCalledTimes(1);
  });

  test("passes null createWithInput to Rivet", () => {
    const { client, getOrCreate } = createClient();
    const rivet = createRivetKitWithClient(client as never);

    rivet.warmUp({
      name: "document" as never,
      key: ["doc-1"],
      createWithInput: null,
    });

    expect(getOrCreate).toHaveBeenCalledWith(["doc-1"], {
      createWithInput: null,
    });
  });

  test("passes createInRegion to getOrCreate", () => {
    const { client, getOrCreate } = createClient();
    const rivet = createRivetKitWithClient(client as never);

    rivet.warmUp({
      name: "document" as never,
      key: ["doc-1"],
      createInRegion: "atl",
    });

    expect(getOrCreate).toHaveBeenCalledWith(["doc-1"], {
      createInRegion: "atl",
    });
  });

  test("uses get when noCreate is requested", () => {
    const { client, get, getOrCreate, resolve } = createClient();
    const rivet = createRivetKitWithClient(client as never);

    rivet.warmUp({
      name: "document" as never,
      key: ["doc-1"],
      noCreate: true,
    });

    expect(get).toHaveBeenCalledWith(["doc-1"]);
    expect(getOrCreate).not.toHaveBeenCalled();
    expect(resolve).toHaveBeenCalledTimes(1);
  });

  test("deduplicates successful warm-ups by actor identity", () => {
    const { client, resolve } = createClient();
    const rivet = createRivetKitWithClient(client as never);

    rivet.warmUp({ name: "document" as never, key: ["doc-1"] });
    rivet.warmUp({ name: "document" as never, key: ["doc-1"] });

    expect(resolve).toHaveBeenCalledTimes(1);
  });

  test("allows retry after resolve failure", async () => {
    let rejectResolve: ((error: Error) => void) | undefined;
    const { client, resolve } = createClient(
      () =>
        new Promise<string>((_resolve, reject) => {
          rejectResolve = reject;
        }),
    );
    const rivet = createRivetKitWithClient(client as never);

    rivet.warmUp({ name: "document" as never, key: ["doc-1"] });
    expect(resolve).toHaveBeenCalledTimes(1);

    rejectResolve!(new Error("resolve failed"));
    await vi.waitFor(() => {
      rivet.warmUp({ name: "document" as never, key: ["doc-1"] });
      expect(resolve).toHaveBeenCalledTimes(2);
    });
  });

  test("deprecated preloadActor alias still resolves the actor", () => {
    const { client, getOrCreate, resolve } = createClient();
    const rivet = createRivetKitWithClient(client as never);

    rivet.preloadActor({ name: "document" as never, key: "doc-1" });

    expect(getOrCreate).toHaveBeenCalledWith(["doc-1"], {});
    expect(resolve).toHaveBeenCalledTimes(1);
  });
});
