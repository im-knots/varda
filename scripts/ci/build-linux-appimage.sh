#!/usr/bin/env bash
# Build the Varda Linux AppImage.
# Usage: ./scripts/ci/build-linux-appimage.sh --version <x.y.z> [--outdir dist] [--skip-deps] [--skip-build]
#
# Produces: dist/Varda-<version>-x86_64.AppImage
#
# One executable file, no installation, no root: the artifact for arriving at a venue and
# running Varda on a machine nobody prepared. See spec/release-strategy.md sections 5
# and 13.
#
# The portable tarball this replaces failed because its dependency closure was written by
# hand and was incomplete (issue #134). linuxdeploy computes the closure from DT_NEEDED
# and applies the AppImage project's excludelist, which is the same job done by tooling
# that has been getting it right for years.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

APPID="io.github.im_knots.varda"
VERSION=""
OUTDIR="dist"
SKIP_DEPS=false
SKIP_BUILD=false
while [ $# -gt 0 ]; do
  case "$1" in
    --version)    VERSION="$2"; shift 2 ;;
    --outdir)     OUTDIR="$2"; shift 2 ;;
    --skip-deps)  SKIP_DEPS=true; shift ;;
    --skip-build) SKIP_BUILD=true; shift ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done
if [ -z "$VERSION" ]; then
  VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
fi
VERSION="${VERSION#v}"

TOOLS="${TOOLS_DIR:-/tmp/appimage-tools}"
mkdir -p "$TOOLS" "$OUTDIR"

echo "==> Varda $VERSION AppImage"

# --- Build dependencies -------------------------------------------------------------
# Same list as the native package build (scripts/ci/build-linux-package.sh) and for the
# same reasons; see the comment there about what a bare container lacks.
if [ "$SKIP_DEPS" = false ]; then
  echo "==> Installing build dependencies"
  export DEBIAN_FRONTEND=noninteractive
  sudo apt-get update
  sudo apt-get install -y --no-install-recommends \
    build-essential cmake pkg-config curl ca-certificates git file desktop-file-utils \
    clang libclang-dev libssl-dev python3 \
    libvulkan-dev \
    libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev \
    ffmpeg \
    libsrt-gnutls-dev libasound2-dev libv4l-dev libfreenect-dev \
    libpipewire-0.3-dev \
    libwayland-dev libxkbcommon-dev libx11-dev libxrandr-dev libxi-dev libgtk-3-dev
fi

if [ "$SKIP_BUILD" = false ]; then
  echo "==> Building release binary"
  cargo build --release
fi

echo "==> Checking the build carries every default feature"
# grep reads the binary directly rather than piping `strings` into `grep -q`: that shape
# is a coin flip, because grep -q exits at the first match, strings then dies of SIGPIPE,
# and pipefail reports 141 for a check that actually succeeded.
if grep -qa 'onnxruntime' target/release/varda; then
  echo "  ok: ONNX Runtime linked (face-detection)"
else
  echo "::error::no ONNX Runtime in target/release/varda; face-detection did not build"
  echo "  enabled features come from Cargo.toml's default: face-detection, html, depth, screen-capture"
  exit 1
fi

# --- Tooling ------------------------------------------------------------------------
fetch() {
  local url="$1" dest="$2"
  [ -x "$dest" ] && return 0
  echo "    fetching $(basename "$dest")"
  curl -fsSL "$url" -o "$dest"
  chmod +x "$dest"
}

echo "==> Fetching linuxdeploy"
fetch "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage" \
      "$TOOLS/linuxdeploy"
fetch "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" \
      "$TOOLS/appimagetool"

# The statically linked runtime. Pinned explicitly rather than relying on whatever the
# plugin embeds by default: this artifact's whole value is running on a machine nobody
# prepared, and libfuse2 is not installed by default on Ubuntu 22.04 or later. See
# spec/release-strategy.md section 13, FUSE.
echo "==> Fetching static AppImage runtime"
fetch "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64" \
      "$TOOLS/runtime-x86_64"

# The build agent may itself lack FUSE, which would stop these tools from running. Extract
# and run rather than mount.
export APPIMAGE_EXTRACT_AND_RUN=1

# --- Stage the AppDir ---------------------------------------------------------------
APPDIR="$PWD/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/varda/shaders" "$APPDIR/usr/share/metainfo"

# linuxdeploy matches the icon to the desktop file's Icon= key, so the filename must be
# the app ID rather than icon.png.
cp assets/icon.png "$TOOLS/$APPID.png"

# usr/bin/varda resolves ../share/varda/shaders, which is BUNDLED_SHADERS_RELATIVE in
# src/internal/registry/mod.rs. The FHS layout adopted for the native packages is also the
# AppDir layout, so this needs no code change and is covered by
# bundled_shader_path_matches_what_packaging_installs.
cp -r shaders/. "$APPDIR/usr/share/varda/shaders/"
sed -e "s/@VERSION@/$VERSION/" -e "s/@DATE@/$(date -u +%Y-%m-%d)/" \
  packaging/linux/varda.metainfo.xml > "$APPDIR/usr/share/metainfo/$APPID.metainfo.xml"

# SRT and recording output shell out to ffmpeg (internal/renderer/subprocess.rs). Deployed
# as a second executable so its own dependencies join the same closure.
FFMPEG_BIN="$(command -v ffmpeg)"
test -n "$FFMPEG_BIN" || { echo "::error::no ffmpeg to bundle"; exit 1; }

# NDI is dlopened by bare soname (internal/ndi/sdk.rs), so it is invisible to a DT_NEEDED
# walk and has to be placed by hand. The UNVERSIONED name must exist: that is the string
# the code asks for.
NDI_SDK_DIR=""
for d in "/tmp/NDI SDK for Linux" "$HOME/NDI SDK for Linux"; do
  [ -d "$d" ] && { NDI_SDK_DIR="$d"; break; }
done
if [ -n "$NDI_SDK_DIR" ]; then
  echo "==> Bundling NDI SDK"
  mkdir -p "$APPDIR/usr/lib"
  cp -P "$NDI_SDK_DIR/lib/x86_64-linux-gnu/"libndi.so* "$APPDIR/usr/lib/"
  test -e "$APPDIR/usr/lib/libndi.so" \
    || { echo "::error::unversioned libndi.so missing; NDI would not load"; exit 1; }
else
  echo "==> NDI SDK not found, skipping (NDI features will be unavailable)"
fi

# --- AppRun environment -------------------------------------------------------------
# linuxdeploy "does not change any environment variables such as $PATH", and its generated
# AppRun sources apprun-hooks/*.sh. Using a hook rather than replacing AppRun keeps
# whatever linuxdeploy puts there instead of silently diverging as the tool changes.
mkdir -p "$APPDIR/apprun-hooks"
cat > "$APPDIR/apprun-hooks/varda-env.sh" <<'HOOK_EOF'
# Reach the bundled ffmpeg, which Varda spawns by name for SRT and recording output.
export PATH="${APPDIR}/usr/bin:${PATH}"
# Resolve the bundled NDI SDK, which is dlopened as the bare soname "libndi.so".
export LD_LIBRARY_PATH="${APPDIR}/usr/lib:${LD_LIBRARY_PATH:-}"
HOOK_EOF

# --- Build ---------------------------------------------------------------------------
# --exclude-library libvulkan: wgpu dlopens the Vulkan loader, so nothing links it and
# linuxdeploy cannot bundle it today. Stated anyway, because a future dependency that
# linked it directly would ship a loader that cannot see the host's ICDs, and the AppImage
# excludelist does not cover it the way it covers libGL.
echo "==> Building AppDir"
"$TOOLS/linuxdeploy" \
  --appdir "$APPDIR" \
  --executable target/release/varda \
  --executable "$FFMPEG_BIN" \
  --desktop-file "packaging/linux/$APPID.desktop" \
  --icon-file "$TOOLS/$APPID.png" \
  --exclude-library "libvulkan.so.1"

# The Vulkan loader arrives transitively rather than directly: Ubuntu's ffmpeg links
# libavfilter, which links libplacebo, which links libvulkan. --exclude-library did not
# stop that, so remove it from the staged AppDir before the image is sealed.
#
# It must come from the host. A bundled loader reads the host's ICD manifests but is not
# the host's loader, which is the same reason libGL is on the AppImage excludelist
# ("known to cause issues if it's bundled"). wgpu dlopens libvulkan.so.1 at runtime and
# will find the host's once ours is gone; libplacebo is only reachable through ffmpeg's
# avfilter, which Varda never calls.
echo "==> Removing host-owned libraries pulled in transitively"
for lib in libvulkan.so libGL.so libEGL.so libGLX.so libdrm.so; do
  for f in "$APPDIR/usr/lib/$lib"*; do
    [ -e "$f" ] || continue
    echo "    removing $(basename "$f")"
    rm -f "$f"
  done
done

# --- What this AppImage expects from the host ---------------------------------------
# Every library the bundle needs but does not carry must be one the AppImage project
# says hosts provide, i.e. on its excludelist. Anything else is issue #134 again: a
# dependency nobody bundled and nobody noticed until a user's machine did not have it.
#
# This is a static property of the AppDir, so it is decided here in seconds rather than
# by a distribution matrix half an hour later. The matrix then answers a different
# question: whether real distributions actually have them.
echo "==> Checking what the bundle expects from the host"
EXCLUDELIST="$TOOLS/excludelist"
if [ ! -s "$EXCLUDELIST" ]; then
  curl -fsSL -o "$EXCLUDELIST" \
    https://raw.githubusercontent.com/AppImage/pkg2appimage/master/excludelist || true
fi
if [ ! -s "$EXCLUDELIST" ]; then
  echo "::error::could not fetch the AppImage excludelist; cannot check the closure"
  exit 1
fi

# DT_NEEDED across every ELF in the AppDir, minus what the AppDir carries.
bundled="$({ find "$APPDIR/usr/lib" -mindepth 1 -printf '%f\n' 2>/dev/null || true; } | sort -u)"
needed="$(
  { find "$APPDIR/usr/bin" "$APPDIR/usr/lib" -type f 2>/dev/null || true; } \
    | while read -r f; do objdump -p "$f" 2>/dev/null | awk '/NEEDED/{print $2}'; done \
    | sort -u
)"
from_host="$(comm -23 <(printf '%s\n' "$needed") <(printf '%s\n' "$bundled"))"

# Strip comments and blank lines; entries look like "libfoo.so.1 # justification".
allowed="$(sed -e 's/#.*//' -e 's/[[:space:]]*$//' "$EXCLUDELIST" | grep -v '^$' | sort -u)"

# Removed on purpose above and deliberately NOT on the excludelist: the AppImage project
# has no entry for the Vulkan loader, but bundling it breaks GPU detection for the same
# reason bundling libGL does. Allowed here explicitly so the rule stays visible.
allowed="$(printf '%s\nlibvulkan.so.1\n' "$allowed" | sort -u)"

unlisted="$(comm -23 <(printf '%s\n' "$from_host") <(printf '%s\n' "$allowed"))"
if [ -n "$unlisted" ]; then
  echo "::error::the bundle expects libraries that hosts are not guaranteed to have"
  printf '%s\n' "$unlisted" | sed 's/^/    /'
  echo "    Either bundle them, or add them to the AppImage excludelist upstream."
  exit 1
fi
host_count="$(printf '%s\n' "$from_host" | grep -c . || true)"
echo "  ok: $host_count host libraries, every one on the AppImage excludelist"
printf '%s\n' "$from_host" | sed 's/^/       /'

echo "==> Building AppImage"
# Packaged with appimagetool rather than `linuxdeploy --output appimage`. That form runs
# linuxdeploy a second time, which redeploys dependencies and undid the removal above.
PKG="$OUTDIR/Varda-$VERSION-x86_64.AppImage"
ARCH=x86_64 "$TOOLS/appimagetool" \
  --runtime-file "$TOOLS/runtime-x86_64" \
  "$APPDIR" "$PKG"

# --- Verify -------------------------------------------------------------------------
test -f "$PKG" || { echo "::error::no AppImage produced"; exit 1; }
chmod +x "$PKG"

echo "==> Checking the bundle"
# The failures this catches are silent ones: an AppImage that starts fine and then cannot
# record, cannot render, or has no shaders.
fail=0
"$PKG" --appimage-extract >/dev/null 2>&1
test -x squashfs-root/usr/bin/ffmpeg \
  || { echo "FAIL: ffmpeg not bundled; SRT and recording output would fail at showtime"; fail=1; }
count=$(ls squashfs-root/usr/share/varda/shaders/*.fs 2>/dev/null | wc -l)
[ "$count" -ge 100 ] \
  || { echo "FAIL: $count shaders bundled, expected the full library"; fail=1; }
if ls squashfs-root/usr/lib/libvulkan.so* >/dev/null 2>&1; then
  echo "FAIL: the Vulkan loader was bundled; it must come from the host to see its ICDs"
  fail=1
fi
rm -rf squashfs-root
[ "$fail" -eq 0 ] || exit 1

echo "==> Done"
ls -lh "$PKG"
