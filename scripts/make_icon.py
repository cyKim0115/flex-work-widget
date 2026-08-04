"""Build Flex logo icons for Tauri / Windows."""
from __future__ import annotations

import struct
from pathlib import Path

from PIL import Image, ImageOps

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "app-icon-source.png"
ICONS = ROOT / "src-tauri" / "icons"
OUT_ICO = ICONS / "icon.ico"
APP_ICON = ROOT / "app-icon.png"
SIZES = [16, 32, 48, 256]


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


def for_tray(im: Image.Image, size: int) -> Image.Image:
    """Dark tile + white logo — readable on light and dark taskbars."""
    logo = im.resize((int(size * 0.72), int(size * 0.72)), Image.Resampling.LANCZOS)
    # invert RGB, keep alpha
    rgb = logo.convert("RGB")
    inv = ImageOps.invert(rgb).convert("RGBA")
    inv.putalpha(logo.getchannel("A"))
    tile = Image.new("RGBA", (size, size), (17, 17, 17, 255))
    x = (size - inv.size[0]) // 2
    y = (size - inv.size[1]) // 2
    tile.alpha_composite(inv, (x, y))
    return tile


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

    and_row = ((w + 31) // 32) * 4
    and_mask = bytes(and_row * h)
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
    return header + bytes(xor) + and_mask


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

    frames: list[Image.Image] = []
    for s in SIZES:
        im = for_tray(logo, s)
        frames.append(im)
        if s == 32:
            im.save(ICONS / "32x32.png")
        if s == 256:
            im.resize((128, 128), Image.Resampling.LANCZOS).save(ICONS / "128x128.png")
            im.save(ICONS / "128x128@2x.png")
            im.save(ICONS / "icon.png")
            # Keep a transparent mark variant for docs/UI if needed
            logo.resize((256, 256), Image.Resampling.LANCZOS).save(ICONS / "logo-mark.png")

    write_ico(frames, OUT_ICO)
    print(f"wrote {OUT_ICO} bytes={OUT_ICO.stat().st_size} sizes={SIZES}")


if __name__ == "__main__":
    main()
