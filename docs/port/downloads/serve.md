# Desktop downloads: lane SERVE

The server now serves the desktop downloads contract under `/download/`, and the image carries the release files at `/app/downloads`. Everything in the contract works locally: on the production-mode binary, in the built image, and in the tests.

There is one open risk: getting the files to Railway with `railway up` is unreliable at release sizes (section 5). A Railway Bucket is proposed instead. It is not built.

Nothing is committed. Date: 2026-11-11.

## 1. What changed

| File | Change |
|---|---|
| `crates/euchre-http/src/downloads.rs` (new, 680 lines) | Loading and checking the manifest, the cache, and the three routes. |
| `crates/euchre-http/src/downloads/tests.rs` (new, 905 lines) | 23 tests. |
| `crates/euchre-http/src/api/mod.rs` | Route `/download/{*rest}` (`get`, so HEAD works and other methods get 405), placed inside the router so the Host check and the security headers apply. |
| `crates/euchre-http/src/config.rs` | `ApiConfig.downloads_dir: Option<PathBuf>`. It is `None` by default: no downloads, and the empty manifest. |
| `crates/euchre-http/src/lib.rs` | `pub mod downloads` plus doc comments. |
| `crates/euchre-http/src/api/tests.rs` | 2 router-level tests. |
| `crates/euchre-http/tests/http_e2e.rs` | `downloads_dir: None` in the config literal. |
| `apps/server/src/server.rs` | `StartOptions.downloads_dir`. `deployed()` sets it from `EUCHRE_DOWNLOADS_DIR` (default `downloads`). The desktop app's `euchre_server::start` uses the default options, so it serves no downloads. |
| `Dockerfile` | New `downloads` stage. The runtime stage gets `COPY --from=downloads /out/downloads /app/downloads` (placed before the binary for layer reuse) and `ENV EUCHRE_DOWNLOADS_DIR=/app/downloads`. |
| `.gitignore` | Adds `/downloads/`. |
| `.railwayignore` (new) | Puts `downloads/` back into the upload, and repeats the `.dockerignore` exclusions for uploads made with no `.git`. |
| `docs/DEPLOY.md` | New section 9, "Desktop downloads". Section 8 gains the step "put the release in `downloads/` before every upload". The section 1 table mentions `/app/downloads`. |

`.dockerignore` and every `Cargo.toml` are unchanged. No new dependencies: the code uses tower-http `ServeFile`, plus `serde_json`, `regex`, `sha2` and `tokio`, which the crate already had.

## 2. How it works

- **Manifest:** read at request time, at most every 5 s (`RELOAD_EVERY`). A tokio mutex makes concurrent requests share one read. It is capped at 64 KiB.
  - Missing folder or missing manifest: the empty manifest, with no warning.
  - Bad JSON, a bad `version` or a non-object `platforms`: the empty manifest and one warning.
  - A bad entry drops only its own platform, with a warning naming the reason: not a plain name, a bad `bytes` or `sha256`, file missing, not a regular file, a symlink leaving the folder, or a size mismatch.
  - Each warning is logged once until it goes away. Each change in what is served logs one `desktop downloads loaded` line.
- **Names:** a single plain file name. Only ASCII letters, digits and `. _ - +` are allowed, 1 to 128 bytes, with no leading `.`, no `..`, and never `manifest.json`. This allow-list rules out `/`, `\`, NUL, control characters and quotes.
- **Paths:** each file is canonicalized and must stay under the canonical folder and be a regular file. A symlink inside the folder is fine; one pointing outside is refused. The manifest itself follows the same rule.
- **URLs:** the part after `/download/` is percent-decoded once. Malformed escapes get `400`. `files/<name>` is looked up by exact name among the manifest entries that passed the checks, so no URL can reach a file that is not listed.
- **Files:**
  - tower-http `ServeFile` streams from disk. It handles one range (`206`), `416` with `Content-Range: bytes */N`, HEAD, `Last-Modified`, `If-Modified-Since` and `If-Unmodified-Since`.
  - I added ETag (the strong tag `"<sha256>"`), `If-None-Match` (weak comparison and `*`; when present it overrides `If-Modified-Since`) and `If-Range` (strong ETag comparison only).
  - Content-Type is set from the extension. Any `Content-Encoding` is removed, and precompressed siblings are never served.

## 3. Tests

Target dir `/Users/brittianwarner/goods/euchre-rune/.cargo-targets/serve`.

- `cargo fmt --all --check`: clean.
- `cargo clippy -p euchre-http -p euchre-server --all-targets -- -D warnings`: clean.
- `cargo test -p euchre-http -p euchre-server`: everything passed. euchre-http lib 160, `http_e2e` 4. euchre-server lib 17, main 1, e2e 2 (2 ignored, as before).

New tests:

- 23 in `downloads::tests`:
  - manifest shape;
  - HEAD;
  - missing folder or manifest;
  - a file in place of the folder;
  - 7 bad-JSON cases;
  - an oversized manifest;
  - 9 bad-entry cases;
  - unknown keys;
  - sha256 lowercased;
  - size mismatch;
  - a redirect per platform, for GET and HEAD;
  - 17 not-found URLs;
  - full GET;
  - HEAD on a file;
  - Range: three 206 cases and five 416 cases;
  - ETag conditionals;
  - name rules;
  - versions and content types;
  - unsafe names in the manifest;
  - traversal and injection in the URL (`%2e%2e%2f`, `..%5C`, `%00`, `%0d%0a`, `%zz`);
  - symlink escape: relative, absolute, and the manifest itself;
  - one warning per problem;
  - cache window.
- 2 in `api::tests`:
  - through the front door: Host `421`, nosniff, `405` for POST, PUT and DELETE, and `404` rather than the SPA;
  - no folder gives the empty manifest.

A note on an unrelated test: `bootstrap::tests::deadline_exhaustion_fails_with_attempt_count` failed once earlier in this lane, at load average about 71. Its 5 ms deadline ran out on the first attempt. It passes alone and passed in this final run. It is not related to these changes.

## 4. Local proof

### 4.1 Production-mode binary

The release `euchre-server` ran on port 7020 with `NODE_ENV=production`, against a local rivet-engine 3.1.0 on 7030. It served the real web build plus a fake `downloads/`:

| File | Bytes |
|---|---|
| `Euchre-0.1.0-mac.dmg` | 20,971,520 |
| `Euchre-0.1.0-windows.zip` | 15,728,640 |
| `Euchre-0.1.0-linux.AppImage` | 26,214,400 |

All three are random data, listed in a manifest with real sha256 values. Abridged output (full transcripts were in the session scratchpad):

```
GET /download/manifest.json
HTTP/1.1 200 OK
content-type: application/json
cache-control: no-cache
x-content-type-options: nosniff
referrer-policy: strict-origin-when-cross-origin
content-length: 443
{"version":"0.1.0","platforms":{"mac":{"file":"Euchre-0.1.0-mac.dmg","bytes":20971520,"sha256":"3dc7…9b90"},"windows":{…},"linux":{…}}}
(HEAD: same headers, no body)

GET /download/mac          302  location: /download/files/Euchre-0.1.0-mac.dmg  cache-control: no-cache
GET /download/windows      302  location: /download/files/Euchre-0.1.0-windows.zip
GET /download/linux        302  location: /download/files/Euchre-0.1.0-linux.AppImage
HEAD /download/mac         302  (same headers)

GET /download/files/Euchre-0.1.0-mac.dmg
HTTP/1.1 200 OK
content-type: application/x-apple-diskimage
accept-ranges: bytes
last-modified: Wed, 11 Nov 2026 14:52:05 GMT
content-length: 20971520
content-disposition: attachment; filename="Euchre-0.1.0-mac.dmg"
cache-control: public, max-age=31536000, immutable
etag: "3dc71312488dcae67051ad65cabd0c11d47773838301dff6c65dc90aa01d9b90"
x-content-type-options: nosniff
referrer-policy: strict-origin-when-cross-origin
(downloaded body sha256 = manifest sha256, 20971520 bytes)

HEAD …windows.zip          200  content-type: application/zip, content-length: 15728640
HEAD …linux.AppImage       200  content-type: application/octet-stream, content-length: 26214400
GET with Accept-Encoding: br, gzip   200, no content-encoding, content-length: 26214400

Range: bytes=0-1023        206  content-range: bytes 0-1023/20971520, content-length: 1024 (body identical to the first 1024 bytes)
Range: bytes=-512          206  content-range: bytes 20971008-20971519/20971520
Range: bytes=99999999-     416  content-range: bytes */20971520, cache-control: no-cache
Range: bytes=0-1,5-6       416  "Cannot serve multipart range requests"
If-None-Match: "<sha>"     304  etag + cache-control kept
If-None-Match: W/"<sha>"   304
If-None-Match: "other"     200
Range 0-9 + If-Range: "<sha>"    206
Range 0-9 + If-Range: "stale"    200 (full file)
-L /download/windows       302 then 200 application/zip

404 "Not found" (text/plain): /download/solaris, /download/files/, /download/files/manifest.json,
  /download/files/Euchre-0.1.0-mac.dmg.gz, /download/files/other.dmg, /download/mac/extra,
  /download/files/..%2fmanifest.json, /download/files/%2e%2e%2f%2e%2e%2fCargo.toml,
  /download/files/..%5CCargo.toml, /download/files/Euchre-0.1.0-mac.dmg%00,
  /download/files/a%0d%0aSet-Cookie:x=1, --path-as-is /download/files/../../Cargo.toml
400 "Bad path": /download/files/%zz
200: /download/files/Euchre-0.1.0-mac%2Edmg (decodes to the listed name)
421 "Unknown host": Host: attacker.example
405 allow: GET,HEAD: POST /download/manifest.json, DELETE /download/mac
/download and /download/ (no further path): 200, the SPA page (outside the /download/{*rest} route)
```

**Broken manifest at runtime.** I swapped `manifest.json` while the server ran, waited past the 5 s window, and made several requests over about 12 s.

- **Truncated JSON:**
  - The manifest became `{"version":null,"platforms":{}}`.
  - `/download/mac` and its file both returned `404`.
  - The log had exactly one warning: `desktop downloads: manifest.json is not valid JSON: EOF while parsing an object at line 1 column 32`.
- **Mac `bytes` one higher than on disk:**
  - The manifest listed `linux` and `windows` only, and every mac route returned `404`.
  - The log had exactly one warning: `manifest.json: mac left out: Euchre-0.1.0-mac.dmg is 20971520 bytes on disk but the manifest says 20971521`.
- **Restored:** all three platforms came back, with one `desktop downloads loaded … platforms=["mac", "windows", "linux"]` line.

### 4.2 Docker image

Built from a copy of the build context: once with no `downloads/`, once with an empty `downloads/`, and once with the fake release plus a `.DS_Store`. The images were tagged `euchre-dl-test:{without,empty,with}`.

| Image | Content size (`docker image inspect .Size`) | `docker image ls` (disk) | `/app/downloads` |
|---|---|---|---|
| without `downloads/` | 52,803,251 B | 218 MB | empty (build log: `no desktop downloads in this image`) |
| empty `downloads/` | 52,803,234 B | 218 MB | empty |
| with the fake release | 115,737,816 B | 344 MB | 3 files + `manifest.json`, `0644 root`; `.DS_Store` left out |

- **Size difference:** 62,934,565 bytes, about 60.0 MiB. That is the 62,915,026 bytes of files plus about 20 KB of layer overhead. `docker history` shows the layer as `62.9MB COPY /out/downloads /app/downloads`. The "disk" column grows by about twice that, because Docker Desktop counts both the stored layer and its unpacked copy.
- **Build log:** the build prints each file and its sha256, and they match the manifest.
- **The "with" image** ran against the local engine (via `host.docker.internal`) on `127.0.0.1:7021`:
  - The log showed `desktop downloads loaded dir=/app/downloads version="0.1.0" platforms=["mac", "windows", "linux"]`.
  - The manifest returned `200`.
  - `/download/linux` returned `302`, then `200` with a sha256 that matched.
  - A range request returned `206`.
  - `/download/files/manifest.json` returned `404`.
- **The "without" image** returned the manifest `{"version":null,"platforms":{}}`, and `/download/mac` returned `404`.
- **Layer reuse:** I changed only `README.md` and rebuilt the "with" context. `[runtime 4/6] COPY --from=downloads` came back `CACHED`, with the same layer digest. The server layer was unchanged too; only the web layer changed, because of its build stamp.
- **Images removed:** all four images (`without`, `empty`, `with`, `with2`) were removed after the run. The shared build cache was left alone.

## 5. Railway: what `railway up` uploads, and its size limit

Sources, read only:

- [docs.railway.com/cli/up](https://docs.railway.com/cli/up)
- [CLI `src/controllers/upload.rs`](https://raw.githubusercontent.com/railwayapp/cli/master/src/controllers/upload.rs) and [`src/commands/up.rs`](https://raw.githubusercontent.com/railwayapp/cli/master/src/commands/up.rs)
- [Forum: "File too large on GitLab CI/CD"](https://station.railway.com/questions/file-too-large-on-gitlab-ci-cd-23f78b59)
- [Forum: "railway up fails with 500 on 90MB upload"](https://station.railway.com/questions/railway-up-fails-with-500-on-90mb-uploa-532e61e8)
- [docs.railway.com/storage-buckets](https://docs.railway.com/storage-buckets)

**How files are picked:**

- The docs say `railway up` respects `.gitignore` and `.railwayignore`, always ignores `.git` and `node_modules`, and has `--no-gitignore`.
- The CLI walks the folder with the `ignore` crate (`add_custom_ignore_filename(".railwayignore")`, `hidden(false)`, `follow_links(true)`) and uploads one gzip tar.
- In `ignore` 0.4.25 a custom ignore file wins over `.gitignore`. `.gitignore` only counts inside a git checkout; a worktree's `.git` file counts.
- I rebuilt that walker as a small scratch tool and checked it on fixtures:
  - In a git checkout with `.railwayignore`, `downloads/` is uploaded, minus `.DS_Store`.
  - Without `.railwayignore`, it is not uploaded.
  - In a copy with no `.git`, `.railwayignore` alone gives the same file set.

**Size limit:**

- The docs state none. The CLI has no limit of its own; a `413` from the server prints `Failed to upload code. File too large (<n> bytes)`.
- Forum, 2024-03-07: a Railway employee wrote "the limit is 40 as far as I know".
- Forum, 2026-07-16: a roughly 90 MB tar.gz that had uploaded on 2026-06-21 failed with `500` five times. Staff said there is "no documented upload size limit", mentioned a 100 GB build image limit, and suggested a GitHub source or a pre-built image.

**Measured upload** (gzip tar, this repo's `.railwayignore`):

- Without `downloads/`: 569 files, 15.9 MB raw, 7.38 MB gzipped.
- With the 60 MiB fake release: 573 files, 78.8 MB raw, 70.3 MB gzipped.
- Release files are already compressed, so they hardly shrink. A raw executable halves under gzip, but a `.dmg`, `.zip` or `.AppImage` does not.
- A real release of three files of 15 to 45 MB means an upload of about 52 to 142 MB.

**Verdict:** shipping the apps inside the `railway up` upload works mechanically, but it is not reliable at these sizes. The only figure from a Railway employee is 40 MB, and a 90 MB upload failed this July.

A second problem: every `railway up` must carry the current release, or the new image serves no downloads. A GitHub source does not help, because `downloads/` is gitignored.

**Proposed alternative (not built):** a Railway Bucket. Railway's docs say buckets are private and S3-compatible, support presigned URLs, and have free bucket egress; public buckets are not supported.

- The release step uploads the files and the manifest to the bucket.
- The server reads the manifest from the bucket with the same checks and 5 s cache.
- `/download/files/<file>` returns a `302` to a short-lived presigned URL.
- The routes and the JSON stay exactly as in the contract, so the web lane and the desktop lane change nothing.
- Code uploads stay around 7 MB, and releasing an app no longer needs a code deploy.

The other option is to build the image locally and push it to a registry as the service source. It works, but it changes every release.

`docs/DEPLOY.md` section 9.4 says all of this for the operator.

## 6. Tools, processes, cleanup

- **Tools installed:** none.
- **Scratch tool:** `upwalk`, a scratch Rust crate in the session scratchpad that copies the Railway CLI's walker (`ignore` 0.4.25, `flate2`, `tar`). It was built offline into `/Users/brittianwarner/goods/euchre-rune/.cargo-targets/serve/release/upwalk`.
- **Engine:** the rivet-engine 3.1.0 binary already in `node_modules/.bun/@rivetkit+engine-cli-darwin-arm64@3.1.0`.
- **Processes I started, all stopped:** the engine (pids 96187 and 7341), the server (pid 96789), and the `euchre-dl-test-run` container.
- **Ports:** nothing listens on 7020 to 7039.
- **Not touched:** pids 748 and 67721, and two older processes from another lane (71587 and 71631, in `webspike/net/serve`).
- **Git:** no git writes. No railway commands were run.

## 7. Open items

- **Decide how the files reach production:** try `railway up` with a real release and accept the risk, or build the bucket version (section 5).
- **Real release check:** with real release files, check `downloads/` on euchre.sh with the commands in `docs/DEPLOY.md` section 9.5.
- **`/download` with no further path:** it shows the SPA page. The web lane could add a page there, or leave it.
