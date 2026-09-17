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

# --- Provide what the AppImage contract leaves to the host ---------------------------
# linuxdeploy does not bundle the libraries on the AppImage excludelist, whose header
# states the rule plainly: "libraries that we will assume to be present on the host
# system". That assumption holds for a desktop machine, which is what Varda targets. It
# does not hold for a bare container, which has no desktop at all.
#
# So install the excludelist, rather than a hand-picked guess at which entries Varda
# happens to reach today. A guess is what failed in issue #134, and it fails the same way
# here: the loader stops at the FIRST missing library, so each wrong guess costs a full
# matrix run to reveal exactly one more name.
#
# Package names are best effort, on purpose: a name that is wrong or absent on one
# distribution must not abort the run and hide the rest. Nothing here is trusted. The
# check that follows resolves the real bundle with the real loader and is authoritative.
install_best_effort() {
  local mgr="$1"; shift
  case "$mgr" in
    apt)    apt-get install -y -qq --no-install-recommends "$@" ;;
    dnf)    dnf install -y -q "$@" ;;
    zypper) zypper -q --non-interactive install -y "$@" ;;
    pacman) pacman -S --noconfirm --needed --quiet "$@" ;;
  esac
}

# Install the whole set in one transaction where possible, then fall back to one at a
# time so a single unavailable name costs only itself.
install_set() {
  local mgr="$1"; shift
  if install_best_effort "$mgr" "$@" >/dev/null 2>&1; then
    echo "    installed ${#@} packages"
    return 0
  fi
  echo "    one or more names differ here; installing individually"
  local p
  for p in "$@"; do
    install_best_effort "$mgr" "$p" >/dev/null 2>&1 || echo "      no package '$p' on this distribution"
  done
}

echo "==> Providing the libraries the AppImage leaves to the host"
case "$DISTRO_ID" in
  debian|ubuntu)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    install_set apt \
      libx11-6 libx11-xcb1 libxcb1 libxcb-dri2-0 libxcb-dri3-0 libice6 libsm6 \
      libxext6 libxrandr2 libxi6 libwayland-client0 \
      libgl1 libglx0 libegl1 libopengl0 libglapi-mesa libgbm1 libdrm2 libvulkan1 \
      libasound2t64 libjack-jackd2-0 libpipewire-0.3-0t64 \
      libfontconfig1 libfreetype6 libharfbuzz0b libfribidi0 libexpat1 \
      libgmp10 libgpg-error0 libcom-err2 libuuid1 libusb-1.0-0 zlib1g libstdc++6
    # The t64 transition renamed several of these. Try the pre-transition names too
    # rather than pin one spelling that moves between releases.
    install_set apt libasound2 libpipewire-0.3-0
    ;;
  fedora)
    install_set dnf \
      libX11 libX11-xcb libxcb libICE libSM libXext libXrandr libXi libwayland-client \
      mesa-libGL mesa-libEGL libglvnd-opengl mesa-libglapi mesa-libgbm libdrm vulkan-loader \
      alsa-lib jack-audio-connection-kit pipewire-libs \
      fontconfig freetype harfbuzz fribidi expat \
      gmp libgpg-error libcom_err libuuid libusb1 zlib-ng-compat libstdc++
    ;;
  opensuse*|sles)
    zypper -q --non-interactive --gpg-auto-import-keys refresh
    install_set zypper \
      libX11-6 libX11-xcb1 libxcb1 libICE6 libSM6 libXext6 libXrandr2 libXi6 libwayland-client0 \
      Mesa-libGL1 Mesa-libEGL1 Mesa-libglapi0 libgbm1 libdrm2 libvulkan1 \
      libasound2 libjack0 libpipewire-0_3-0 \
      fontconfig libfreetype6 libharfbuzz0 libfribidi0 libexpat1 \
      libgmp10 libgpg-error0 libcom_err2 libuuid1 libusb-1_0-0 libz1 libstdc++6
    ;;
  arch|cachyos|manjaro|endeavouros)
    # -Syu, not -Sy: a partial upgrade on Arch is how you get a container whose
    # packages disagree with each other, which would look like a Varda bug.
    pacman -Syu --noconfirm --quiet >/dev/null 2>&1 || true
    install_set pacman \
      libx11 libxcb libice libsm libxext libxrandr libxi wayland \
      libglvnd mesa libdrm vulkan-icd-loader \
      alsa-lib jack2 libpipewire \
      fontconfig freetype2 harfbuzz fribidi expat \
      gmp libgpg-error e2fsprogs util-linux-libs libusb zlib gcc-libs
    ;;
  *) echo "::error::no runtime install path for '$DISTRO_ID'"; exit 1 ;;
esac

failed=0

# No FUSE in a container, and that is fine: it is what --appimage-extract-and-run is for,
# and it exercises the same binary and the same bundled libraries. What it cannot test is
# the runtime's own mounting, which is covered by the static runtime being pinned rather
# than by this check.
export APPIMAGE_EXTRACT_AND_RUN=1

# --- Unpack first --------------------------------------------------------------------
# Deliberately before launching. A launch failure names ONE missing library, because the
# loader stops at the first one it cannot resolve; resolving the bundle by hand names all
# of them. Finding them one per matrix run is how days get spent.
echo "==> Unpacking"
WORK="$(mktemp -d)"
( cd "$WORK" && "$PKG" --appimage-extract >/dev/null 2>&1 ) || true
ROOT="$WORK/squashfs-root"
if [ ! -d "$ROOT" ]; then
  echo "FAIL: could not extract the AppImage"
  rm -rf "$WORK"
  echo "==> SMOKE TEST FAILED"
  exit 1
fi

# --- Resolve every bundled object against this system --------------------------------
# ldd walks the whole graph and reports every unresolved name, not just the first. The
# library path mirrors what AppRun sets, so this is the environment the binary really runs
# in. This is the check the package list above is not trusted to satisfy.
echo "==> Resolving the bundle against this system"
unresolved="$(
  { find "$ROOT/usr/bin" "$ROOT/usr/lib" -type f 2>/dev/null || true; } \
    | while read -r f; do
        LD_LIBRARY_PATH="$ROOT/usr/lib" ldd "$f" 2>/dev/null || true
      done \
    | awk '/=> not found/{print $1}' | sort -u
)"
if [ -n "$unresolved" ]; then
  echo "FAIL: this system is missing every library listed here, not just the first"
  printf '%s\n' "$unresolved" | sed 's/^/    /'
  echo "    These are libraries the AppImage leaves to the host. If they are on the"
  echo "    AppImage excludelist, this image needs them added above; the build already"
  echo "    refuses any host dependency that is NOT on that list."
  failed=1
else
  echo "  ok: every bundled object resolves here"
fi

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
#
# grep reads the binary directly. The obvious `strings -a BIN | grep -q` is a coin flip:
# grep -q exits at the first match, strings then dies of SIGPIPE, and pipefail reports
# 141 for a check that succeeded. That is what failed this matrix, claiming a missing
# ONNX Runtime that was present the whole time, and killing the script mid-report.
if [ ! -f "$ROOT/usr/bin/varda" ]; then
  echo "FAIL: $ROOT/usr/bin/varda is missing from the extracted AppImage"
  failed=1
elif grep -qa 'onnxruntime' "$ROOT/usr/bin/varda"; then
  echo "  ok: ONNX Runtime linked (face-detection present)"
else
  echo "FAIL: no ONNX Runtime in the binary; face-detection did not build"
  echo "    binary: $(stat -c %s "$ROOT/usr/bin/varda" 2>/dev/null || echo unknown) bytes"
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

rm -rf "$WORK"

if [ "$failed" -ne 0 ]; then
  echo "==> SMOKE TEST FAILED"
  exit 1
fi
echo "==> SMOKE TEST PASSED"
