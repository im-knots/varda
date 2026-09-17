#!/usr/bin/env bash
# Install a Varda Flatpak bundle on this distribution and verify it runs.
# Usage: ./scripts/ci/smoke-linux-flatpak.sh <path-to-.flatpak>
#
# Runs INSIDE a clean distribution container. One row per packaging family rather than
# all six: the Flatpak runtime is identical on every host by construction, so extra rows
# re-test the host's own flatpak installation rather than our artifact. Contrast the
# AppImage, where every distribution is a genuinely different test.
# See spec/release-strategy.md section 12.
set -euo pipefail

PKG="${1:?Usage: smoke-linux-flatpak.sh <path-to-.flatpak>}"
PKG="$(cd "$(dirname "$PKG")" && pwd)/$(basename "$PKG")"
APPID="io.github.im_knots.varda"

os_release_field() {
  ( . /etc/os-release && eval "printf '%s' \"\${$1-}\"" )
}
DISTRO_ID="$(os_release_field ID)"; DISTRO_ID="${DISTRO_ID:-unknown}"

echo "==> $(os_release_field PRETTY_NAME)"
echo "    bundle: $(basename "$PKG")"

echo "==> Installing flatpak itself"
case "$DISTRO_ID" in
  debian|ubuntu)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq && apt-get install -y -qq --no-install-recommends flatpak ca-certificates >/dev/null ;;
  fedora)          dnf install -y -q flatpak >/dev/null ;;
  opensuse*|sles)  zypper --non-interactive --gpg-auto-import-keys install -y flatpak >/dev/null ;;
  arch|cachyos|manjaro|endeavouros)
                   pacman -Syu --noconfirm --needed --quiet flatpak >/dev/null ;;
  *) echo "::error::no flatpak install path for '$DISTRO_ID'"; exit 1 ;;
esac

failed=0

# The bundle carries the app but not the runtime, so Flathub has to be reachable. That is
# exactly what a user does, and it is the step that proves the declared runtime version
# actually exists.
echo "==> Adding Flathub"
flatpak remote-add --if-not-exists --user flathub https://dl.flathub.org/repo/flathub.flatpakrepo

echo "==> Installing the bundle"
if flatpak install --user --noninteractive --bundle "$PKG"; then
  echo "  ok: installed"
else
  echo "FAIL: the bundle did not install"
  exit 1
fi

echo "==> Launching"
# --version is handled by clap and returns before any GPU, window or audio init, so this
# needs no display. It does prove the runtime resolved and the app starts inside it.
if out="$(flatpak run --user "$APPID" --version 2>&1)"; then
  echo "  ok: $out"
else
  echo "FAIL: flatpak run exited non-zero"
  echo "$out" | sed 's/^/    /'
  failed=1
fi

echo "==> Checking what it carries"
# Same assertions as the other artifacts. A Varda that starts with an empty shader library
# looks healthy and is useless, and nothing else here would notice.
INSTALL_DIR="$HOME/.local/share/flatpak/app/$APPID/current/active/files"
# Prove the path before asserting anything about its contents. Every check below reads
# from here, and if the layout moves they would all report what they look for as missing:
# no shaders, no ONNX Runtime, no metadata. That is three confident wrong diagnoses for
# one wrong path.
if [ ! -d "$INSTALL_DIR" ]; then
  echo "FAIL: nothing installed at $INSTALL_DIR"
  echo "    the app is installed, so this is the path shape, not the build:"
  ls -d "$HOME/.local/share/flatpak/app/$APPID"/* 2>/dev/null | sed 's/^/      /' \
    || echo "      (no $APPID directory at all)"
  echo "==> SMOKE TEST FAILED"
  exit 1
fi

shopt -s nullglob
shaders=("$INSTALL_DIR/share/varda/shaders"/*.fs "$INSTALL_DIR/share/varda/shaders"/*/*.fs)
shopt -u nullglob
if [ "${#shaders[@]}" -ge 100 ]; then
  echo "  ok: ${#shaders[@]} shaders installed"
else
  echo "FAIL: ${#shaders[@]} shaders installed, expected the full library"
  failed=1
fi

# face-detection links ONNX Runtime statically, so its absence is invisible from the
# outside: Varda starts normally and the analyzer is simply missing. Check the binary
# carries it, the same way the v0.6.0 artifact was checked.
#
# grep reads the binary directly. `strings -a BIN | grep -q` is a coin flip: grep -q exits
# at the first match, strings then dies of SIGPIPE, and pipefail reports 141 for a check
# that succeeded. That shape cost the AppImage matrix a full round of false failures.
if [ ! -f "$INSTALL_DIR/bin/varda" ]; then
  echo "FAIL: no binary at $INSTALL_DIR/bin/varda"
  failed=1
elif grep -qa 'onnxruntime' "$INSTALL_DIR/bin/varda"; then
  echo "  ok: ONNX Runtime linked (face-detection present)"
else
  echo "FAIL: no ONNX Runtime in the binary; face-detection did not build"
  failed=1
fi

if [ -f "$INSTALL_DIR/share/metainfo/$APPID.metainfo.xml" ]; then
  echo "  ok: AppStream metadata present"
else
  echo "FAIL: no AppStream metadata; Flathub requires it"
  failed=1
fi

if [ "$failed" -ne 0 ]; then
  echo "==> SMOKE TEST FAILED"
  exit 1
fi
echo "==> SMOKE TEST PASSED"
