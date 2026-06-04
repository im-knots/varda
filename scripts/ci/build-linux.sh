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

# Bundle media shared libs — allowlist approach.
# Ubuntu's system FFmpeg links against dozens of optional deps (Java via libbluray,
# Intel Media SDK, etc.) that we don't use. Instead of trying to exclude all of those,
# we bundle only the libs we know we need.
MEDIA_LIBS="
  libavcodec libavformat libavutil libswscale libswresample libavdevice libavfilter libpostproc
  libsrt libsrt-gnutls
  libx264 libx265 libvpx libopus libvorbis libvorbisenc libogg libmp3lame libfdk-aac
  libass libharfbuzz libfribidi
  libaom libdav1d librav1e libSvtAv1Enc
  libtheoraenc libtheoradec libtheora
  libwebp libwebpmux
  libnuma libgnutls libhogweed libnettle libgmp
  libsoxr libvidstab libzimg librubberband libsamplerate
  libspeex libshine libtwolame libgsm
  liblzma libbz2 libsnappy
  libva libva-drm libva-x11 libva-wayland
  libOpenCL
"

echo "==> Bundling media shared libraries..."
for lib_base in $MEDIA_LIBS; do
  find /usr/lib -name "${lib_base}.so*" -print0 2>/dev/null | while IFS= read -r -d '' lib; do
    libname=$(basename "$lib")
    if [ ! -e "Varda.AppDir/usr/lib/$libname" ]; then
      echo "  bundling: $libname"
      cp -P "$lib" "Varda.AppDir/usr/lib/" 2>/dev/null || true
    fi
  done
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
