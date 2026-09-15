#!/usr/bin/env bash
# Verify a Varda macOS DMG mounts, contains both architecture slices, and runs.
# Usage: ./scripts/ci/smoke-macos-dmg.sh <path-to-Varda-macOS-universal.dmg>
#
# Run on BOTH a native arm64 runner and a macos-15-intel runner. Each slice is now built
# natively on its own hardware, but building and running are still different claims: this
# is what proves the lipo merge produced a binary that actually launches on both
# architectures, with every bundled dylib resolving.
#
# See spec/release-strategy.md section 12.
set -euo pipefail

DMG="${1:?Usage: smoke-macos-dmg.sh <path-to-dmg>}"
WORK="$(mktemp -d)"
MOUNT="$WORK/mnt"

cleanup() {
  hdiutil detach "$MOUNT" -quiet 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "==> Host"
echo "    macOS $(sw_vers -productVersion), arch $(uname -m)"

echo "==> Mounting $(basename "$DMG")"
mkdir -p "$MOUNT"
hdiutil attach "$DMG" -mountpoint "$MOUNT" -nobrowse -quiet

APP_SRC="$MOUNT/Varda.app"
test -d "$APP_SRC" || { echo "::error::no Varda.app inside the DMG"; exit 1; }
cp -R "$APP_SRC" "$WORK/Varda.app"
hdiutil detach "$MOUNT" -quiet

APP="$WORK/Varda.app"
BIN="$APP/Contents/MacOS/varda"
failed=0

# Artifacts downloaded by actions/download-artifact are not quarantined, but a DMG
# opened any other way would be. Strip it so Gatekeeper is not what this measures.
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true

# --- 1. Both slices present ---
# This is what catches a broken lipo merge in combine-universal.sh. A DMG with only the
# native slice would pass every other check on the runner that built it.
echo "==> Checking universal binary"
ARCHS="$(lipo -archs "$BIN" 2>/dev/null || echo '')"
echo "    varda: $ARCHS"
for want in x86_64 arm64; do
  case " $ARCHS " in
    *" $want "*) ;;
    *) echo "FAIL: universal binary is missing the $want slice"; failed=1 ;;
  esac
done

# The bundled FFmpeg dylibs are lipo-merged by the same script. If one of them is not
# universal, the app launches on the runner that matches it and dies on the other.
echo "==> Checking bundled dylibs are universal"
thin=0
while IFS= read -r dylib; do
  a="$(lipo -archs "$dylib" 2>/dev/null || echo '')"
  case " $a " in
    *" x86_64 "*) case " $a " in *" arm64 "*) continue ;; esac ;;
  esac
  echo "    thin: $(basename "$dylib") [$a]"
  thin=$((thin + 1))
done < <(find "$APP/Contents/Frameworks" -name '*.dylib' -type f 2>/dev/null)
if [ "$thin" -gt 0 ]; then
  echo "FAIL: $thin bundled dylib(s) are not universal"
  failed=1
else
  echo "  ok: all bundled dylibs carry both slices"
fi

# --- 2. Shaders shipped where the binary looks for them ---
# Contents/MacOS/varda resolves ../Resources/shaders (BUNDLED_SHADERS_RELATIVE in
# src/internal/registry/mod.rs). Drift here means an app that starts with an empty
# shader library, which nothing else in this script would notice.
echo "==> Checking the shader library"
count="$(find "$APP/Contents/Resources/shaders" -name '*.fs' 2>/dev/null | wc -l | tr -d ' ')"
if [ "$count" -lt 100 ]; then
  echo "FAIL: found $count bundled shaders in Contents/Resources/shaders, expected the full library"
  failed=1
else
  echo "  ok: $count shaders bundled"
fi

# --- 3. It actually launches on this architecture ---
# --version is handled by clap and returns before any GPU, window or audio
# initialization, so this needs no display and no GPU. It does exercise the full dyld
# load of every bundled dylib, which is the failure this is looking for.
echo "==> Launching (native $(uname -m))"
if out="$("$BIN" --version 2>&1)"; then
  echo "  ok: $out"
else
  echo "FAIL: varda --version exited non-zero"
  echo "$out" | sed 's/^/    /'
  failed=1
fi

if [ "$failed" -ne 0 ]; then
  echo "==> SMOKE TEST FAILED"
  exit 1
fi
echo "==> SMOKE TEST PASSED"
