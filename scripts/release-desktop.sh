#!/usr/bin/env bash
# Build the Euchre desktop downloads and write downloads/manifest.json (docs/DEPLOY.md section 9).
#
#   scripts/release-desktop.sh                # mac, windows and linux
#   scripts/release-desktop.sh mac linux      # only these; files already built for the others stay
#   scripts/release-desktop.sh --upload       # also upload to the Railway bucket euchre-downloads
#                                             # (needs bun and a linked railway CLI; see scripts/upload-downloads.ts)
#
# Needs an Apple-silicon Mac with the Xcode command line tools, rustup and a running Docker
# (Docker Desktop or OrbStack). Linux and Windows are cross-built in an arm64 Debian 11
# container, so the Mac itself only gains two rustup targets. Works with macOS's bash 3.2.
#
# Writes, into EUCHRE_DOWNLOADS_DIR (default: <repo>/downloads, gitignored):
#   Euchre-<v>-mac.dmg          universal (arm64 + x86_64), ad-hoc signed, macOS 13 or newer
#   Euchre-<v>-windows.zip      Euchre.exe, x86_64, Windows 10 or newer
#   Euchre-<v>-linux.AppImage   x86_64, glibc 2.31 or newer
#   manifest.json               {"version", "platforms": {mac|windows|linux: {file, bytes, sha256}}}
# <v> is euchre-desktop's version from the workspace Cargo.toml.
#
# Environment:
#   EUCHRE_DEFAULT_SERVER_URL  server the app opens unless EUCHRE_SERVER_URL or
#                              EUCHRE_DESKTOP_BACKEND says otherwise (default https://euchre.sh;
#                              set it empty for an app that starts its own offline server)
#   CARGO_TARGET_DIR           cargo target dir for the Mac builds (default: the workspace's)
#   RELEASE_WORK_DIR           temp files and the pinned AppImage tools
#                              (default: <target dir>/release-desktop)
#   RELEASE_DOCKER_PREFIX      names the builder image and its two cache volumes
#                              (default euchre-desktop; `docker volume rm <prefix>-cargo
#                              <prefix>-target` frees the caches)
#   RELEASE_REPLACE=1          rebuild files this version already has (only before publishing)
#
# Safe to re-run. A platform whose file for this version exists is kept, because
# /download/files/* is cached for a year and a name must never get new bytes. Builds are
# incremental. Each download is copied in under a hidden name and renamed when complete,
# manifest.json is rewritten atomically from the files on disk, and downloads of other
# versions are deleted only after the new manifest stops naming them.
set -euo pipefail

RUST_VERSION=1.99.1   # rust-toolchain.toml
MACOS_MIN=13.0        # the oldest macOS wgpu's Metal backend supports
BUNDLE_ID=sh.euchre.desktop
APPIMAGETOOL_URL=https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-aarch64.AppImage
APPIMAGETOOL_SHA256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
RUNTIME_URL=https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

die() { printf 'release-desktop: %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s (%ss)\n' "$*" "$SECONDS" >&2; }
sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }

repo=$(cd "$(dirname "$0")/.." && pwd)
icons=$repo/apps/desktop/assets/icon
export EUCHRE_DEFAULT_SERVER_URL=${EUCHRE_DEFAULT_SERVER_URL-https://euchre.sh}
prefix=${RELEASE_DOCKER_PREFIX:-euchre-desktop}

upload=0; args=()
for a in "$@"; do
  case $a in
    --upload) upload=1 ;;
    *) args+=("$a") ;;
  esac
done
set -- ${args[@]+"${args[@]}"}
[ $# -gt 0 ] || set -- mac windows linux
for p in "$@"; do
  case $p in
    mac | windows | linux) ;;
    *) die "unknown platform '$p' (use mac, windows or linux)" ;;
  esac
done
[ "$(uname -s) $(uname -m)" = "Darwin arm64" ] || die "run this on an Apple-silicon Mac"
if [ "$upload" = 1 ]; then
  for tool in bun railway; do command -v "$tool" >/dev/null || die "$tool is not installed (needed for --upload)"; done
fi
for tool in cargo rustup docker lipo strip codesign hdiutil plutil curl shasum /usr/bin/python3; do
  command -v "$tool" >/dev/null || die "$tool is not installed"
done
docker info >/dev/null 2>&1 || die "Docker is not running"
for f in Euchre.icns Euchre.ico euchre-512.png; do [ -f "$icons/$f" ] || die "missing $icons/$f"; done

# Version and target dir, as cargo sees them (CARGO_TARGET_DIR and .cargo/config included).
{ read -r version; read -r target_dir; } < <(
  cd "$repo" && cargo +"$RUST_VERSION" metadata --locked --no-deps --format-version 1 |
    /usr/bin/python3 -I -c 'import json, sys
m = json.load(sys.stdin)
print(next(p["version"] for p in m["packages"] if p["name"] == "euchre-desktop"))
print(m["target_directory"])')
[[ $version =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] ||
  die "version '$version' is not semver"
core=${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.${BASH_REMATCH[3]}   # Info.plist and VERSIONINFO want digits

work=${RELEASE_WORK_DIR:-$target_dir/release-desktop}
downloads=${EUCHRE_DOWNLOADS_DIR:-downloads}
case $downloads in /*) ;; *) downloads=$repo/$downloads ;; esac
mkdir -p "$work/tools" "$downloads"

# One run at a time per work dir; a lock left by a run that died is taken over.
lock=$work/lock
if ! mkdir "$lock" 2>/dev/null; then
  other=$(cat "$lock/pid" 2>/dev/null || true)
  if [ -n "$other" ] && kill -0 "$other" 2>/dev/null; then die "another run (pid $other) is using $work"; fi
  rm -rf "$lock"; mkdir "$lock"
fi
echo $$ > "$lock/pid"
tmp=$(mktemp -d "$work/run.XXXXXX")
trap 'rm -rf "$tmp" "$lock"' EXIT

printf 'Euchre %s  server=%s\ndownloads: %s\n' "$version" "${EUCHRE_DEFAULT_SERVER_URL:-<offline>}" "$downloads" >&2

# A published file name must never get new bytes, and a rebuild never gives the same bytes
# (zip, dmg and squashfs carry timestamps). So a platform whose file for this version is
# already in the folder is kept as it is, unless RELEASE_REPLACE=1 (only before publishing).
want_mac=0; docker_platforms=""
for p in "$@"; do
  case $p in
    mac) name=Euchre-$version-mac.dmg ;;
    windows) name=Euchre-$version-windows.zip ;;
    linux) name=Euchre-$version-linux.AppImage ;;
  esac
  if [ -f "$downloads/$name" ] && [ "${RELEASE_REPLACE:-0}" != 1 ]; then
    printf 'kept %s (already built; bump the version, or set RELEASE_REPLACE=1)\n' "$name" >&2
  elif [ "$p" = mac ]; then
    want_mac=1
  else
    case " $docker_platforms " in *" $p "*) ;; *) docker_platforms="$docker_platforms $p" ;; esac
  fi
done

# publish <built file> <download name>
publish() {
  cp "$1" "$downloads/.$2.part"
  chmod 0644 "$downloads/.$2.part"
  mv -f "$downloads/.$2.part" "$downloads/$2"
  printf 'published %s (%s bytes)\n' "$2" "$(wc -c < "$downloads/$2" | tr -d ' ')" >&2
}

build_mac() {
  local t root=$tmp/mac app=$tmp/mac/Euchre.app bin
  rustup target add --toolchain "$RUST_VERSION" aarch64-apple-darwin x86_64-apple-darwin >&2
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    step "mac: build $t"
    (cd "$repo" && MACOSX_DEPLOYMENT_TARGET=$MACOS_MIN cargo +"$RUST_VERSION" build --locked --release \
      -p euchre-desktop --bin euchre --target "$t")
  done
  step "mac: Euchre.app and dmg"
  bin=$app/Contents/MacOS/euchre
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  lipo -create -output "$bin" "$target_dir/aarch64-apple-darwin/release/euchre" \
    "$target_dir/x86_64-apple-darwin/release/euchre"
  strip -x "$bin"   # local symbols only: 147 MB -> 123 MB
  lipo "$bin" -verify_arch arm64 && lipo "$bin" -verify_arch x86_64 || die "the Mac binary is not universal"
  [ "$(otool -arch all -l "$bin" | awk '$1 == "minos" { print $2 }' | sort -u)" = "$MACOS_MIN" ] ||
    die "the Mac binary does not target macOS $MACOS_MIN"
  cp "$icons/Euchre.icns" "$app/Contents/Resources/Euchre.icns"
  printf 'APPL????' > "$app/Contents/PkgInfo"
  /usr/bin/python3 -I - "$app/Contents/Info.plist" "$core" "$BUNDLE_ID" "$MACOS_MIN" <<'PY'
import plistlib, sys
path, core, bundle_id, macos_min = sys.argv[1:]
with open(path, "wb") as f:
    plistlib.dump({
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": "Euchre",
        "CFBundleExecutable": "euchre",
        "CFBundleIconFile": "Euchre",
        "CFBundleIdentifier": bundle_id,
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "Euchre",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": core,
        "CFBundleVersion": core,
        "LSApplicationCategoryType": "public.app-category.card-games",
        "LSMinimumSystemVersion": macos_min,
        "NSHighResolutionCapable": True,
        "NSSupportsAutomaticGraphicsSwitching": True,
        "NSHumanReadableCopyright": "Euchre, euchre.sh",
    }, f)
PY
  plutil -lint -s "$app/Contents/Info.plist"
  codesign --force --sign - --timestamp=none "$app"
  codesign --verify --strict "$app"
  ln -s /Applications "$root/Applications"
  hdiutil create -quiet -volname Euchre -srcfolder "$root" -fs HFS+ -format ULMO "$tmp/mac.dmg"
  hdiutil verify -quiet "$tmp/mac.dmg"
  publish "$tmp/mac.dmg" "Euchre-$version-mac.dmg"
}

# fetch_tool <name> <url> <sha256>: a pinned download, kept in $work/tools.
fetch_tool() {
  local f=$work/tools/$1
  [ -f "$f" ] && [ "$(sha256 "$f")" = "$3" ] && return 0
  step "tools: download $1"
  curl -fsSL --retry 3 -o "$f.part" "$2"
  [ "$(sha256 "$f.part")" = "$3" ] || { rm -f "$f.part"; die "$1 does not match its pinned sha256"; }
  chmod 0755 "$f.part"; mv -f "$f.part" "$f"
}

# The cross builder: arm64 Debian 11 (glibc 2.31) with amd64 dev libraries and mingw-w64.
read -r -d '' BUILDER_DOCKERFILE <<'DOCKERFILE' || true
FROM debian:bullseye-slim@sha256:e5b6442dd2e9684cf5e87d8338b5968f3b348636fc0be6d7850a381e3731a2bd

ARG RUST_VERSION=1.99.1
ENV DEBIAN_FRONTEND=noninteractive
# Bullseye's LTS ended 2026-08-31 and deb.debian.org now lists security packages it no longer
# serves (404), so apt reads a frozen snapshot instead.
RUN printf '%s\n' \
      'deb http://snapshot.debian.org/archive/debian/20260824T000000Z bullseye main' \
      'deb http://snapshot.debian.org/archive/debian-security/20260824T000000Z bullseye-security main' \
      'deb http://snapshot.debian.org/archive/debian/20260824T000000Z bullseye-updates main' \
      > /etc/apt/sources.list \
 && echo 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99snapshot \
 && dpkg --add-architecture amd64 \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git file xz-utils zip pkg-config build-essential \
      crossbuild-essential-amd64 \
      libasound2-dev:amd64 libxkbcommon-dev:amd64 \
      gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 binutils-mingw-w64-x86-64 \
 && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:$PATH \
    RUSTUP_TOOLCHAIN=${RUST_VERSION}
RUN curl -sSf https://sh.rustup.rs | CARGO_HOME=/usr/local/cargo sh -s -- -y --no-modify-path \
      --profile minimal --default-toolchain ${RUST_VERSION} \
      --target x86_64-unknown-linux-gnu --target x86_64-pc-windows-gnu \
 && rustc --version && cargo --version

ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CXX_x86_64_unknown_linux_gnu=x86_64-linux-gnu-g++ \
    AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar \
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_PATH_x86_64_unknown_linux_gnu=/usr/lib/x86_64-linux-gnu/pkgconfig \
    PKG_CONFIG_LIBDIR_x86_64_unknown_linux_gnu=/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc \
    CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++ \
    AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
    CARGO_HOME=/cargo \
    CARGO_TARGET_DIR=/target
WORKDIR /src
DOCKERFILE

builder_image() {
  local tag
  tag=$prefix-builder:$(printf '%s' "$BUILDER_DOCKERFILE" | shasum -a 256 | cut -c1-12)
  if ! docker image inspect "$tag" >/dev/null 2>&1; then
    step "docker: build $tag (once; about 5 minutes and 3.5 GB)"
    printf '%s\n' "$BUILDER_DOCKERFILE" | docker build --platform linux/arm64 -t "$tag" - >&2
  fi
  printf '%s\n' "$tag"
}

# Runs inside the builder. /src is the repo (read-only), /job this run's temp dir, /icons
# apps/desktop/assets/icon, /tools the pinned AppImage tools, /cargo and /target cache volumes.
cat > "$tmp/in-docker.sh" <<'IN_DOCKER'
set -euo pipefail
IFS=. read -r major minor patch <<<"$CORE"
for p in "$@"; do
  case $p in
  windows)
    t=x86_64-pc-windows-gnu
    echo "== windows: build $t" >&2
    mkdir -p /job/win && cd /job/win
    cp /icons/Euchre.ico .
    cat > euchre.rc <<RC
#include <winver.h>
1 ICON "Euchre.ico"
1 VERSIONINFO
FILEVERSION     $major,$minor,$patch,0
PRODUCTVERSION  $major,$minor,$patch,0
FILEOS          VOS_NT_WINDOWS32
FILETYPE        VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "euchre.sh"
      VALUE "FileDescription", "Euchre"
      VALUE "FileVersion", "$VERSION"
      VALUE "InternalName", "Euchre"
      VALUE "OriginalFilename", "Euchre.exe"
      VALUE "ProductName", "Euchre"
      VALUE "ProductVersion", "$VERSION"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
RC
    x86_64-w64-mingw32-windres -O coff -i euchre.rc -o res.o
    # The object's name carries its hash, so cargo relinks exactly when the icon or version changes.
    res=/target/euchre-res-$(sha256sum res.o | cut -c1-16).o
    cp res.o "$res"
    (cd /src && cargo rustc --locked --release -p euchre-desktop --bin euchre --target $t \
      -- -C link-arg="$res")
    x86_64-w64-mingw32-strip -o Euchre.exe /target/$t/release/euchre.exe
    x86_64-w64-mingw32-objdump -p Euchre.exe > pe.txt
    grep -q 'Subsystem.*(Windows GUI)' pe.txt || { echo "Euchre.exe is not a GUI app" >&2; exit 1; }
    dlls=$(sed -n 's/.*DLL Name: //p' pe.txt | sort -fu | tr '\n' ' ')
    echo "windows: imports $dlls" >&2
    if echo "$dlls" | grep -Eiq 'libgcc|libstdc|libwinpthread'; then
      echo "Euchre.exe needs a mingw runtime DLL" >&2; exit 1
    fi
    rm -f windows.zip && zip -9 -X -q windows.zip Euchre.exe
    ;;
  linux)
    t=x86_64-unknown-linux-gnu
    echo "== linux: build $t" >&2
    (cd /src && cargo build --locked --release -p euchre-desktop --bin euchre --target $t)
    bin=/target/$t/release/euchre
    echo "linux: needs $(x86_64-linux-gnu-objdump -T "$bin" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)" >&2
    rm -rf /job/linux && app=/job/linux/Euchre.AppDir
    mkdir -p "$app/usr/bin" "$app/usr/share/applications" "$app/usr/share/icons/hicolor/512x512/apps"
    x86_64-linux-gnu-strip -o "$app/usr/bin/euchre" "$bin"
    cat > "$app/sh.euchre.desktop.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Euchre
Comment=Play euchre at the table, online at euchre.sh
Exec=euchre
Icon=sh.euchre.desktop
Categories=Game;CardGame;
Terminal=false
StartupWMClass=sh.euchre.desktop
DESKTOP
    cp "$app/sh.euchre.desktop.desktop" "$app/usr/share/applications/"
    cp /icons/euchre-512.png "$app/sh.euchre.desktop.png"
    cp /icons/euchre-512.png "$app/usr/share/icons/hicolor/512x512/apps/sh.euchre.desktop.png"
    ln -s sh.euchre.desktop.png "$app/.DirIcon"
    ln -s usr/bin/euchre "$app/AppRun"
    cd /job/linux
    ARCH=x86_64 /tools/appimagetool-aarch64.AppImage --appimage-extract-and-run --no-appstream \
      --runtime-file /tools/runtime-x86_64 "$app" /job/linux.AppImage >&2
    ;;
  esac
done
IN_DOCKER

build_docker() {   # build_docker <platform>...
  local image
  case " $* " in *" linux "*)
    fetch_tool appimagetool-aarch64.AppImage "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA256"
    fetch_tool runtime-x86_64 "$RUNTIME_URL" "$RUNTIME_SHA256" ;;
  esac
  image=$(builder_image)
  step "docker:$docker_platforms"
  docker run --rm --init --platform linux/arm64 \
    -v "$repo:/src:ro" -v "$tmp:/job" -v "$icons:/icons:ro" -v "$work/tools:/tools:ro" \
    -v "$prefix-cargo:/cargo" -v "$prefix-target:/target" \
    -e EUCHRE_DEFAULT_SERVER_URL -e VERSION="$version" -e CORE="$core" \
    "$image" bash /job/in-docker.sh "$@"
  if [ -f "$tmp/win/windows.zip" ]; then publish "$tmp/win/windows.zip" "Euchre-$version-windows.zip"; fi
  if [ -f "$tmp/linux.AppImage" ]; then publish "$tmp/linux.AppImage" "Euchre-$version-linux.AppImage"; fi
}

if [ "$want_mac" = 1 ]; then build_mac; fi
if [ -n "$docker_platforms" ]; then build_docker $docker_platforms; fi

step "manifest"
/usr/bin/python3 -I - "$downloads" "$version" <<'PY'
import hashlib, json, os, re, sys

folder, version = sys.argv[1:]
names = {
    "mac": f"Euchre-{version}-mac.dmg",
    "windows": f"Euchre-{version}-windows.zip",
    "linux": f"Euchre-{version}-linux.AppImage",
}
platforms = {}
for key, name in names.items():
    path = os.path.join(folder, name)
    if not os.path.isfile(path) or os.path.islink(path):
        continue
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    platforms[key] = {"file": name, "bytes": os.path.getsize(path), "sha256": digest.hexdigest()}
if not platforms:
    sys.exit(f"release-desktop: no downloads for {version} in {folder}")

part = os.path.join(folder, ".manifest.json.part")
with open(part, "w") as f:
    json.dump({"version": version, "platforms": platforms}, f, indent=2)
    f.write("\n")
    f.flush()
    os.fsync(f.fileno())
os.replace(part, os.path.join(folder, "manifest.json"))

# Only now is it safe to drop other versions and leftovers of interrupted copies.
ours = re.compile(r"^\.?Euchre-.+-(mac\.dmg|windows\.zip|linux\.AppImage)(\.part)?$")
for name in sorted(os.listdir(folder)):
    if ours.match(name) and name not in names.values():
        os.remove(os.path.join(folder, name))
        print(f"removed {name}", file=sys.stderr)
missing = sorted(set(names) - set(platforms))
print(json.dumps({"version": version, "platforms": platforms}, indent=2))
if missing:
    print(f"not in this release: {', '.join(missing)}", file=sys.stderr)
PY
if [ "$upload" = 1 ]; then
  step "upload to bucket euchre-downloads"
  # Credentials go from the railway CLI straight into bun's stdin; nothing is written or printed.
  railway bucket credentials -b euchre-downloads --json |
    bun run "$repo/scripts/upload-downloads.ts" "$downloads"
fi
step "done"
