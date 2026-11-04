#!/usr/bin/env bash
# Verify every load command in a macOS .app resolves to the OS or to the bundle.
# Usage: ./scripts/ci/check-macos-bundle.sh <path-to-Varda.app>
#
# Launching the app cannot prove this on a machine with Homebrew: an absolute Homebrew
# path resolves there and fails only for users. GitHub's macOS runners have Homebrew, so
# this reads the load commands of every slice of every image.
set -euo pipefail

APP="${1:?Usage: check-macos-bundle.sh <path-to-Varda.app>}"
FRAMEWORKS="$APP/Contents/Frameworks"
failed=0
checked=0

check_slice() {
  local image="$1" arch="$2" dep
  for dep in $(otool -arch "$arch" -L "$image" | tail -n +2 | awk '{print $1}'); do
    case "$dep" in
      # Intel Homebrew is under /usr/local, so this must stay narrower than /usr/*.
      /usr/lib/*|/System/*) ;;
      @rpath/*|@loader_path/*|@executable_path/*)
        if [ ! -f "$FRAMEWORKS/$(basename "$dep")" ]; then
          echo "FAIL: $(basename "$image") [$arch] loads $dep, which is not in Frameworks"
          failed=1
        fi
        ;;
      *)
        echo "FAIL: $(basename "$image") [$arch] loads $dep, which is outside the bundle"
        failed=1
        ;;
    esac
  done
}

echo "==> Checking load commands"
for image in "$APP/Contents/MacOS/varda" "$FRAMEWORKS"/*.dylib; do
  [ -f "$image" ] || continue
  for arch in $(lipo -archs "$image"); do
    check_slice "$image" "$arch"
    checked=$((checked + 1))
  done
done

if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "  ok: $checked slices load only the OS and the bundle"
