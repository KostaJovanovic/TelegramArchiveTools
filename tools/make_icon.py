"""Draw crates/tga-app/icon/TelegramAnalyser.ico from the design tokens.

    python tools/make_icon.py

**This script is the source; the .ico is its build artefact.** The .ico is
committed so that a build never depends on Pillow being installed, but every
change to the mark belongs here. It is the exporter's ``tools/make_icon.py``
with a different drawing: the container code is the same, because the two
icons have to be one family and the hard-won parts — drawn per size, BMP below
256, an AND mask that is present and aligned — are not worth learning twice.

What is drawn
-------------
**A chart, made of the three primitives the design has: a filled field, one
hairline, one red.**

* the field is ``bg`` dark, ``#0a0a0a`` — the page, full-bleed, square corners;
* the hairline is ``fg`` dark, ``#e8e8e8``, full-bleed across the **lower**
  quarter — a baseline;
* three bars stand on it, flush to the left edge and ascending, the first two
  in ``muted`` ``#888888`` and the tallest in ``accent`` ``#ff3347``, the one
  red. They stop at 11/16 of the width.

**It is the exporter's mark answered.** That one is a rule across the upper
quarter and a red bar lying on its side; this is a rule across the lower
quarter and a red bar standing up. Side by side on a taskbar they read as a
pair, and at 16px they cannot be mistaken for each other: one is horizontal
and one is vertical. The bars are the report's own subject — every view in it
is counts over time — and the one red is the figure the report is about.

Stopping short of the right edge is the same Swiss asymmetry as the
exporter's: a chart that filled the tile would read as a generic statistics
glyph, and this must not look like some other program's tile.

How the small sizes differ
--------------------------
**Each size is drawn, not downscaled**, for the exporter's reason: resampling
turns a 1px hairline into rows of grey mush. The hairline stays one device
pixel up to 64 and thickens sublinearly at 128 and 256, which Windows
resamples for its intermediate views. Every bar is whole pixels at every size,
and the gap between bars is never less than one, or at 16px three bars become
one block.
"""

from __future__ import annotations

import io
import struct
from pathlib import Path

from PIL import Image

OUT = Path(__file__).resolve().parent.parent / "crates" / "tga-app" / "icon" / "TelegramAnalyser.ico"

# crates/tgx-ui/src/tokens.rs, Palette::dark().
BG = (0x0A, 0x0A, 0x0A)
FG = (0xE8, 0xE8, 0xE8)
MUTED = (0x88, 0x88, 0x88)
ACCENT = (0xFF, 0x33, 0x47)

# size -> (baseline_y, baseline_thickness, bar_width, gap, (h1, h2, h3)), in
# pixels. Written out per size rather than computed, so what lands in each entry
# is readable here and a rounding change is a visible diff. The bars end at
# 3*bar_width + 2*gap, which is 11/16 of the size wherever that is whole.
GEOMETRY = {
    16: (12, 1, 3, 1, (3, 5, 8)),
    24: (18, 1, 4, 2, (4, 8, 12)),
    32: (24, 1, 6, 2, (6, 10, 16)),
    48: (36, 1, 9, 3, (9, 15, 24)),
    64: (48, 1, 12, 4, (12, 20, 32)),
    128: (96, 2, 24, 8, (24, 40, 64)),
    256: (192, 3, 48, 16, (48, 80, 128)),
}

# PNG-compress at and above this size; BMP below it.
PNG_FROM = 256


def draw(size: int) -> Image.Image:
    """The mark at one size, in whole pixels, with no resampling."""
    base_y, base_t, bar_w, gap, heights = GEOMETRY[size]
    image = Image.new("RGBA", (size, size), BG + (0xFF,))
    # `paste` with a colour and a box fills exact integer bounds.
    image.paste(FG + (0xFF,), (0, base_y, size, base_y + base_t))
    for i, height in enumerate(heights):
        left = i * (bar_w + gap)
        colour = ACCENT if i == len(heights) - 1 else MUTED
        image.paste(colour + (0xFF,), (left, base_y - height, left + bar_w, base_y))
    return image


def as_bmp(image: Image.Image) -> bytes:
    """A 32-bit BGRA DIB with the AND mask an ICO entry still has to carry.

    The header declares twice the real height: the XOR bitmap and the 1-bit AND
    mask are stacked, as in BMP. The mask is all zeroes — the alpha channel
    does the work — but it must be present and 4-byte row aligned or the shell
    reads the next entry's bytes as image data.
    """
    size = image.width
    pixels = image.load()
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    xor = bytearray()
    for y in range(size - 1, -1, -1):  # DIBs are bottom-up.
        for x in range(size):
            r, g, b, a = pixels[x, y]
            xor += bytes((b, g, r, a))
    mask_stride = ((size + 31) // 32) * 4
    return header + bytes(xor) + bytes(mask_stride * size)


def as_png(image: Image.Image) -> bytes:
    buffer = io.BytesIO()
    image.save(buffer, format="PNG", optimize=True)
    return buffer.getvalue()


def check() -> None:
    """The geometry's promises, asserted before anything is written."""
    for size, (base_y, base_t, bar_w, gap, heights) in GEOMETRY.items():
        assert gap >= 1, f"{size}: bars touching read as one block"
        assert list(heights) == sorted(heights), f"{size}: the bars must ascend"
        assert base_y - max(heights) >= 0, f"{size}: the red bar leaves the tile"
        assert base_y + base_t <= size, f"{size}: the baseline leaves the tile"
        right = 3 * bar_w + 2 * gap
        assert abs(right - size * 11 / 16) <= 1, f"{size}: bars end at {right}"


def main() -> None:
    check()
    sizes = sorted(GEOMETRY)
    payloads = [as_png(draw(s)) if s >= PNG_FROM else as_bmp(draw(s)) for s in sizes]

    # ICONDIR, then one 16-byte ICONDIRENTRY each, then the payloads.
    offset = 6 + 16 * len(sizes)
    directory = struct.pack("<HHH", 0, 1, len(sizes))
    for size, payload in zip(sizes, payloads):
        # 256 is stored as 0: the field is a single byte and 256 does not fit.
        directory += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(payload), offset)
        offset += len(payload)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(directory + b"".join(payloads))
    print(f"{OUT}  {OUT.stat().st_size:,} bytes")
    for size, payload in zip(sizes, payloads):
        kind = "png" if size >= PNG_FROM else "bmp"
        print(f"  {size:>3}  {kind}  {len(payload):>7,} bytes")


if __name__ == "__main__":
    main()
