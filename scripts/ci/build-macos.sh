#!/usr/bin/env bash
# Build Varda macOS DMG (Universal binary)
# Usage: ./scripts/ci/build-macos.sh [--skip-deps] [--skip-build] [--tag v0.2.0]
#
# Produces: Varda-macOS-universal.dmg in the project root
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

SKIP_DEPS=false
SKIP_BUILD=false
NATIVE_ONLY=false
TAG="${GITHUB_REF_NAME:-v0.0.0-dev}"
TARGET=""
LIB_DIR=""
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-deps)    SKIP_DEPS=true ;;
    --skip-build)   SKIP_BUILD=true ;;
    --native-only)  NATIVE_ONLY=true ;;
    --tag=*)        TAG="${1#--tag=}" ;;
    --tag)          shift; TAG="$1" ;;
    --target=*)     TARGET="${1#--target=}" ;;
    --target)       shift; TARGET="$1" ;;
    --lib-dir=*)    LIB_DIR="${1#--lib-dir=}" ;;
    --lib-dir)      shift; LIB_DIR="$1" ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
  shift
done
VERSION="${TAG#v}"
NATIVE_ARCH="$(uname -m)"
if [ "$NATIVE_ARCH" = "arm64" ]; then
  NATIVE_TARGET="aarch64-apple-darwin"
  CROSS_TARGET="x86_64-apple-darwin"
else
  NATIVE_TARGET="x86_64-apple-darwin"
  CROSS_TARGET="aarch64-apple-darwin"
fi
# Override target if explicitly specified
if [ -n "$TARGET" ]; then
  NATIVE_TARGET="$TARGET"
fi

echo "==> Project root: $PROJECT_ROOT"
echo "==> Version: $VERSION (tag: $TAG)"

# --- Install system dependencies ---
if [ "$SKIP_DEPS" = false ]; then
  echo "==> Installing system dependencies..."
  brew tap homebrew-ffmpeg/ffmpeg
  brew install homebrew-ffmpeg/ffmpeg/ffmpeg --with-srt
  brew install cmake pkg-config create-dmg
fi

# --- Build architectures ---
if [ "$SKIP_BUILD" = false ]; then
  echo "==> Building $NATIVE_TARGET..."
  cargo build --release --target "$NATIVE_TARGET"

  if [ "$NATIVE_ONLY" = false ]; then
    echo "==> Building $CROSS_TARGET (cross)..."
    cargo build --release --target "$CROSS_TARGET"
  fi
fi

# --- Create binary (universal or single-arch) ---
if [ "$NATIVE_ONLY" = true ]; then
  echo "==> Using native-only binary ($NATIVE_TARGET)..."
  cp "target/$NATIVE_TARGET/release/varda" varda-universal
else
  echo "==> Creating universal binary..."
  lipo -create \
    "target/aarch64-apple-darwin/release/varda" \
    "target/x86_64-apple-darwin/release/varda" \
    -output varda-universal
  lipo -info varda-universal
fi

# --- Build .app bundle ---
echo "==> Building .app bundle..."
rm -rf Varda.app
mkdir -p Varda.app/Contents/MacOS
mkdir -p Varda.app/Contents/Frameworks
mkdir -p Varda.app/Contents/Resources/shaders
mkdir -p Varda.app/Contents/Resources/licenses

cp varda-universal Varda.app/Contents/MacOS/varda
cp assets/icon.png Varda.app/Contents/Resources/varda.png

# Bundle shaders
cp -r shaders/* Varda.app/Contents/Resources/shaders/

# Bundle FFmpeg dylibs and fix load paths using @rpath (Apple-recommended approach).
# See: https://developer.apple.com/forums/thread/736728
#
# Strategy:
#   1. Copy each dylib into Contents/Frameworks/
#   2. Set each dylib's install name (id) to @rpath/<basename>
#   3. Rewrite the main binary's load commands from absolute paths to @rpath/<basename>
#   4. Add LC_RPATH = @executable_path/../Frameworks to the main binary
#   5. Fix inter-library references (e.g. libavcodec → libavutil) the same way
#
# This works regardless of where libs were built (Homebrew, /tmp cross-compile, etc.)
# because we use otool -L to discover the actual baked-in paths.

HOMEBREW_LIB="${LIB_DIR:-$(brew --prefix)/lib}"
BINARY="Varda.app/Contents/MacOS/varda"
FRAMEWORKS="Varda.app/Contents/Frameworks"

# Step 1: Discover which dylibs the binary actually needs via otool, copy them in.
# This is authoritative — we copy exactly the filenames the linker recorded,
# not whatever find happens to return.
BUNDLED_LIBS=()
for lib in libavcodec libavformat libavutil libswscale libswresample libavdevice libavfilter libsrt; do
  old_path=$(otool -L "$BINARY" | grep "$lib" | awk '{print $1}' | head -1 || true)
  [ -z "$old_path" ] && continue
  case "$old_path" in @*) continue ;; esac  # already relative — skip

  libname=$(basename "$old_path")
  # Resolve the source file (may be a symlink chain)
  src="$HOMEBREW_LIB/$libname"
  if [ ! -e "$src" ]; then
    # Fallback: search for any matching file
    src=$(find "$HOMEBREW_LIB" -maxdepth 1 -name "${lib}*.dylib" | head -1)
  fi
  if [ -n "$src" ] && [ -e "$src" ]; then
    cp -L "$src" "$FRAMEWORKS/$libname"   # -L follows symlinks, copies actual file
    BUNDLED_LIBS+=("$libname")
  fi
done

# Step 2: Set each dylib's install name (id) to @rpath/<basename>
for libname in "${BUNDLED_LIBS[@]}"; do
  install_name_tool -id "@rpath/$libname" "$FRAMEWORKS/$libname"
done

# Step 3: Rewrite the main binary's load commands from absolute paths to @rpath/<basename>
for libname in "${BUNDLED_LIBS[@]}"; do
  lib_prefix=$(echo "$libname" | sed 's/\.[0-9].*//')
  otool -L "$BINARY" | { grep "$lib_prefix" || true; } | awk '{print $1}' | while read -r old_path; do
    case "$old_path" in @*) continue ;; esac
    install_name_tool -change "$old_path" "@rpath/$libname" "$BINARY"
  done
done

# Step 4: Add LC_RPATH so dyld resolves @rpath to Contents/Frameworks/
install_name_tool -add_rpath "@executable_path/../Frameworks" "$BINARY" 2>/dev/null || true

# Step 5: Fix inter-library references and bundle transitive deps.
# Run multiple passes: first bundle any missing transitive deps, then rewrite
# all non-system absolute paths to @rpath. Two passes handles typical depth.
for _pass in 1 2 3; do
  for fw_dylib in "$FRAMEWORKS"/*.dylib; do
    deps=$(otool -L "$fw_dylib" | tail -n +2 | awk '{print $1}')
    for dep_path in $deps; do
      case "$dep_path" in /usr/*|/System/*) continue ;; esac
      dep_name=$(basename "$dep_path")
      # Already bundled? Just rewrite the reference if needed
      if [ -f "$FRAMEWORKS/$dep_name" ]; then
        case "$dep_path" in @*) continue ;; esac  # already @rpath — nothing to do
        install_name_tool -change "$dep_path" "@rpath/$dep_name" "$fw_dylib"
        continue
      fi
      # Not bundled — try to find and copy it
      if [ "${dep_path#@rpath/}" != "$dep_path" ]; then
        # @rpath reference but not bundled — search LIB_DIR/HOMEBREW_LIB
        src_path=""
        [ -f "$HOMEBREW_LIB/$dep_name" ] && src_path="$HOMEBREW_LIB/$dep_name"
      else
        # Absolute path — use it directly, fallback to LIB_DIR
        src_path="$dep_path"
        [ ! -f "$src_path" ] && [ -f "$HOMEBREW_LIB/$dep_name" ] && src_path="$HOMEBREW_LIB/$dep_name"
      fi
      if [ -n "$src_path" ] && [ -f "$src_path" ]; then
        cp -L "$src_path" "$FRAMEWORKS/$dep_name"
        install_name_tool -id "@rpath/$dep_name" "$FRAMEWORKS/$dep_name"
        BUNDLED_LIBS+=("$dep_name")
        case "$dep_path" in @*) ;; *)
          install_name_tool -change "$dep_path" "@rpath/$dep_name" "$fw_dylib"
        ;; esac
      fi
    done
  done
done

# Bundle NDI SDK (runtime-loaded via libloading, universal binary from cask)
NDI_LIB="/Library/NDI SDK for Apple/lib/macOS/libndi.dylib"
if [ -f "$NDI_LIB" ]; then
  echo "==> Bundling NDI SDK..."
  cp "$NDI_LIB" Varda.app/Contents/Frameworks/libndi.dylib
  install_name_tool -id "@rpath/libndi.dylib" Varda.app/Contents/Frameworks/libndi.dylib 2>/dev/null || true
else
  echo "==> NDI SDK not found, skipping bundle (install with: brew install --cask libndi)"
fi

# Bundle ONNX Runtime dylib (arm64-only; used by ort load-dynamic for face detection)
ORT_VERSION="1.24.1"
ORT_URL="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-osx-arm64-${ORT_VERSION}.tgz"
ORT_TMP="/tmp/ort-bundle"
echo "==> Downloading ONNX Runtime v${ORT_VERSION} (arm64)..."
rm -rf "$ORT_TMP"
mkdir -p "$ORT_TMP"
if curl -fsSL "$ORT_URL" | tar xz -C "$ORT_TMP" 2>/dev/null; then
  ORT_DYLIB=$(find "$ORT_TMP" -name "libonnxruntime.*.dylib" -type f | head -1)
  if [ -n "$ORT_DYLIB" ]; then
    cp "$ORT_DYLIB" "$FRAMEWORKS/libonnxruntime.dylib"
    install_name_tool -id "@rpath/libonnxruntime.dylib" "$FRAMEWORKS/libonnxruntime.dylib" 2>/dev/null || true
    echo "==> Bundled ONNX Runtime from $ORT_DYLIB"
  else
    echo "==> WARN: ONNX Runtime dylib not found in download"
  fi
  rm -rf "$ORT_TMP"
else
  echo "==> WARN: Failed to download ONNX Runtime; face detection will be unavailable"
fi

# Bundle licenses
cp LICENSE Varda.app/Contents/Resources/licenses/ 2>/dev/null || echo "MIT License" > Varda.app/Contents/Resources/licenses/LICENSE
echo "FFmpeg is licensed under the LGPL v2.1+. See https://ffmpeg.org/legal.html" > Varda.app/Contents/Resources/licenses/FFMPEG-LICENSE


# Info.plist
cat > Varda.app/Contents/Info.plist << PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Varda</string>
  <key>CFBundleDisplayName</key><string>Varda</string>
  <key>CFBundleIdentifier</key><string>com.varda.app</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>varda</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>varda.png</string>
  <key>LSMinimumSystemVersion</key><string>10.15</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSCameraUsageDescription</key><string>Varda uses the camera for live video input</string>
  <key>NSMicrophoneUsageDescription</key><string>Varda uses the microphone for audio-reactive visuals</string>
</dict>
</plist>
PLIST_EOF

# --- Code sign ---
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
echo ""
echo "The 'varda' CLI wrapper is auto-installed on first launch."