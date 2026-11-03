#!/usr/bin/env python3
"""Preserve k6x8's actual BDF bitmap pixels in a compact Unicode lookup table."""
from pathlib import Path
import gzip
import re
import struct

root = Path(__file__).resolve().parents[1]
source = root / "assets/fonts/k6x8/k6x8_gothic.bdf.gz"
text = gzip.decompress(source.read_bytes()).decode("utf-8")
ascent = int(re.search(r"^FONT_ASCENT (\d+)$", text, re.M)[1])
glyphs = {}
for block in re.findall(r"STARTCHAR .*?\n(.*?)ENDCHAR", text, re.S):
    codepoint = int(re.search(r"^ENCODING (-?\d+)$", block, re.M)[1])
    if codepoint < 0:
        continue
    width, height, xoffset, yoffset = map(int, re.search(r"^BBX (.*)$", block, re.M)[1].split())
    bitmap = re.search(r"^BITMAP\n(.*)", block, re.M | re.S)[1].split()
    assert len(bitmap) == height
    rows = [0] * 8
    for y, hexrow in enumerate(bitmap):
        value = int(hexrow, 16)
        for x in range(width):
            if value & (1 << (len(hexrow) * 4 - 1 - x)):
                dx, dy = xoffset + x, ascent - height - yoffset + y
                assert 0 <= dx < 6 and 0 <= dy < 8, (codepoint, dx, dy)
                rows[dy] |= 0x80 >> dx
    glyphs[codepoint] = bytes(rows)
assert len(glyphs) == int(re.search(r"^CHARS (\d+)$", text, re.M)[1])
assert all(ord(c) in glyphs and any(glyphs[ord(c)]) for c in "まとう")
output = bytearray(b"KF68" + struct.pack("<I", len(glyphs)))
for codepoint, rows in sorted(glyphs.items()):
    output += struct.pack("<I", codepoint) + rows
destination = source.with_name("k6x8-gothic.bin")
if not destination.exists() or destination.read_bytes() != output:
    destination.write_bytes(output)
print(f"Packed {len(glyphs)} exact 6x8 glyphs ({len(output)} bytes)")
