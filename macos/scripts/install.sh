#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
  echo "This script builds the macOS-only app; use the root Cargo project on Linux/Windows." >&2
  exit 1
fi
./scripts/build.sh
DESTINATION="$HOME/Applications/msutop Mac.app"
if pgrep -f '/msutop Mac.app/Contents/MacOS/msutop-mac$' >/dev/null; then
  echo '请先从 msutop Mac 菜单中退出，再运行安装脚本。' >&2
  exit 1
fi
mkdir -p "$HOME/Applications"
ditto --norsrc --noextattr "build/msutop Mac.app" "$DESTINATION"
codesign --verify --deep --strict "$DESTINATION"
echo "Installed: $DESTINATION"
