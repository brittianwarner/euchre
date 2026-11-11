# syntax=docker/dockerfile:1
#
# Euchre server image: the Rust server (apps/server, binary `euchre-server`)
# plus the static web build it serves from `build/client` (ARCHITECTURE.md D4).
# Desktop downloads are not in the image: production serves them from a Railway
# bucket (EUCHRE_DOWNLOADS_S3_*, docs/DEPLOY.md section 9).
#
#   docker build -t euchre .
#   docker run --rm -p 3000:3000 --env-file <secrets> euchre
#
# No secrets are baked in. Railway passes the service variables at run time
# (.railway/railway.ts, docs/DEPLOY.md). No BuildKit cache mounts: Railway
# needs `s/<service-id>-` mount ids, so caching comes from the layers below.

# Base images are pinned by multi-arch index digest, with the tag kept for
# reading. When a digest is present the tag is ignored, so bump each version
# ARG together with its digest (the index digest from Docker Hub, or
# `docker buildx imagetools inspect <image:tag>`), or the old image stays.
#
# Must equal `channel` in rust-toolchain.toml; the planner stage fails if not.
ARG RUST_VERSION=1.99.1
ARG CHEF_VERSION=0.1.78
ARG DEBIAN=trixie
# lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-slim-trixie
ARG CHEF_DIGEST=sha256:c7b109b68df2196f7a8e5f370c4dfd5e4b5b98151f56c2a728887610dc8d1632
# Must equal `packageManager` in package.json.
ARG BUN_VERSION=1.4.4
# oven/bun:1.4.4-slim
ARG BUN_DIGEST=sha256:1846adf22496bd711036aae8be147e57bdebbaea2e5264e3e0b5af1db187d9a4
# debian:trixie-slim
ARG DEBIAN_DIGEST=sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

# ---------------------------------------------------------------------------
# Rust toolchain with cargo-chef (dependency-layer caching for the workspace).
FROM lukemathwalker/cargo-chef:${CHEF_VERSION}-rust-${RUST_VERSION}-slim-${DEBIAN}@${CHEF_DIGEST} AS chef
ARG RUST_VERSION
# Use the image's toolchain as is. rust-toolchain.toml would otherwise make
# rustup download clippy and rustfmt, which a release build does not need.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
WORKDIR /src

# ---------------------------------------------------------------------------
# Recipe: every workspace manifest, so the dependency layer only rebuilds when
# Cargo.toml / Cargo.lock change.
FROM chef AS planner
ARG RUST_VERSION
COPY . .
RUN set -eu; \
    channel="$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)"; \
    if [ "$channel" != "$RUST_VERSION" ]; then \
      echo "rust-toolchain.toml pins $channel but this Dockerfile builds with $RUST_VERSION; update ARG RUST_VERSION" >&2; \
      exit 1; \
    fi; \
    cargo chef prepare --recipe-path recipe.json

# ---------------------------------------------------------------------------
# Server binary.
FROM chef AS server
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p euchre-server --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p euchre-server \
 && install -D -m 0755 target/release/euchre-server /out/euchre-server

# ---------------------------------------------------------------------------
# Static web build (SvelteKit adapter-static SPA, fallback index.html) and the
# rivetkit bridge bundle the Rune wasm imports (apps/desktop/js/rivet-bridge.js).
FROM oven/bun:${BUN_VERSION}-slim@${BUN_DIGEST} AS web
WORKDIR /src
COPY . .
RUN bun install --frozen-lockfile
RUN bun run --cwd apps/desktop bridge
RUN set -eu; \
    cd apps/web; \
    bun run build; \
    if [ -f build/client/index.html ]; then out=build/client; \
    elif [ -f build/index.html ]; then out=build; \
    else \
      echo "apps/web build has no index.html. The server needs an adapter-static SPA with fallback index.html (D4)." >&2; \
      ls -la build >&2 || true; \
      exit 1; \
    fi; \
    mkdir -p /out; \
    cp -R "$out" /out/client

# ---------------------------------------------------------------------------
# Rune web app: the euchre-desktop crate on wasm32, wasm-bindgen, art, loader
# (apps/desktop/scripts/web-build.sh). The wasm-bindgen CLI must equal the
# `wasm-bindgen` version in Cargo.lock (the script checks).
FROM chef AS rune-web
ARG RUNE_FEATURES=""
ARG WASM_BINDGEN_VERSION=0.2.130
# Rune's release profile for the web; web-build.sh sets the same values.
ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=s \
    CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
    CARGO_PROFILE_RELEASE_DEBUG=0 \
    CARGO_PROFILE_RELEASE_STRIP=debuginfo \
    RUSTFLAGS="--remap-path-prefix=/root=/home --remap-path-prefix=/usr/local/cargo=/cargo"
RUN rustup target add wasm32-unknown-unknown \
 && cargo install wasm-bindgen-cli --version ${WASM_BINDGEN_VERSION} --locked
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --target wasm32-unknown-unknown -p euchre-desktop \
    ${RUNE_FEATURES:+--features "$RUNE_FEATURES"} --recipe-path recipe.json
COPY . .
COPY --from=web /src/apps/desktop/js apps/desktop/js
RUN RUNE_FEATURES="${RUNE_FEATURES}" bash apps/desktop/scripts/web-build.sh --out /out/rune

# ---------------------------------------------------------------------------
# The merged client: Svelte build plus Rune build, precompressed
# (apps/desktop/scripts/web-site.sh).
FROM oven/bun:${BUN_VERSION}-slim@${BUN_DIGEST} AS site
WORKDIR /src
COPY apps/desktop/scripts apps/desktop/scripts
COPY --from=web /out/client /in/svelte
COPY --from=rune-web /out/rune /in/rune
RUN bash apps/desktop/scripts/web-site.sh /in/svelte /in/rune /out/client

# ---------------------------------------------------------------------------
# Runtime. No curl and no Docker HEALTHCHECK: Railway probes `GET /health` itself
# (`healthcheck` in .railway/railway.ts) and ignores HEALTHCHECK, and curl would
# only add attack surface. /health answers on any Host: 200 `ok`, or 503
# `draining` before the runner connects and during shutdown.
FROM debian:${DEBIAN}-slim@${DEBIAN_DIGEST} AS runtime
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates; \
    rm -rf /var/lib/apt/lists/*; \
    groupadd --system --gid 10001 euchre; \
    useradd --system --uid 10001 --gid euchre --create-home --home-dir /home/euchre \
      --shell /usr/sbin/nologin euchre
WORKDIR /app
COPY --from=server /out/euchre-server /app/euchre-server
# The server serves `build/client` relative to its working directory.
COPY --from=site /out/client /app/build/client
USER euchre:euchre
ENV PORT=3000
EXPOSE 3000
CMD ["/app/euchre-server"]
