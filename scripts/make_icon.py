"""Build Flex logo icons for Tauri / Windows.

The app icon (also the tray icon) is a dark tile with only its corners rounded
and the white Flex mark scaled up to nearly fill it.
"""
from __future__ import annotations

import struct
from pathlib import Path

from PIL import Image, ImageDraw, ImageOps

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "app-icon-source.png"
ICONS = ROOT / "src-tauri" / "icons"
OUT_ICO = ICONS / "icon.ico"
APP_ICON = ROOT / "app-icon.png"
# The first entry (16) becomes Tauri's default window icon and the tray icon.
SIZES = [16, 32, 48, 256]

TILE = (17, 17, 17, 255)
WHITE = (255, 255, 255, 255)
# Corner radius relative to the tile side (same as the usage widgets).
CORNER_RATIO = 0.18
# Longer side of the mark relative to the tile side.
MARK_RATIO = 0.8
# Draw this many times larger, then downsample for anti-aliased edges.
SUPERSAMPLE = 8


def load_logo() -> Image.Image:
    im = Image.open(SRC)
    if im.mode != "RGBA":
        im = im.convert("RGBA")
    # Make near-white background transparent; keep dark circles.
    px = im.load()
    w, h = im.size
    for y in range(h):
        for x in range(w):
            r, g, b, a = px[x, y]
            # Source is grayscale logo on white — whiten → transparent
            if r > 240 and g > 240 and b > 240:
                px[x, y] = (0, 0, 0, 0)
            else:
                # Force solid black marks
                px[x, y] = (0, 0, 0, 255)
    return im


def pad_to_square(im: Image.Image, pad_ratio: float = 0.14) -> Image.Image:
    w, h = im.size
    side = max(w, h)
    pad = int(side * pad_ratio)
    canvas = Image.new("RGBA", (side + pad * 2, side + pad * 2), (0, 0, 0, 0))
    canvas.paste(im, ((canvas.size[0] - w) // 2, (canvas.size[1] - h) // 2), im)
    return canvas


def mark_coverage() -> Image.Image:
    """Coverage of the mark (L, 255 = mark), cropped to the mark itself."""
    # Source is an anti-aliased dark mark on white.
    cover = ImageOps.invert(Image.open(SRC).convert("L"))
    return cover.crop(cover.point(lambda v: 255 if v > 16 else 0).getbbox())


def render(size: int, mark: Image.Image) -> Image.Image:
    """Dark rounded tile + white mark — readable on light and dark taskbars."""
    k = SUPERSAMPLE
    side = size * k
    big = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    ImageDraw.Draw(big).rounded_rectangle(
        [0, 0, side - 1, side - 1], radius=round(side * CORNER_RATIO), fill=TILE
    )
    scale = side * MARK_RATIO / max(mark.size)
    mw, mh = round(mark.width * scale), round(mark.height * scale)
    # Upscale the coverage and threshold it again so the circles stay round and
    # crisp; the final downsample brings the anti-aliasing back.
    shape = mark.resize((mw, mh), Image.Resampling.BICUBIC).point(lambda v: 255 if v >= 128 else 0)
    big.paste(Image.new("RGBA", (mw, mh), WHITE), ((side - mw) // 2, (side - mh) // 2), shape)
    return big.resize((size, size), Image.Resampling.LANCZOS)


def to_bmp_dib(im: Image.Image) -> bytes:
    im = im.convert("RGBA")
    w, h = im.size
    pixels = im.split()
    r, g, b, a = [ch.tobytes() for ch in pixels]
    bgra = bytearray(w * h * 4)
    for i in range(w * h):
        bgra[i * 4 + 0] = b[i]
        bgra[i * 4 + 1] = g[i]
        bgra[i * 4 + 2] = r[i]
        bgra[i * 4 + 3] = a[i]

    stride = w * 4
    xor = bytearray(stride * h)
    for row in range(h):
        src_off = row * stride
        dst_off = (h - 1 - row) * stride
        xor[dst_off : dst_off + stride] = bgra[src_off : src_off + stride]

    # Pixels outside the rounded corners (alpha 0) are transparent in the AND
    # mask too, so paths that ignore alpha do not fill the corners with black.
    and_row = ((w + 31) // 32) * 4
    and_mask = bytearray(and_row * h)
    for row in range(h):
        dst_off = (h - 1 - row) * and_row
        for col in range(w):
            if a[row * w + col] == 0:
                and_mask[dst_off + col // 8] |= 0x80 >> (col % 8)
    header = struct.pack(
        "<IIIHHIIIIII",
        40,
        w,
        h * 2,
        1,
        32,
        0,
        len(xor) + len(and_mask),
        0,
        0,
        0,
        0,
    )
    return header + bytes(xor) + bytes(and_mask)


def write_ico(frames: list[Image.Image], dest: Path) -> None:
    dibs = [to_bmp_dib(im) for im in frames]
    count = len(frames)
    offset = 6 + 16 * count
    entries = bytearray()
    blob = bytearray()
    for im, dib in zip(frames, dibs):
        w, h = im.size
        entries += struct.pack(
            "<BBBBHHII",
            0 if w >= 256 else w,
            0 if h >= 256 else h,
            0,
            0,
            1,
            32,
            len(dib),
            offset + len(blob),
        )
        blob += dib
    dest.write_bytes(struct.pack("<HHH", 0, 1, count) + entries + blob)


def main() -> None:
    ICONS.mkdir(parents=True, exist_ok=True)
    logo = pad_to_square(load_logo())
    logo.save(APP_ICON)

    mark = mark_coverage()
    frames: list[Image.Image] = []
    for s in SIZES:
        im = render(s, mark)
        frames.append(im)
        if s == 32:
            im.save(ICONS / "32x32.png")
        if s == 256:
            render(128, mark).save(ICONS / "128x128.png")
            im.save(ICONS / "128x128@2x.png")
            im.save(ICONS / "icon.png")
            # Keep a transparent mark variant for docs/UI if needed
            logo.resize((256, 256), Image.Resampling.LANCZOS).save(ICONS / "logo-mark.png")

    write_ico(frames, OUT_ICO)
    print(f"wrote {OUT_ICO} bytes={OUT_ICO.stat().st_size} sizes={SIZES}")


if __name__ == "__main__":
    main()
