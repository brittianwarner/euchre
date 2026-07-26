// Guard for the @rivetkit/framework-base getParams patch.
//
// The reactive-actor path (apps/web BaseActorViewModel) mints a FRESH auth token
// on every connect AND reconnect by passing `getParams` to createReactiveActor.
// Stock @rivetkit/framework-base@2.3.5 does NOT forward `getParams` from
// ActorOptions down to the rivetkit client's get()/getOrCreate() calls — the
// forwarding is added by the root patch `patches/@rivetkit%2Fframework-base@2.3.5.patch`
// (registered in root package.json `patchedDependencies`, keyed to EXACT "2.3.5").
//
// RISK this guards: if the dep ever bumps off exact 2.3.5, bun silently skips the
// version-keyed patch, getParams forwarding vanishes, and every actor reconnect
// replays a FROZEN token. This test reads the INSTALLED dist that the runtime
// actually loads and fails loudly the moment the patch stops applying.
//
// Why read the dist instead of importing the module: `create()` (the function
// that forwards getParams) is an internal, non-exported function — there is no
// runtime seam to assert against. Every other test in this dir vi.mock()s the
// whole package, so none exercise the real patched dist. A static read is the
// only faithful guard.
//
// NOTE: This targets @rivetkit/framework-base (stock npm + root patch).
// It is NOT the forked `rivetkit` runtime at packages/vendored/rivetkit
// (2.3.5-layerr.0, via root `overrides`) — a different package, out of scope.

import { describe, expect, test } from "vitest";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";

const PATCH_FILE = "patches/@rivetkit%2Fframework-base@2.3.5.patch";
const PATCH_VERSION = "2.3.5";

// Resolve the installed dist. `import.meta.resolve` throws under vitest's module
// runner ("'import.meta.resolve' is not supported"), so use createRequire, which
// resolves the `require`/CJS condition (dist/mod.js). The patch modifies mod.js
// and mod.mjs identically. Do NOT hardcode the node_modules/.bun/... path — it
// is bun-version/hash-specific and will rot.
const require = createRequire(import.meta.url);
const distPath = require.resolve("@rivetkit/framework-base");

describe("@rivetkit/framework-base getParams patch integrity", () => {
  test(`installed dep is EXACTLY ${PATCH_VERSION} so the version-keyed patch still applies`, () => {
    const pkgPath = resolve(dirname(distPath), "..", "package.json");
    const pkg = JSON.parse(readFileSync(pkgPath, "utf8")) as { version: string };

    expect(
      pkg.version,
      `@rivetkit/framework-base is installed at ${pkg.version}, not ${PATCH_VERSION}. ` +
        `bun keys the patch "${PATCH_FILE}" to @rivetkit/framework-base@${PATCH_VERSION} ` +
        `and SILENTLY skips it on any other version — getParams forwarding will be gone ` +
        `and every actor reconnect will replay a frozen auth token. Re-key/regenerate the ` +
        `patch for the new version (root package.json patchedDependencies) before bumping.`,
    ).toBe(PATCH_VERSION);
  });

  test("dist forwards getParams to BOTH client.get and client.getOrCreate", () => {
    const src = readFileSync(distPath, "utf8");

    // Anchor on the full forward token: stock framework-base has ZERO
    // `actor.opts.getParams` references, so an incidental `getParams` elsewhere
    // cannot make this vacuously pass. Whitespace-tolerant for a future reflow.
    const forwards = src.match(/getParams\s*:\s*actor\.opts\.getParams/g) ?? [];

    expect(
      forwards.length,
      `Expected >=2 \`getParams: actor.opts.getParams\` forwards in ${distPath} ` +
        `(one for client.get's noCreate branch, one for client.getOrCreate's create ` +
        `branch) but found ${forwards.length}. The patch "${PATCH_FILE}" likely did not ` +
        `apply — check root package.json patchedDependencies after any dep bump. Without ` +
        `this forwarding, BaseActorViewModel's getParams never reaches the rivetkit ` +
        `client and reconnects replay a frozen token.`,
    ).toBeGreaterThanOrEqual(2);
  });

  test("type surface exposes getParams? on ActorOptions (.d.ts stays in sync)", () => {
    // Belt-and-suspenders: the patch also adds the field to the type decls. If
    // the value forward survived but the type was dropped, getParams calls would
    // forward at runtime but fail typecheck for consumers — catch that too.
    let dts: string | null = null;
    for (const name of ["mod.d.ts", "mod.d.mts"]) {
      try {
        dts = readFileSync(resolve(dirname(distPath), name), "utf8");
        break;
      } catch {
        // try the next decl filename
      }
    }

    expect(
      dts !== null && /getParams\s*\?\s*:/.test(dts),
      `Expected an optional \`getParams?:\` declaration in the framework-base type ` +
        `surface (mod.d.ts/mod.d.mts) but found none — the patch's type hunk did not ` +
        `apply. Regenerate "${PATCH_FILE}".`,
    ).toBe(true);
  });
});
