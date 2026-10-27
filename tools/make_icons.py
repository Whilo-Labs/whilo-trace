"""Draws the Whilo Trace icon (a path of hops) and writes the PNG and ICO files Tauri needs.

Usage: python tools/make_icons.py app/icons
Pure standard library, so it runs anywhere.
"""
import math
import struct
import sys
import zlib
from pathlib import Path

BG_TOP = (22, 30, 52)
BG_BOTTOM = (12, 17, 30)
LINE = (79, 140, 255)
NODE = (232, 238, 255)
DEST = (62, 207, 142)

# Hop nodes in unit coordinates, left to right; the last one is the destination.
NODES = [(0.22, 0.70), (0.40, 0.52), (0.58, 0.60), (0.78, 0.32)]


def seg_dist(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def in_round_rect(x, y, r):
    cx = min(max(x, r), 1 - r)
    cy = min(max(y, r), 1 - r)
    return math.hypot(x - cx, y - cy) <= r


def shade(x, y):
    """Colour and alpha of one sample point in unit coordinates."""
    if not in_round_rect(x, y, 0.22):
        return None
    k = y
    col = tuple(BG_TOP[i] * (1 - k) + BG_BOTTOM[i] * k for i in range(3))
    for (ax, ay), (bx, by) in zip(NODES, NODES[1:]):
        if seg_dist(x, y, ax, ay, bx, by) <= 0.035:
            col = LINE
    for i, (nx, ny) in enumerate(NODES):
        last = i == len(NODES) - 1
        if math.hypot(x - nx, y - ny) <= (0.085 if last else 0.06):
            col = DEST if last else NODE
    return col


def render(size, ss=4):
    rows = []
    for py in range(size):
        row = bytearray()
        for px in range(size):
            acc = [0.0, 0.0, 0.0]
            hits = 0
            for sy in range(ss):
                for sx in range(ss):
                    c = shade((px + (sx + 0.5) / ss) / size, (py + (sy + 0.5) / ss) / size)
                    if c is not None:
                        hits += 1
                        for i in range(3):
                            acc[i] += c[i]
            if hits:
                row += bytes(int(acc[i] / hits) for i in range(3)) + bytes([int(255 * hits / (ss * ss))])
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    return rows


def png(size, rows):
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    raw = b"".join(b"\0" + r for r in rows)
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def ico(images):
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for size, data in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    return header + entries + blobs


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "app/icons")
    out.mkdir(parents=True, exist_ok=True)
    pngs = {s: png(s, render(s, 4 if s >= 64 else 6)) for s in (16, 24, 32, 48, 64, 128, 256)}
    (out / "32x32.png").write_bytes(pngs[32])
    (out / "128x128.png").write_bytes(pngs[128])
    (out / "icon.png").write_bytes(pngs[256])
    (out / "icon.ico").write_bytes(ico([(s, pngs[s]) for s in (16, 24, 32, 48, 64, 256)]))
    print("icons written to", out)


if __name__ == "__main__":
    main()
