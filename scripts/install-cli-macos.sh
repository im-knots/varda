#!/usr/bin/env bash
# Install varda CLI wrapper to /usr/local/bin so 'varda' works from any terminal.
#
# Usage: ./scripts/install-cli-macos.sh [/path/to/Varda.app]
#
# Default: /Applications/Varda.app
set -euo pipefail

APP_PATH="${1:-/Applications/Varda.app}"
BINARY="$APP_PATH/Contents/MacOS/varda"
FRAMEWORKS="$APP_PATH/Contents/Frameworks"
WRAPPER="/usr/local/bin/varda"

if [ ! -f "$BINARY" ]; then
  echo "Error: $BINARY not found."
  echo "Is Varda.app installed? Pass the path as an argument:"
  echo "  ./scripts/install-cli-macos.sh /path/to/Varda.app"
  exit 1
fi

echo "Installing varda CLI wrapper..."
echo "  Binary:  $BINARY"
echo "  Wrapper: $WRAPPER"

sudo tee "$WRAPPER" > /dev/null << WRAPPER_EOF
#!/bin/bash
# Varda CLI wrapper — forwards to the binary inside Varda.app
export DYLD_FALLBACK_LIBRARY_PATH="$FRAMEWORKS:\${DYLD_FALLBACK_LIBRARY_PATH:-}"
exec "$BINARY" "\$@"
WRAPPER_EOF

sudo chmod +x "$WRAPPER"

echo ""
echo "Done. You can now run 'varda' from any terminal."
echo ""
echo "Usage examples:"
echo "  varda                              # Launch UI (default workspace: ~/.varda/)"
echo "  varda --headless --port 9090       # Headless API mode"
echo "  cd ~/shows/festival && varda       # Use project workspace"
echo "  varda --workspace /shows/gig       # Explicit workspace"
