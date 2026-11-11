#!/bin/sh
# `bun run dev` (root `bun run dev:server`): `cargo run -p euchre-server`, pointed
# at the rivet-engine 3.1.0 binary that `bun install` put in node_modules.
#
# In local mode (RIVET_ENDPOINT unset) the server spawns its own engine and
# never downloads one. Without this it finds the binary only through
# RIVET_ENGINE_BINARY_PATH or rivetkit's cache in ~/.rivetkit, which a fresh
# machine does not have. An explicit RIVET_ENGINE_BINARY_PATH always wins, and
# an external engine (RIVET_ENDPOINT set) needs no binary.
#
# It runs from any directory and keeps the caller's working directory, which is
# where the server looks for `build/client` (run it from apps/web to serve a web
# build). `exec` keeps one process, so ctrl-c and SIGTERM reach the server once
# and it drains and stops its engine as usual.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)

if [ -z "${RIVET_ENGINE_BINARY_PATH:-}" ] && [ -z "${RIVET_ENDPOINT:-}" ]; then
	if engine=$(cd "$here" && bun -e 'console.log(require("@rivetkit/engine-cli").getEnginePath())' 2>/dev/null) && [ -n "$engine" ]; then
		RIVET_ENGINE_BINARY_PATH=$engine
		export RIVET_ENGINE_BINARY_PATH
	else
		echo "dev: no bun-installed rivet-engine; the server falls back to ~/.rivetkit. Run bun install, or set RIVET_ENGINE_BINARY_PATH." >&2
	fi
fi

exec cargo run -p euchre-server "$@"
