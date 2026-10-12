"""Draws assets/icon.ico: a folder with a check mark.

Pure Python (no libraries). Sizes 16-64 are stored as plain 32-bit bitmaps
(the app reads one of them for its window icon), 256 as PNG.
Run: python3 tools/make_icon.py
"""
import struct
import zlib
from pathlib import Path

SS = 4  # supersampling per axis, for smooth edges

FOLDER = (244, 180, 60)
TAB = (214, 150, 40)
BACK = (226, 160, 45)
TICK = (255, 255, 255)
GREEN = (46, 160, 90)


def seg_dist(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    cx, cy = ax + t * dx, ay + t * dy
    return ((px - cx) ** 2 + (py - cy) ** 2) ** 0.5


def in_round_rect(x, y, x0, y0, x1, y1, r):
    if not (x0 <= x <= x1 and y0 <= y <= y1):
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def shade(u, v):
    """Colour at (u, v) in a 0..1 square, or None for transparent."""
    c = None
    if in_round_rect(u, v, 0.06, 0.14, 0.46, 0.34, 0.05):
        c = TAB
    if in_round_rect(u, v, 0.06, 0.22, 0.94, 0.86, 0.06):
        c = BACK
    if in_round_rect(u, v, 0.06, 0.30, 0.94, 0.88, 0.06):
        c = FOLDER
    # green badge with a white tick, bottom right
    bx, by, br = 0.70, 0.70, 0.25
    d = ((u - bx) ** 2 + (v - by) ** 2) ** 0.5
    if d <= br + 0.035:
        c = (255, 255, 255)
    if d <= br:
        c = GREEN
        t = min(seg_dist(u, v, 0.585, 0.705, 0.665, 0.785), seg_dist(u, v, 0.665, 0.785, 0.82, 0.615))
        if t <= 0.045:
            c = TICK
    return c


def render(n):
    px = []
    for y in range(n):
        row = []
        for x in range(n):
            r = g = b = a = 0
            for sy in range(SS):
                for sx in range(SS):
                    c = shade((x + (sx + 0.5) / SS) / n, (y + (sy + 0.5) / SS) / n)
                    if c:
                        r += c[0]
                        g += c[1]
                        b += c[2]
                        a += 1
            if a:
                row.append((r // a, g // a, b // a, 255 * a // (SS * SS)))
            else:
                row.append((0, 0, 0, 0))
        px.append(row)
    return px


def bmp_entry(px):
    n = len(px)
    header = struct.pack("<IiiHHIIiiII", 40, n, n * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    data = bytearray()
    for row in reversed(px):  # bottom-up
        for r, g, b, a in row:
            data += bytes((b, g, r, a))
    mask_row = ((n + 31) // 32) * 4
    data += bytes(mask_row * n)  # AND mask unused with 32-bit alpha
    return header + bytes(data)


def png_entry(px):
    n = len(px)
    raw = b"".join(b"\x00" + bytes(v for p in row for v in p) for row in px)

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def main():
    sizes = [16, 24, 32, 48, 64, 256]
    images = [(n, png_entry(render(n)) if n == 256 else bmp_entry(render(n))) for n in sizes]
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for n, data in images:
        out += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    for _, data in images:
        out += data
    path = Path(__file__).resolve().parent.parent / "assets" / "icon.ico"
    path.write_bytes(out)
    print(f"wrote {path} ({len(out)} bytes)")


if __name__ == "__main__":
    main()
