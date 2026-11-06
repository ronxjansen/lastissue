"""Placeholder app icon: a 1024x1024 PNG (stdlib only), written to argv[1].

A rounded indigo square with a white "last item" mark: three list bars,
the bottom one highlighted. Replace with a designed icon later.
"""
import struct
import sys
import zlib

N = 1024


def inside_round_rect(x, y, x0, y0, x1, y1, r):
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def pixel(x, y):
    # macOS icon grid: ~100px margin, ~185px corner radius.
    if not inside_round_rect(x, y, 100, 100, 924, 924, 185):
        return (0, 0, 0, 0)
    t = (y - 100) / 824
    bg = (int(88 - 30 * t), int(86 - 40 * t), int(214 - 60 * t), 255)
    bars = [(300, 380, (255, 255, 255, 120)), (472, 552, (255, 255, 255, 120)),
            (644, 724, (255, 255, 255, 255))]
    for y0, y1, c in bars:
        if inside_round_rect(x, y, 280, y0, 744, y1, 40):
            a = c[3] / 255
            return tuple(int(bg[i] * (1 - a) + c[i] * a) for i in range(3)) + (255,)
    return bg


def png(path):
    rows = bytearray()
    for y in range(N):
        rows.append(0)
        for x in range(N):
            rows.extend(pixel(x, y))

    def chunk(kind, data):
        c = kind + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", N, N, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(bytes(rows), 9)))
        f.write(chunk(b"IEND", b""))


if __name__ == "__main__":
    png(sys.argv[1])
