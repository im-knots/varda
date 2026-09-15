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

. /etc/os-release
echo "==> ${PRETTY_NAME:-${NAME:-unknown}}"
echo "    glibc: $(getconf GNU_LIBC_VERSION 2>/dev/null || echo unknown)"
echo "    package: $(basename "$PKG")"

echo "==> Installing"
case "${ID:-unknown}" in
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
    su builder -c 'cd /tmp/build && makepkg -si --noconfirm'
    ;;
  *)
    echo "::error::no install path for '${ID:-unknown}'"
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

# ffmpeg is a declared dependency that no SONAME names. If the declaration is missing,
# Varda starts fine and then fails at showtime when someone hits record.
if command -v ffmpeg >/dev/null 2>&1; then
  echo "  ok: ffmpeg present ($(ffmpeg -version 2>/dev/null | head -1))"
else
  echo "FAIL: ffmpeg not installed; the package did not declare it"
  failed=1
fi

echo "==> Checking the shader library installed"
# /usr/bin/varda resolves shaders at ../share/varda/shaders. If packaging and
# BUNDLED_SHADERS_RELATIVE drift apart, Varda starts with an empty shader library and
# nothing else here would notice.
count="$(find /usr/share/varda/shaders -name '*.fs' 2>/dev/null | wc -l)"
if [ "$count" -lt 100 ]; then
  echo "FAIL: found $count bundled shaders under /usr/share/varda/shaders, expected the full library"
  failed=1
else
  echo "  ok: $count shaders installed"
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
