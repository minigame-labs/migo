#!/usr/bin/env python3
"""Print a PNG's dominant sampled pixel and how many distinct colours it sampled.

Output: `r,g,b,a count` — the most common sampled colour, then the number of
distinct sampled colours.

**The count is not decoration.** Reporting only the dominant colour made this an
always-green instrument for one real failure mode: a frame presented through a
*partial* damage region carries the wrong pixels in only part of the surface, and
the stale majority still reads as the expected colour. That was found by a mutant
that walked. The fixtures this serves all clear to one flat colour, so `count == 1`
is exactly the claim "nothing else reached the window".

Used by scripts/verify-bypass-present.sh. Kept to the standard library on purpose:
the gate has to run wherever the player runs, and pulling Pillow in to read a
handful of pixels would make a presentation check depend on a wheel build.

Samples on a 17-pixel lattice rather than every row: the fixtures are flat, so a
lattice separates "presented" from "did not" at a fraction of the cost.
"""
import struct
import sys
import zlib
from collections import Counter


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
        if f == 1:  # Sub
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 255
        elif f == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif f == 3:  # Average
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 255
        elif f == 4:  # Paeth
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


def main() -> int:
    data = open(sys.argv[1], "rb").read()
    pos, idat, width, height = 8, b"", 0, 0
    while pos < len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        kind = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            width, height, depth, colour = struct.unpack(">IIBB", chunk[:10])
            if (depth, colour) != (8, 6):
                raise ValueError(f"expected 8-bit RGBA, got depth={depth} colour={colour}")
        elif kind == b"IDAT":
            idat += chunk
    pixels = unfilter(zlib.decompress(idat), width, height)

    stride = width * 4
    counts: Counter = Counter()
    for y in range(0, height, 17):
        for x in range(0, width, 17):
            i = y * stride + x * 4
            counts[tuple(pixels[i : i + 4])] += 1
    print(",".join(str(c) for c in counts.most_common(1)[0][0]), len(counts))
    return 0


if __name__ == "__main__":
    sys.exit(main())
