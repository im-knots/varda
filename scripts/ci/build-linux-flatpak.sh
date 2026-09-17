#!/usr/bin/env bash
# Build the Varda Flatpak bundle.
# Usage: ./scripts/ci/build-linux-flatpak.sh --version <x.y.z> [--outdir dist]
#
# Produces: dist/Varda-<version>-x86_64.flatpak  (a single-file bundle)
#
# The artifact for an installed Varda whose dependencies Flathub patches centrally.
# See spec/release-strategy.md sections 5 and 14.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

APPID="io.github.im_knots.varda"
MANIFEST="packaging/flatpak/$APPID.yml"
RUNTIME_VERSION="$(grep -m1 'runtime-version:' "$MANIFEST" | tr -d " '" | cut -d: -f2)"

VERSION=""
OUTDIR="dist"
SKIP_DEPS=false
while [ $# -gt 0 ]; do
  case "$1" in
    --version)   VERSION="$2"; shift 2 ;;
    --outdir)    OUTDIR="$2"; shift 2 ;;
    --skip-deps) SKIP_DEPS=true; shift ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done
if [ -z "$VERSION" ]; then
  VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
fi
VERSION="${VERSION#v}"
mkdir -p "$OUTDIR"

echo "==> Varda $VERSION Flatpak (runtime $RUNTIME_VERSION)"

if [ "$SKIP_DEPS" = false ]; then
  echo "==> Installing flatpak-builder"
  sudo apt-get update -qq
  sudo apt-get install -y -qq flatpak flatpak-builder python3-aiohttp python3-toml >/dev/null
fi

echo "==> Adding Flathub and the runtime"
flatpak remote-add --if-not-exists --user flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive flathub \
  "org.freedesktop.Platform//$RUNTIME_VERSION" \
  "org.freedesktop.Sdk//$RUNTIME_VERSION" \
  "org.freedesktop.Sdk.Extension.rust-stable//$RUNTIME_VERSION"

# --- Vendor the cargo dependencies ----------------------------------------------------
# The build sandbox has no network, so every crate must be a declared source.
# flatpak-cargo-generator reads Cargo.lock, including its checksums, and emits them.
GEN="/tmp/flatpak-cargo-generator.py"
if [ ! -f "$GEN" ]; then
  echo "==> Fetching flatpak-cargo-generator"
  curl -fsSL -o "$GEN" \
    https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
fi

echo "==> Generating cargo-sources.json from Cargo.lock"
python3 "$GEN" Cargo.lock -o "packaging/flatpak/cargo-sources.json"
# Roughly a thousand crates; a tiny file means the generator silently produced nothing,
# which would fail much later as an opaque offline-fetch error inside the sandbox.
SRC_COUNT=$(python3 -c "import json;print(len(json.load(open('packaging/flatpak/cargo-sources.json'))))")
echo "    $SRC_COUNT source entries"
[ "$SRC_COUNT" -gt 100 ] \
  || { echo "::error::cargo-sources.json has only $SRC_COUNT entries; the generator did not run properly"; exit 1; }

# --- Build ----------------------------------------------------------------------------
echo "==> Building (this compiles Servo; expect it to be slow)"
flatpak-builder --user --disable-rofiles-fuse --force-clean \
  --repo="/tmp/varda-flatpak-repo" \
  /tmp/varda-flatpak-build "$MANIFEST"

PKG="$OUTDIR/Varda-$VERSION-x86_64.flatpak"
echo "==> Exporting single-file bundle"
flatpak build-bundle "/tmp/varda-flatpak-repo" "$PKG" "$APPID" "$RUNTIME_VERSION"

test -f "$PKG" || { echo "::error::no bundle produced"; exit 1; }
echo "==> Done"
ls -lh "$PKG"
