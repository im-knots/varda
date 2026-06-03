#!/usr/bin/env bash
# Build Varda Linux AppImage
# Usage: ./scripts/ci/build-linux.sh [--skip-deps] [--skip-build]
#
# Produces: Varda-x86_64.AppImage in the project root
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

SKIP_DEPS=false
SKIP_BUILD=false
for arg in "$@"; do
  case "$arg" in
    --skip-deps)  SKIP_DEPS=true ;;
    --skip-build) SKIP_BUILD=true ;;
    *) echo "Unknown arg: $arg"; exit 1 ;;
  esac
done

echo "==> Project root: $PROJECT_ROOT"

# --- Install system dependencies ---
if [ "$SKIP_DEPS" = false ]; then
  echo "==> Installing system dependencies..."
  sudo apt-get update
  sudo apt-get install -y \
    build-essential cmake pkg-config \
    libvulkan-dev \
    libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev libavdevice-dev \
    libsrt-gnutls-dev \
    libasound2-dev \
    libv4l-dev \
    libwayland-dev libxkbcommon-dev libx11-dev libxrandr-dev libxi-dev \
    libgtk-3-dev \
    libfuse2
fi

# --- Build release binary ---
if [ "$SKIP_BUILD" = false ]; then
  echo "==> Building release binary..."
  cargo build --release
fi

echo "==> Preparing AppDir..."
rm -rf Varda.AppDir
mkdir -p Varda.AppDir/usr/bin
mkdir -p Varda.AppDir/usr/lib
mkdir -p Varda.AppDir/usr/share/varda/shaders
mkdir -p Varda.AppDir/usr/share/licenses

cp target/release/varda Varda.AppDir/usr/bin/

# Bundle shaders
cp -r shaders/* Varda.AppDir/usr/share/varda/shaders/

# Bundle all non-system shared libs (discovered via ldd)
# System libs we do NOT bundle — the host must provide these:
EXCLUDE_PATTERN="linux-vdso|ld-linux|libc\.so|libm\.so|libdl\.so|librt\.so|libpthread|libstdc\+\+|libgcc_s|libX|libxcb|libxkb|libwayland|libvulkan|libGLX|libGLdispatch|libEGL|libdrm|libgbm|libgio|libglib|libgobject|libgtk|libgdk|libpango|libcairo|libatk|libfontconfig|libfreetype|libdbus|libresolv|libnss|libffi|libz\.so"

echo "==> Bundling shared libraries (ldd-based)..."
ldd Varda.AppDir/usr/bin/varda | grep "=> /" | awk '{print $3}' | sort -u | while read -r lib; do
  libname=$(basename "$lib")
  # Skip system/desktop libs that the host provides
  if echo "$libname" | grep -qE "$EXCLUDE_PATTERN"; then
    continue
  fi
  echo "  bundling: $libname"
  cp -L "$lib" "Varda.AppDir/usr/lib/$libname" 2>/dev/null || true
done
# Also bundle any soname symlinks for the libs we just copied
for lib in Varda.AppDir/usr/lib/*.so.*; do
  libname=$(basename "$lib")
  base=$(echo "$libname" | sed -E 's/\.so\..*//')
  find /usr/lib -name "${base}.so*" -exec cp -P {} Varda.AppDir/usr/lib/ \; 2>/dev/null || true
done

# Bundle NDI SDK (runtime-loaded via libloading)
NDI_SDK_DIR=""
for d in "/tmp/NDI SDK for Linux" "$HOME/NDI SDK for Linux"; do
  if [ -d "$d" ]; then
    NDI_SDK_DIR="$d"
    break
  fi
done
if [ -n "$NDI_SDK_DIR" ]; then
  echo "==> Bundling NDI SDK from: $NDI_SDK_DIR"
  cp -P "$NDI_SDK_DIR/lib/x86_64-linux-gnu/"libndi.so* Varda.AppDir/usr/lib/ 2>/dev/null || true
else
  echo "==> NDI SDK not found, skipping bundle"
fi

# Bundle licenses
cp LICENSE Varda.AppDir/usr/share/licenses/ 2>/dev/null || echo "MIT License" > Varda.AppDir/usr/share/licenses/LICENSE
echo "FFmpeg is licensed under the LGPL v2.1+. See https://ffmpeg.org/legal.html" > Varda.AppDir/usr/share/licenses/FFMPEG-LICENSE

# Desktop entry
cat > Varda.AppDir/varda.desktop << 'DESKTOP_EOF'
[Desktop Entry]
Name=Varda
Exec=varda
Icon=varda
Type=Application
Categories=AudioVideo;Video;
Comment=Live visuals engine
DESKTOP_EOF

# Icon
cp assets/icon.png Varda.AppDir/varda.png

# AppRun
cat > Varda.AppDir/AppRun << 'APPRUN_EOF'
#!/bin/bash
SELF=$(readlink -f "$0")
HERE=${SELF%/*}
export LD_LIBRARY_PATH="${HERE}/usr/lib:${LD_LIBRARY_PATH}"
exec "${HERE}/usr/bin/varda" "$@"
APPRUN_EOF
chmod +x Varda.AppDir/AppRun

# Download appimagetool if not present
if [ ! -f appimagetool-x86_64.AppImage ]; then
  echo "==> Downloading appimagetool..."
  wget -q https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x appimagetool-x86_64.AppImage
fi

echo "==> Building AppImage..."
ARCH=x86_64 ./appimagetool-x86_64.AppImage --no-appstream Varda.AppDir Varda-x86_64.AppImage

echo "==> Done: Varda-x86_64.AppImage"
ls -lh Varda-x86_64.AppImage
echo ""
echo "The 'varda' CLI symlink is auto-installed to ~/.local/bin/ on first launch."
