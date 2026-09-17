#!/usr/bin/env bash
# Build and install a package from the AUR.
# Usage: ./scripts/ci/install-aur-package.sh <pkgname>
#
# Some libraries Varda uses are not in Arch's official repositories. libfreenect (Kinect
# v1 depth sensors) is the case today. Varda supports every feature it can on every
# platform it can, so the dependency living in a different repository is not a reason to
# ship Arch users a build without depth sensors.
#
# For CI containers. makepkg refuses to run as root, so this creates an unprivileged
# build user; on a developer's own machine use an AUR helper (yay, paru) instead.
set -euo pipefail

PKG="${1:?Usage: install-aur-package.sh <pkgname>}"

if [ "$(id -u)" -ne 0 ]; then
  echo "::error::install-aur-package.sh needs root (it creates a build user and installs via pacman)."
  echo "          On your own machine use an AUR helper: yay -S $PKG"
  exit 1
fi

if pacman -Qi "$PKG" >/dev/null 2>&1; then
  echo "==> $PKG is already installed"
  exit 0
fi

echo "==> Building $PKG from the AUR"
pacman -Sy --noconfirm --needed base-devel git sudo

BUILDER=aurbuild
id "$BUILDER" >/dev/null 2>&1 || useradd -m "$BUILDER"
# makepkg -s installs the package's own dependencies through pacman, which needs root.
echo "$BUILDER ALL=(ALL) NOPASSWD: ALL" > "/etc/sudoers.d/$BUILDER"
chmod 440 "/etc/sudoers.d/$BUILDER"

WORK="/tmp/aur-$PKG"
rm -rf "$WORK"
install -d -o "$BUILDER" -m 755 "$WORK"

su "$BUILDER" -c "
  set -e
  git clone --depth 1 'https://aur.archlinux.org/$PKG.git' '$WORK/pkg'
  cd '$WORK/pkg'
  makepkg -si --noconfirm --needed
"

# makepkg can exit 0 having skipped work, so confirm rather than trust it.
pacman -Qi "$PKG" >/dev/null 2>&1 \
  || { echo "::error::$PKG was not installed after makepkg"; exit 1; }
echo "==> $PKG $(pacman -Q "$PKG" | awk '{print $2}') installed"
