#!/usr/bin/env python3
"""Check an NxN grid screencap against the pbo-stream-burst fixture's expected
per-cell colour.

Unlike dominant_pixel.py (one flat colour expected, so a dominant-colour +
distinct-count check is enough), this fixture paints a distinct colour per
cell, so the check has to be per-cell: read one pixel from the centre of each
of GRID x GRID cells and compare it against colourFor(i) from game.js.

Usage: grid_pixel_check.py PNG GRID
Output: one line per mismatch ("cell R,C: got r,g,b,a want r,g,b,a"), then
  "MISMATCHES=<n>" as the last line.
Exit code: 0 if no mismatches, 1 otherwise (also on decode failure).

Standard library only -- same reasoning as dominant_pixel.py: this has to run
wherever the screencap does.
"""
import struct
import sys
import zlib


def unfilter(raw: bytes, width: int, height: int) -> bytearray:
    stride = width * 4
    out = bytearray()
    prev = bytearray(stride)
    o = 0
    for _ in range(height):
        f = raw[o]
        o += 1
        line = bytearray(raw[o : o + stride])
        o += stride
        if f == 1:
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 255
        elif f == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif f == 3:
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                b = prev[i]
                c = prev[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 255
        elif f != 0:
            raise ValueError(f"unknown PNG filter type {f}")
        out += line
        prev = line
    return out


def colour_for(i: int) -> tuple[int, int, int, int]:
    # Must match game.js's colourFor exactly.
    return ((i * 41) % 256, (i * 89) % 256, (i * 157 + 30) % 256, 255)


def main() -> int:
    path, grid = sys.argv[1], int(sys.argv[2])
    data = open(path, "rb").read()
    pos, idat, width, height = 8, b"", 0, 0
    while pos < len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        kind = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            w, h, depth, colour = struct.unpack(">IIBB", chunk[:10])
            width, height = w, h
            if (depth, colour) != (8, 6):
                print(f"expected 8-bit RGBA, got depth={depth} colour={colour}", file=sys.stderr)
                return 1
        elif kind == b"IDAT":
            idat += chunk
    if not idat or not width:
        print("no image data decoded", file=sys.stderr)
        return 1
    pixels = unfilter(zlib.decompress(idat), width, height)
    stride = width * 4

    cw, ch = width // grid, height // grid
    mismatches = 0
    for row in range(grid):
        for col in range(grid):
            i = row * grid + col
            x = col * cw + cw // 2
            y = row * ch + ch // 2
            off = y * stride + x * 4
            got = tuple(pixels[off : off + 4])
            # game.js's gl.viewport/gl.scissor place row 0 at GL's bottom-left
            # origin, which is the *bottom* of the presented frame; the PNG
            # decodes top-to-bottom. So row r in the screencap corresponds to
            # drawn row (grid-1-r), not row r -- verified empirically: every
            # mismatch before this fix was an exact row-mirror swap (r <->
            # grid-1-r, column unchanged), never a wrong column and never a
            # non-mirror value, which a real corrupted upload would not
            # produce.
            want = colour_for((grid - 1 - row) * grid + col)
            if got != want:
                mismatches += 1
                print(f"cell {row},{col} (i={i}): got {got} want {want}")
    print(f"MISMATCHES={mismatches}")
    return 0 if mismatches == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
