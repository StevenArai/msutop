#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
  echo "This script builds the macOS-only app; use the root Cargo project on Linux/Windows." >&2
  exit 1
fi
RUST_ROOT="$HOME/Library/Caches/msutop-mac/toolchain"
if [[ -n "${MSUTOP_RUSTC:-}" ]]; then
  RUSTC_BIN="$MSUTOP_RUSTC"
elif [[ -x "$RUST_ROOT/cargo/bin/rustc" ]]; then
  export CARGO_HOME="$RUST_ROOT/cargo" RUSTUP_HOME="$RUST_ROOT/rustup"
  RUSTC_BIN="$RUST_ROOT/cargo/bin/rustc"
else
  RUSTC_BIN="$(command -v rustc || true)"
fi
if [[ -z "$RUSTC_BIN" ]]; then
  echo 'Requires Rust 1.90+ for the macOS build.' >&2; exit 1
fi
mkdir -p build/rust-font
python3 - <<'PY'
from pathlib import Path
import re
root=Path('..')
rows=re.findall(r'\{((?:0x[0-9A-Fa-f]{2},?\s*){8})\}',(root/'assets/font6x8.h').read_text())
large=[int(v,16) for row in rows for v in re.findall(r'0x[0-9A-Fa-f]{2}',row)]
small=[int(v,16) for line in (root/'assets/font3x5.inc').read_text().splitlines() if line.startswith('0x') for v in re.findall(r'0x[0-9A-Fa-f]+',line)]
assert len(large)==760 and len(small)==95
Path('build/rust-font/font.rs').write_text(f'pub const FONT: [u8; 760] = {large};\npub const FONT_3X5: [u16; 95] = {small};\n')
PY
export OUT_DIR="$PWD/build/rust-font"
"$RUSTC_BIN" --edition=2021 -O --crate-name msutop --crate-type rlib ../src/lib.rs -o build/libmsutop.rlib
"$RUSTC_BIN" --edition=2021 -O --crate-name msutop_mac_renderer --crate-type staticlib \
  --extern msutop=build/libmsutop.rlib Sources/Renderer.rs -o build/libmsutop_mac_renderer.a
if [[ "${1:-}" == --test ]]; then
  "$RUSTC_BIN" --edition=2021 -O --test --extern msutop=build/libmsutop.rlib Sources/Renderer.rs -o build/renderer-tests
  build/renderer-tests
fi
