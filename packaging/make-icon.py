#!/usr/bin/env python3
"""Draws the app icon (a rounded blue square with a white T and a text cursor) as a 256x256 PNG.
Pure Python, no extra packages. Usage: make-icon.py output.png"""
import struct
import sys
import zlib

SIZE = 256
RADIUS = 52

def inside_rounded(x, y):
    cx = min(max(x, RADIUS), SIZE - 1 - RADIUS)
    cy = min(max(y, RADIUS), SIZE - 1 - RADIUS)
    return (x - cx) ** 2 + (y - cy) ** 2 <= RADIUS ** 2

def rect(x, y, x0, y0, x1, y1):
    return x0 <= x < x1 and y0 <= y < y1

rows = []
for y in range(SIZE):
    row = bytearray([0])  # filter type 0
    for x in range(SIZE):
        if not inside_rounded(x, y):
            row += bytes((0, 0, 0, 0))
            continue
        # background: a light vertical gradient
        shade = int(20 * y / SIZE)
        r, g, b = 38 - shade // 2, 99 - shade, 190 - shade
        # the letter T
        if rect(x, y, 64, 62, 192, 94) or rect(x, y, 112, 94, 144, 196):
            r, g, b = 255, 255, 255
        # the text cursor
        if rect(x, y, 168, 150, 180, 202):
            r, g, b = 120, 220, 255
        row += bytes((r, g, b, 255))
    rows.append(bytes(row))

raw = b"".join(rows)

def chunk(tag, data):
    body = tag + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(raw, 9))
png += chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
