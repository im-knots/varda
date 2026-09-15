#!/usr/bin/env bash
# Install a Varda Linux package on this distribution and verify it runs.
# Usage: ./scripts/ci/smoke-linux-package.sh <path-to-package>
#
# Runs INSIDE a clean distribution container (spec/release-strategy.md section 12).
#
# Installing through the distribution's own package manager is the point. It is what
# proves the declared dependencies are correct and complete: if dpkg-shlibdeps or rpm's
# find-requires missed something, or a dependency does not exist under that name here,
# the install fails in CI rather than on a user's machine. Issue #134 shipped because
# nothing ever ran the Linux artifact anywhere but the box that built it.
set -euo pipefail

PKG="${1:?Usage: smoke-linux-package.sh <path-to-package>}"
# Absolute, because a bare relative path is ambiguous to apt: `apt install dist/x.deb`
# is parsed as package `dist` from release `x.deb`, not as a file. Only a path with a
# leading / or ./ is treated as a local package.
PKG="$(cd "$(dirname "$PKG")" && pwd)/$(basename "$PKG")"

# Read fields in a subshell rather than sourcing os-release into this one. It defines
# VERSION, NAME and more, and sourcing a third-party file over this script's own
# variables is how the build script came to emit `Version: 13 (trixie)` into a .deb
# control file.
os_release_field() {
  ( . /etc/os-release && eval "printf '%s' \"\${$1-}\"" )
}
DISTRO_ID="$(os_release_field ID)"
DISTRO_ID="${DISTRO_ID:-unknown}"

echo "==> $(os_release_field PRETTY_NAME)"
echo "    glibc: $(getconf GNU_LIBC_VERSION 2>/dev/null || echo unknown)"
echo "    package: $(basename "$PKG")"

echo "==> Installing"
case "$DISTRO_ID" in
  debian|ubuntu)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    # `apt install ./file.deb` resolves and installs the declared Depends, which is
    # exactly what is under test. `dpkg -i` would skip that and prove nothing.
    apt-get install -y "$PKG"
    ;;
  fedora)
    dnf install -y \
      "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm"
    dnf install -y "$PKG"
    ;;
  opensuse*|sles)
    # Packman, same as the build container. Stock Leap has no ffmpeg, so the package's
    # /usr/bin/ffmpeg dependency is unresolvable without it. A user installing Varda on
    # Leap needs this repository too, which is why the smoke adds it rather than
    # pre-installing ffmpeg: it tests the instructions the README gives.
    LEAP_VER="$(os_release_field VERSION_ID)"
    zypper --non-interactive --gpg-auto-import-keys addrepo -cfp 90 \
      "https://ftp.gwdg.de/pub/linux/misc/packman/suse/openSUSE_Leap_${LEAP_VER}/" packman || true
    zypper --non-interactive --gpg-auto-import-keys refresh
    zypper --non-interactive install --allow-unsigned-rpm "$PKG"
    ;;
  arch|cachyos|manjaro|endeavouros)
    # The Arch artifact is a PKGBUILD, so this builds it the way a user would. makepkg
    # refuses to run as root, hence the unprivileged build user.
    pacman -Syu --noconfirm --needed base-devel sudo git
    # libfreenect is an AUR dependency of the package. An AUR helper (yay, paru) resolves
    # this automatically for a real user; plain makepkg does not, so install it first.
    ./scripts/ci/install-aur-package.sh libfreenect
    useradd -m builder 2>/dev/null || true
    echo 'builder ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/builder
    install -d -o builder /tmp/build
    cp "$PKG" /tmp/build/PKGBUILD
    chown builder /tmp/build/PKGBUILD
    su builder -c 'cd /tmp/build && makepkg -si --noconfirm --nocheck'
    ;;
  *)
    echo "::error::no install path for '$DISTRO_ID'"
    exit 1
    ;;
esac

# --- Verify ---
failed=0

echo "==> Checking dependency resolution"
BIN="$(command -v varda || echo /usr/bin/varda)"
missing="$(ldd "$BIN" 2>/dev/null | awk '/not found/ {print $1}')"
if [ -n "$missing" ]; then
  echo "FAIL: $BIN cannot resolve:"
  echo "$missing" | sed 's/^/    /'
  failed=1
else
  echo "  ok: every DT_NEEDED entry resolves"
fi

# Dependencies with no SONAME reference. dpkg-shlibdeps and rpm find-requires
# structurally cannot derive these, so the package declares them by hand and they have to
# be verified by hand here. Both fail late and confusingly when a declaration is dropped:
# Varda installs, starts, and then cannot record or cannot render.
if command -v ffmpeg >/dev/null 2>&1; then
  echo "  ok: ffmpeg present ($(ffmpeg -version 2>/dev/null | head -1))"
else
  echo "FAIL: ffmpeg not installed; the package did not declare it"
  failed=1
fi

# wgpu dlopens libvulkan.so.1. It never appears in ldd output, and `varda --version`
# returns before any GPU work, so nothing else here would notice it missing.
if ldconfig -p 2>/dev/null | grep -q 'libvulkan\.so\.1'; then
  echo "  ok: Vulkan loader present"
else
  echo "FAIL: libvulkan.so.1 not installed; the package did not declare the Vulkan loader"
  failed=1
fi

echo "==> Checking the shader library installed"
# /usr/bin/varda resolves shaders at ../share/varda/shaders. If packaging and
# BUNDLED_SHADERS_RELATIVE drift apart, Varda starts with an empty shader library and
# nothing else here would notice.
# Guard the directory before counting. Under `set -euo pipefail` a find over a missing
# directory fails the pipeline, which kills this script mid-check with no message at all
# rather than reporting the thing it was looking for.
SHADER_DIR=/usr/share/varda/shaders
if [ ! -d "$SHADER_DIR" ]; then
  echo "FAIL: $SHADER_DIR does not exist; the package did not install the shader library"
  failed=1
else
  count="$(find "$SHADER_DIR" -name '*.fs' | wc -l | tr -d ' ')"
  if [ "${count:-0}" -lt 100 ]; then
    echo "FAIL: found $count bundled shaders under $SHADER_DIR, expected the full library"
    failed=1
  else
    echo "  ok: $count shaders installed"
  fi
fi

echo "==> Launching"
# --version is handled by clap and returns before any GPU, window or audio
# initialization, so this needs no display and no GPU.
if out="$(varda --version 2>&1)"; then
  echo "  ok: $out"
else
  echo "FAIL: varda --version exited non-zero"
  echo "$out" | sed 's/^/    /'
  failed=1
fi

if [ "$failed" -ne 0 ]; then
  echo "==> SMOKE TEST FAILED"
  exit 1
fi
echo "==> SMOKE TEST PASSED"
