#!/usr/bin/env bash
# Combine arm64 and x86_64 macOS builds into a universal .app + DMG
# Usage: ./scripts/ci/combine-universal.sh <arm64-app-dir> <x86-app-dir>
#
# Takes two .app bundles (one per arch) and produces:
#   - Varda.app with universal binary and universal dylibs
#   - Varda-macOS-universal.dmg
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

RENAMED=()
ARM64_APP="${1:?Usage: combine-universal.sh <arm64-app-dir> <x86-app-dir>}"
X86_APP="${2:?Usage: combine-universal.sh <arm64-app-dir> <x86-app-dir>}"

echo "==> arm64 app: $ARM64_APP"
echo "==> x86_64 app: $X86_APP"

# Use arm64 .app as the base (has shaders, plist, licenses, icon)
rm -rf Varda.app
cp -R "$ARM64_APP" Varda.app

# --- Lipo the main binary ---
echo "==> Creating universal binary..."
lipo -create \
  "$ARM64_APP/Contents/MacOS/varda" \
  "$X86_APP/Contents/MacOS/varda" \
  -output Varda.app/Contents/MacOS/varda
lipo -info Varda.app/Contents/MacOS/varda

# --- Lipo each Framework dylib ---
echo "==> Creating universal dylibs..."
for arm64_dylib in "$ARM64_APP/Contents/Frameworks/"*.dylib; do
  basename=$(basename "$arm64_dylib")
  x86_dylib="$X86_APP/Contents/Frameworks/$basename"

  if [ -f "$x86_dylib" ]; then
    # Check that the two dylibs actually have different architectures before
    # lipo — if both slices carry the same arch, lipo will fail.
    arm64_archs=$(lipo -info "$arm64_dylib" 2>/dev/null | sed 's/.*: //')
    x86_archs=$(lipo -info "$x86_dylib" 2>/dev/null | sed 's/.*: //')
    if [ "$arm64_archs" = "$x86_archs" ]; then
      echo "::error::$basename is $arm64_archs in both builds; one slice was built for the wrong architecture"
      MISSING_SLICES=$((${MISSING_SLICES:-0} + 1))
    else
      echo "    lipo: $basename"
      lipo -create "$arm64_dylib" "$x86_dylib" -output "Varda.app/Contents/Frameworks/$basename"
    fi
  else
    # Try to find matching lib by prefix (version numbers may differ)
    lib_prefix=$(echo "$basename" | sed 's/\.[0-9].*/.dylib/')
    x86_match=$(find "$X86_APP/Contents/Frameworks/" -name "${lib_prefix%.dylib}*" -maxdepth 1 | head -1)
    if [ -n "$x86_match" ]; then
      # The two Homebrew installs resolved different versions of the same library
      # (x265 217 on arm64, 216 on Intel, say), so the slices have different filenames.
      # Merge into the arm64 name and record the rename: the x86_64 slice's load commands
      # still point at its own filename and have to be repointed, or dyld looks for a file
      # that is about to not exist.
      echo "    lipo: $basename (x86 match: $(basename "$x86_match"))"
      RENAMED+=("$(basename "$x86_match")=$basename")
      lipo -create "$arm64_dylib" "$x86_match" -output "Varda.app/Contents/Frameworks/$basename"
    else
      # Fatal, not a warning. An app advertised as universal that carries a
      # single-architecture dylib fails at dyld load time on the other architecture,
      # which is a user's problem rather than ours if it ships.
      echo "::error::no x86_64 match for $basename; the universal app would be incomplete"
      MISSING_SLICES=$((${MISSING_SLICES:-0} + 1))
    fi
  fi
done

# Repoint references from the x86_64-only filename to the merged one, across the main
# binary and every bundled dylib. Without this the Intel slice asks dyld for a library
# that was merged away, and the only alternative is shipping a second thin copy of it.
for rename in ${RENAMED[@]+"${RENAMED[@]}"}; do
  from="${rename%%=*}"
  to="${rename##*=}"
  echo "    repoint: @rpath/$from -> @rpath/$to"
  install_name_tool -change "@rpath/$from" "@rpath/$to" Varda.app/Contents/MacOS/varda 2>/dev/null || true
  for fw in Varda.app/Contents/Frameworks/*.dylib; do
    install_name_tool -change "@rpath/$from" "@rpath/$to" "$fw" 2>/dev/null || true
  done
done

if [ "${MISSING_SLICES:-0}" -ne 0 ]; then
  echo "::error::${MISSING_SLICES} dylib(s) could not be made universal"
  exit 1
fi

# Add any x86-only dylibs that don't exist in arm64
for x86_dylib in "$X86_APP/Contents/Frameworks/"*.dylib; do
  basename=$(basename "$x86_dylib")
  # Skip anything already merged under the arm64 filename above; copying it here would
  # put a thin x86_64 duplicate of a library the bundle already carries universally.
  skip=false
  for rename in ${RENAMED[@]+"${RENAMED[@]}"}; do
    [ "${rename%%=*}" = "$basename" ] && skip=true
  done
  if [ "$skip" = true ]; then
    echo "    skip x86-only duplicate: $basename (merged as ${rename##*=})"
    continue
  fi
  if [ ! -f "Varda.app/Contents/Frameworks/$basename" ]; then
    echo "    copy x86-only: $basename"
    cp "$x86_dylib" "Varda.app/Contents/Frameworks/$basename"
  fi
done

# --- Re-sign ---
echo "==> Ad-hoc code signing..."
codesign --force --deep --sign - Varda.app

# --- Create DMG ---
echo "==> Creating DMG..."
rm -f Varda-macOS-universal.dmg
create-dmg \
  --volname "Varda" \
  --window-pos 200 120 \
  --window-size 600 400 \
  --icon-size 100 \
  --icon "Varda.app" 150 190 \
  --app-drop-link 450 190 \
  --no-internet-enable \
  "Varda-macOS-universal.dmg" \
  "Varda.app"

echo "==> Done: Varda-macOS-universal.dmg"
ls -lh Varda-macOS-universal.dmg
