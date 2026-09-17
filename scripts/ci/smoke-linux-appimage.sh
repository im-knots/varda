#!/usr/bin/env bash
# Verify a Varda AppImage runs on this distribution.
# Usage: ./scripts/ci/smoke-linux-appimage.sh <path-to-AppImage>
#
# Runs INSIDE a clean distribution container. Unlike the package smoke there is no install
# step, which is the point: the claim under test is that this file runs on a machine nobody
# prepared. See spec/release-strategy.md section 12.
#
# Every row of the matrix carries real information for this artifact, because an AppImage
# links the host's glibc, GL, X11 and audio stack. That is not true of the Flatpak, whose
# runtime is identical everywhere.
set -euo pipefail

PKG="${1:?Usage: smoke-linux-appimage.sh <path-to-AppImage>}"
PKG="$(cd "$(dirname "$PKG")" && pwd)/$(basename "$PKG")"
chmod +x "$PKG"

os_release_field() {
  ( . /etc/os-release && eval "printf '%s' \"\${$1-}\"" )
}
DISTRO_ID="$(os_release_field ID)"; DISTRO_ID="${DISTRO_ID:-unknown}"

echo "==> $(os_release_field PRETTY_NAME)"
echo "    glibc: $(getconf GNU_LIBC_VERSION 2>/dev/null || echo unknown)"
echo "    file:  $(basename "$PKG")"

# The host libraries the AppImage deliberately does not bundle, because they are coupled to
# the host's drivers, display server or sound system. A venue machine running a desktop has
# these; a bare container does not, so install them the way a user never has to.
echo "==> Installing the desktop runtime this distribution would already have"
case "$DISTRO_ID" in
  debian|ubuntu)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends \
      libx11-6 libxext6 libxrandr2 libxi6 libxcb1 libgl1 libdrm2 \
      libasound2t64 libfontconfig1 libfreetype6 libvulkan1 zlib1g binutils >/dev/null
    # The t64 transition renamed this; try both rather than pin a name that moves.
    apt-get install -y -qq --no-install-recommends libpipewire-0.3-0t64 >/dev/null 2>&1 \
      || apt-get install -y -qq --no-install-recommends libpipewire-0.3-0 >/dev/null
    ;;
  fedora)
    dnf install -y -q \
      libX11 libXext libXrandr libXi libxcb mesa-libGL libdrm \
      alsa-lib fontconfig freetype vulkan-loader zlib-ng-compat binutils \
      pipewire-libs >/dev/null
    ;;
  opensuse*|sles)
    zypper -q --non-interactive --gpg-auto-import-keys refresh
    zypper -q --non-interactive install -y \
      libX11-6 libXext6 libXrandr2 libXi6 libxcb1 Mesa-libGL1 libdrm2 \
      libasound2 fontconfig libfreetype6 libvulkan1 binutils \
      libpipewire-0_3-0 >/dev/null
    ;;
  arch|cachyos|manjaro|endeavouros)
    pacman -Syu --noconfirm --needed --quiet \
      libx11 libxext libxrandr libxi libxcb libglvnd libdrm \
      alsa-lib fontconfig freetype2 vulkan-icd-loader zlib binutils \
      libpipewire >/dev/null
    ;;
  *) echo "::error::no runtime install path for '$DISTRO_ID'"; exit 1 ;;
esac

failed=0

# No FUSE in a container, and that is fine: it is what --appimage-extract-and-run is for,
# and it exercises the same binary and the same bundled libraries. What it cannot test is
# the runtime's own mounting, which is covered by the static runtime being pinned rather
# than by this check.
export APPIMAGE_EXTRACT_AND_RUN=1

echo "==> Launching"
# --version is handled by clap and returns before any GPU, window or audio initialization,
# so this needs no display and no GPU. It does exercise the full loader run across every
# bundled library against this distribution's glibc.
if out="$("$PKG" --version 2>&1)"; then
  echo "  ok: $out"
else
  echo "FAIL: the AppImage did not run"
  echo "$out" | sed 's/^/    /'
  failed=1
fi

echo "==> Checking what it carries"
WORK="$(mktemp -d)"
( cd "$WORK" && "$PKG" --appimage-extract >/dev/null 2>&1 ) || true
ROOT="$WORK/squashfs-root"

if [ ! -d "$ROOT" ]; then
  echo "FAIL: could not extract the AppImage"
  failed=1
else
  # Dependencies with no DT_NEEDED entry, which no closure tool can find and which fail
  # late and confusingly: Varda starts, then cannot record or cannot render.
  if [ -x "$ROOT/usr/bin/ffmpeg" ]; then
    echo "  ok: bundled ffmpeg present"
  else
    echo "FAIL: no bundled ffmpeg; SRT and recording output would fail at showtime"
    failed=1
  fi

  shopt -s nullglob
  shaders=("$ROOT/usr/share/varda/shaders"/*.fs "$ROOT/usr/share/varda/shaders"/*/*.fs)
  shopt -u nullglob
  if [ "${#shaders[@]}" -ge 100 ]; then
    echo "  ok: ${#shaders[@]} shaders bundled"
  else
    echo "FAIL: ${#shaders[@]} shaders bundled, expected the full library"
    failed=1
  fi

  # Statically linked, so its absence is invisible from the outside: Varda starts and the
  # analyzer is simply missing. Every Linux artifact ships the same feature set.
  if ! command -v strings >/dev/null 2>&1; then
    echo "  skip: no strings here, cannot check for ONNX Runtime"
  elif [ ! -f "$ROOT/usr/bin/varda" ]; then
    echo "FAIL: $ROOT/usr/bin/varda is missing from the extracted AppImage"
    failed=1
  elif strings -a "$ROOT/usr/bin/varda" | grep -q 'onnxruntime'; then
    echo "  ok: ONNX Runtime linked (face-detection present)"
  else
    echo "FAIL: no ONNX Runtime in the binary; face-detection did not build"
    echo "    binary: $(ls -l "$ROOT/usr/bin/varda" | awk '{print $5}') bytes"
    echo "    a few strings found, for comparison:"
    strings -a "$ROOT/usr/bin/varda" | grep -iE 'shaderc|wgpu|varda' | head -3 | sed 's/^/      /'
    failed=1
  fi

  # Must come from the host: a bundled loader cannot see the host's ICDs, so Varda would
  # start and then find no GPU.
  if ls "$ROOT/usr/lib"/libvulkan.so* >/dev/null 2>&1; then
    echo "FAIL: the Vulkan loader is bundled; it must come from the host"
    failed=1
  else
    echo "  ok: Vulkan loader left to the host"
  fi
fi
rm -rf "$WORK"

if [ "$failed" -ne 0 ]; then
  echo "==> SMOKE TEST FAILED"
  exit 1
fi
echo "==> SMOKE TEST PASSED"
