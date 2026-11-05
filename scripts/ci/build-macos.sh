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
  # libfreenect (Kinect v1) backs the `depth` feature, which both slices now carry:
  # the x86_64 slice is built natively on a macos-15-intel runner, where Homebrew
  # provides an x86_64 libfreenect.
  brew install libfreenect
fi

# Cargo feature flags for a given target.
#
# Two features are off on x86_64, each for its own reason:
#   face-detection - ONNX Runtime has no x86_64 macOS dylib.
#   html           - Servo's start_render blocks indefinitely on x86_64 and never
#                    returns a frame.
#                    That was observed under Rosetta, which was the only way to run
#                    the slice before the macos-15-intel runner existed. It stays off
#                    until someone retests it on native Intel hardware.
#
# depth and screen-capture are NOT in that list. They were lost to a blunt
# --no-default-features back when this slice was cross-compiled, with no reason
# recorded for either. libfreenect builds for x86_64 and ScreenCaptureKit is
# available on Intel Macs, so a native Intel build keeps both. Before this, the
# "universal" DMG gave Intel users a Varda with no screen capture and no depth
# sensors, and nothing said so.
cargo_features_for() {
  case "$1" in
    x86_64-apple-darwin) echo "--no-default-features --features depth,screen-capture" ;;
    *) echo "" ;;
  esac
}

# Homebrew's prefix is not on the default linker search path, so the `depth`
# feature's `-lfreenect` will not resolve without it. Both slices now enable depth,
# so this keys off the feature rather than off which slice is being built.
link_env_for() {
  local features
  features="$(cargo_features_for "$1")"
  if [[ "$features" != *--no-default-features* || "$features" == *depth* ]]; then
    echo "LIBRARY_PATH=$(brew --prefix)/lib:${LIBRARY_PATH:-}"
  fi
}

# freetype-sys links the Homebrew dylib when pkg-config finds it and compiles its vendored
# copy otherwise. Only the arm64 slice has Servo, so a dylib would exist in one slice and
# could not be merged into a universal bundle.
export FREETYPE2_NO_PKG_CONFIG=1

# --- Build architectures ---
if [ "$SKIP_BUILD" = false ]; then
  echo "==> Building $NATIVE_TARGET..."
  env $(link_env_for "$NATIVE_TARGET") \
    cargo build --release --target "$NATIVE_TARGET" $(cargo_features_for "$NATIVE_TARGET")

  if [ "$NATIVE_ONLY" = false ]; then
    echo "==> Building $CROSS_TARGET (cross)..."
    env $(link_env_for "$CROSS_TARGET") \
      cargo build --release --target "$CROSS_TARGET" $(cargo_features_for "$CROSS_TARGET")
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

# Bundle every non-system dylib the binary loads, directly or transitively, and rewrite
# each reference to @rpath/<basename> (Apple's recommended layout, see
# https://developer.apple.com/forums/thread/736728). otool is the source of truth, so
# this covers any library the linker recorded, wherever it came from.

HOMEBREW_LIB="${LIB_DIR:-$(brew --prefix)/lib}"
BINARY="Varda.app/Contents/MacOS/varda"
FRAMEWORKS="Varda.app/Contents/Frameworks"

# The file behind a load command. Prints nothing if it cannot be found.
lib_source() {
  local dep="$1" name
  name="$(basename "$dep")"
  if [ -e "$HOMEBREW_LIB/$name" ]; then
    echo "$HOMEBREW_LIB/$name"
  elif [ -e "$dep" ]; then
    echo "$dep"
  fi
}

bundle_deps() {
  local image="$1" dep name src
  for dep in $(otool -L "$image" | tail -n +2 | awk '{print $1}'); do
    # Intel Homebrew is under /usr/local, so this must stay narrower than /usr/*.
    case "$dep" in /usr/lib/*|/System/*) continue ;; esac
    name="$(basename "$dep")"
    if [ ! -f "$FRAMEWORKS/$name" ]; then
      src="$(lib_source "$dep")"
      if [ -z "$src" ]; then
        echo "::error::$(basename "$image") needs $dep, which was not found"
        exit 1
      fi
      cp -L "$src" "$FRAMEWORKS/$name"
      install_name_tool -id "@rpath/$name" "$FRAMEWORKS/$name"
      bundle_deps "$FRAMEWORKS/$name"
    fi
    case "$dep" in
      @rpath/*) ;;
      *) install_name_tool -change "$dep" "@rpath/$name" "$image" ;;
    esac
  done
}

bundle_deps "$BINARY"
install_name_tool -add_rpath "@executable_path/../Frameworks" "$BINARY" 2>/dev/null || true

# Bundle NDI SDK (runtime-loaded via libloading, universal binary from cask)
NDI_LIB="/Library/NDI SDK for Apple/lib/macOS/libndi.dylib"
if [ -f "$NDI_LIB" ]; then
  echo "==> Bundling NDI SDK..."
  cp "$NDI_LIB" Varda.app/Contents/Frameworks/libndi.dylib
  install_name_tool -id "@rpath/libndi.dylib" Varda.app/Contents/Frameworks/libndi.dylib 2>/dev/null || true
else
  echo "==> NDI SDK not found, skipping bundle (install with: brew install --cask libndi)"
fi

# ONNX Runtime is statically linked into the arm64 binary via ort's
# `download-binaries` feature — no dylib to bundle.

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
  <key>NSAudioCaptureUsageDescription</key><string>Varda records system audio from output devices for audio-reactive visuals</string>
</dict>
</plist>
PLIST_EOF

# --- Code sign ---
echo "==> Ad-hoc code signing..."
codesign --force --deep --sign - Varda.app

"$SCRIPT_DIR/check-macos-bundle.sh" Varda.app

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