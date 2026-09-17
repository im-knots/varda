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
EXCEPTIONS="packaging/flatpak/linter-exceptions.json"
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

# --- Manifest lint --------------------------------------------------------------------
# flatpak-builder-lint is Flathub's own manifest linter. It runs in seconds and catches
# structural mistakes that would otherwise surface half an hour into a build, which is
# how several rounds of this were spent.
#
# It runs AFTER cargo-sources.json is generated, not before. The linter shells out to
# `flatpak-builder --show-manifest`, which resolves the manifest completely, including
# the `- cargo-sources.json` include. With that file absent, flatpak-builder's
# load_sources_from_json returns NULL, its deserializer returns FALSE, and the linter
# reports manifest-json-warnings:
#
#   Failed to deserialize "sources" property ... for an object of type "BuilderModule"
#
# which reads like a malformed manifest and is really a missing file. Linting a manifest
# whose sources are not all present is not linting the manifest that gets built.
#
# --exceptions is required for --user-exceptions to be consulted at all (cli.py gates it
# on enable_exceptions). With a user exceptions file set, no remote Flathub lookup is
# made, so this needs no network beyond installing the linter itself.
lint_manifest() {
  if ! flatpak info org.flatpak.Builder >/dev/null 2>&1; then
    echo "==> Installing org.flatpak.Builder (for the linter)"
    flatpak remote-add --if-not-exists --user flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    flatpak install --user --noninteractive flathub org.flatpak.Builder || {
      echo "::warning::could not install flatpak-builder-lint; skipping manifest lint"
      return 0
    }
  fi
  test -f "packaging/flatpak/cargo-sources.json" || {
    echo "::error::lint_manifest ran before cargo-sources.json was generated"
    return 1
  }
  echo "==> Linting the manifest"
  flatpak run --command=flatpak-builder-lint org.flatpak.Builder \
    --exceptions --user-exceptions "$EXCEPTIONS" \
    manifest "$MANIFEST" || {
    echo "::error::flatpak-builder-lint rejected the manifest"
    return 1
  }
  echo "    manifest passes"
}

if [ "$SKIP_DEPS" = false ]; then
  echo "==> Installing flatpak-builder"
  sudo apt-get update -qq
  sudo apt-get install -y -qq \
    flatpak flatpak-builder \
    python3-aiohttp python3-tomlkit python3-yaml >/dev/null
fi

echo "==> Adding Flathub and the runtime"
flatpak remote-add --if-not-exists --user flathub https://dl.flathub.org/repo/flathub.flatpakrepo
mapfile -t SDK_EXTENSIONS < <(
  python3 -c "
import sys, yaml
m = yaml.safe_load(open(sys.argv[1]))
for ext in m.get('sdk-extensions', []):
    print(ext)
" "$MANIFEST"
)
echo "    sdk extensions: ${SDK_EXTENSIONS[*]:-none}"

REFS=(
  "org.freedesktop.Platform//$RUNTIME_VERSION"
  "org.freedesktop.Sdk//$RUNTIME_VERSION"
)
for ext in ${SDK_EXTENSIONS[@]+"${SDK_EXTENSIONS[@]}"}; do
  REFS+=("$ext//$RUNTIME_VERSION")
done
printf '    installing %s\n' "${REFS[@]}"
flatpak install --user --noninteractive flathub "${REFS[@]}"

# --- Vendor the cargo dependencies ----------------------------------------------------
# The build sandbox has no network, so every crate must be a declared source.
# flatpak-cargo-generator reads Cargo.lock, including its checksums, and emits them.
GEN="/tmp/flatpak-cargo-generator.py"
if [ ! -f "$GEN" ]; then
  echo "==> Fetching flatpak-cargo-generator"
  curl -fsSL -o "$GEN" \
    https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
fi

# Check the generator's imports up front. It fails with a bare ModuleNotFoundError
# otherwise, which reads like a bug in this script rather than a missing package.
for mod in aiohttp tomlkit yaml; do
  python3 -c "import $mod" 2>/dev/null \
    || { echo "::error::python module '$mod' is missing; flatpak-cargo-generator needs it"; exit 1; }
done

echo "==> Generating cargo-sources.json from Cargo.lock"
python3 "$GEN" Cargo.lock -o "packaging/flatpak/cargo-sources.json"
# Roughly a thousand crates; a tiny file means the generator silently produced nothing,
# which would fail much later as an opaque offline-fetch error inside the sandbox.
SRC_COUNT=$(python3 -c "import json;print(len(json.load(open('packaging/flatpak/cargo-sources.json'))))")
echo "    $SRC_COUNT source entries"
[ "$SRC_COUNT" -gt 100 ] \
  || { echo "::error::cargo-sources.json has only $SRC_COUNT entries; the generator did not run properly"; exit 1; }

# Every source the manifest names now exists, so the linter sees what flatpak-builder
# will see. Still before the build, which is the expensive part.
lint_manifest

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
