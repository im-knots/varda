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
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-deps)    SKIP_DEPS=true ;;
    --skip-build)   SKIP_BUILD=true ;;
    --native-only)  NATIVE_ONLY=true ;;
    --tag=*)        TAG="${1#--tag=}" ;;
    --tag)          shift; TAG="$1" ;;
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
  echo "==> Building $NATIVE_TARGET (native)..."
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

# Bundle FFmpeg dylibs and fix paths
HOMEBREW_LIB="$(brew --prefix)/lib"
for lib in libavcodec libavformat libavutil libswscale libswresample libavdevice libsrt; do
  dylib=$(find "$HOMEBREW_LIB" -maxdepth 1 -name "${lib}*.dylib" -not -name "*_*" | head -1)
  if [ -n "$dylib" ]; then
    cp "$dylib" Varda.app/Contents/Frameworks/
    basename=$(basename "$dylib")
    install_name_tool -change "$dylib" "@executable_path/../Frameworks/$basename" Varda.app/Contents/MacOS/varda 2>/dev/null || true
  fi
done

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
echo "To add 'varda' to your PATH after installing Varda.app:"
echo "  ./scripts/install-cli-macos.sh"