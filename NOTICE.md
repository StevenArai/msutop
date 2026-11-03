# Licenses and sources

msutop's original Rust implementation, scripts, documentation, and user-supplied
3x5 glyph data are published under the MIT license in `LICENSE`. Third-party
components retain the licenses and notices listed below.

## 6x8 text cells

`assets/font6x8.h` contains the ASCII 32..126 part of Adafruit GFX's classic 5x7
font, converted into eight MSB-first rows in a six-pixel-wide cell. The sixth
column is spacing. The glyphs were converted from column-major data on 2026-11-03;
no Linux kernel font is included in this release.

- Source: https://github.com/adafruit/Adafruit-GFX-Library/blob/6abf6930565018657600aca8a5995bb8a4068339/glcdfont.c
- Copyright (c) 2012 Adafruit Industries. All rights reserved.
- License: BSD-2-Clause; see `licenses/Adafruit-GFX-BSD-2-Clause.txt`.

## 3x5 values

`assets/font3x5.inc` preserves the user-supplied ASCII 32..126 font from the
credits_vfd project, source commit `20d5a8a108df38753b96bc66c7187f5afea3b50e`.
The original file is `src/font_generated.inc`, SHA-256
`1779c1fed58a8212bbfec8decc7e128a6f0fe3d7bfb91e6159fd65af34c7aef9`.
Its row-major, bit-14-first glyphs and user edits are unchanged. The source
project did not declare a separate font license; this user-supplied asset is
included under the project's MIT release terms.

## btop colors

The Default_theme palette values were verified at btop commit
`fd1d2920e5442045f89b8991cca4c3c503ff48f6`. btop is Copyright 2021 Aristocratos
(jakob@qvantnet.com), licensed under Apache-2.0. Source:
https://github.com/aristocratos/btop. The palette constants in `src/theme.rs`
use that source; the vivid CPU stops were user-supplied.
`licenses/btop-Apache-2.0.txt` preserves the upstream license.

## SDL2 and Rust dependencies

SDL2 2.32.10 is distributed unchanged in the Windows package and retains its
zlib license; see `licenses/SDL2-zlib.txt`. The GNU/MinGW build script verifies
the upstream archive's SHA-256 before unpacking it. Rust dependencies retain
their upstream licenses. The release package includes their applicable license
texts under `licenses/dependencies/`.
