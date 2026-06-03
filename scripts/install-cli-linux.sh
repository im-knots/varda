#!/usr/bin/env bash
# Install varda CLI symlink so 'varda' works from any terminal.
#
# Usage: ./scripts/install-cli-linux.sh <path-to-Varda.AppImage> [install-dir]
#
# Default install dir: ~/.local/bin
set -euo pipefail

APPIMAGE="${1:?Usage: install-cli-linux.sh <path-to-Varda.AppImage> [install-dir]}"
APPIMAGE="$(realpath "$APPIMAGE")"
INSTALL_DIR="${2:-$HOME/.local/bin}"

if [ ! -f "$APPIMAGE" ]; then
  echo "Error: $APPIMAGE not found."
  exit 1
fi

if [ ! -x "$APPIMAGE" ]; then
  echo "Making AppImage executable..."
  chmod +x "$APPIMAGE"
fi

mkdir -p "$INSTALL_DIR"
ln -sf "$APPIMAGE" "$INSTALL_DIR/varda"

echo "Installed: $INSTALL_DIR/varda → $APPIMAGE"
echo ""

# Check if install dir is in PATH
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo "NOTE: $INSTALL_DIR is not in your PATH."
  echo "Add it to your shell profile:"
  echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc"
  echo ""
fi

echo "Usage examples:"
echo "  varda                              # Launch UI (default workspace: ~/.varda/)"
echo "  varda --headless --port 9090       # Headless API mode"
echo "  cd ~/shows/festival && varda       # Use project workspace"
echo "  varda --workspace /shows/gig       # Explicit workspace"
