#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
  echo "This script builds the macOS-only app; use the root Cargo project on Linux/Windows." >&2
  exit 1
fi
RUST_ROOT="$HOME/Library/Caches/msutop-mac/toolchain"
if [[ -x "$RUST_ROOT/cargo/bin/cargo" ]]; then
  export CARGO_HOME="$RUST_ROOT/cargo" RUSTUP_HOME="$RUST_ROOT/rustup"
  CARGO_BIN="$RUST_ROOT/cargo/bin/cargo"
else
  CARGO_BIN="$(command -v cargo)"
fi
export CARGO_TARGET_DIR="$PWD/build/sensor-target"
"$CARGO_BIN" build --release --locked --manifest-path Sensors/Cargo.toml
cp "$CARGO_TARGET_DIR/release/msutop-sensors" build/msutop-sensors
"$CARGO_BIN" metadata --format-version 1 --locked --manifest-path Sensors/Cargo.toml > build/sensor-dependencies.json
python3 - <<'PY'
from pathlib import Path
import json, shutil
root=Path.cwd()
metadata=json.loads((root/'build/sensor-dependencies.json').read_text())
for package in metadata['packages']:
    if not (package.get('source') or '').startswith('registry+'):
        continue
    name=f"{package['name']}-{package['version']}"
    destination=root/'licenses/sensor-dependencies'/name
    for item in Path(package['manifest_path']).parent.iterdir():
        if item.is_file() and item.name.upper().startswith(('LICENSE','COPYING','COPYRIGHT')):
            destination.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(item,destination/item.name)
PY
