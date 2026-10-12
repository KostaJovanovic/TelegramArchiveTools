"""Draw crates/tt-app/icon/TelegramTools.ico from the design tokens.

    python tools/make_icon.py

**This script is the source; the .ico is its build artefact** — the same
relationship ``tools/gen_jpeg_header.py`` has with
``crates/tgx-media/src/jpeg_header.rs``, which CLAUDE.md describes as
"Regenerate rather than hand-edit". The .ico is committed so that a build never
depends on Pillow being installed, but every change to the mark belongs here.

What is drawn
-------------
**The two marks it replaces, made one**, from the three primitives the design
has: a filled field, hairlines, one red.

* the field is ``bg`` dark, ``#0a0a0a`` — the page, full-bleed, square corners
  (``metrics::RADIUS`` is 0; square corners are the design, not a default);
* a hairline in ``fg`` dark, ``#e8e8e8``, full-bleed across the **upper**
  quarter — the exporter's mark, the rule under the nav bar;
* a second across the **lower** quarter — the analyser's, a baseline;
* three bars stand on the baseline, flush to the left edge and ascending, the
  first two in ``muted`` ``#888888`` and the tallest in ``accent`` ``#ff3347``,
  the one red. **The red one reaches the upper rule**: it is the only element
  that spans both, which is the point of the tool — what is exported is what is
  reported on. The bars stop at 11/16 of the width.

The exporter's icon was the upper rule and a red bar lying on its side; the
analyser's was the baseline and the bars. Stopping short of the right edge is
the Swiss asymmetry both had: a chart that filled the tile would read as a
generic statistics glyph, and this must not look like some other program's.

Why not something else
----------------------
No wordmark, no letterform, no paper-plane, no gradient, no rounded corners, no
colour beyond the palette. None of those exist in ``crates/tgx-ui/src/tokens.rs``.
The mark is drawn from the *dark* palette because a Windows icon sits on
backgrounds it does not control: a near-black tile reads on the light Explorer
list, the dark taskbar and the Alt-Tab strip alike.

Every element is an axis-aligned rectangle on integer pixel bounds, so there is
no antialiasing anywhere in the file and no element can land half on a pixel.

How the small sizes differ
--------------------------
**Each size is drawn, not downscaled.** Resampling a 256px master to 16px turns
a 1px hairline into three rows of grey mush — the same failure
``components::rule``'s doc comment describes for on-screen hairlines. So the
geometry below is a table of integers per size:

* **The hairlines stay one device pixel** at 16 to 64, where Windows draws the
  entry 1:1. They thicken only at 128 (2px) and 256 (3px), sublinearly, because
  the shell resamples the 256 entry for its intermediate views and a single
  pixel does not survive that.
* **Every bar is whole pixels, and the gap between bars is never less than
  one**, or at 16px three bars become one block.

The container
-------------
Entries at 16, 24, 32, 48, 64, 128 and 256 — the sizes Windows actually asks
for. The ICO is assembled by hand rather than through ``Image.save(sizes=...)``,
which resizes one bitmap into every entry and would undo the whole point above.
Sizes up to 128 are stored as 32-bit BGRA DIBs, which every consumer of the
format has understood since Windows 95; only the 256 entry is PNG-compressed,
the one size where that encoding is expected — and the one the window decodes
for its title bar (``window_icon`` in ``main.rs``). Pillow is used solely to
produce those PNG bytes.
"""

from __future__ import annotations

import io
import struct
from pathlib import Path

from PIL import Image

OUT = Path(__file__).resolve().parent.parent / "crates" / "tt-app" / "icon" / "TelegramTools.ico"

# crates/tgx-ui/src/tokens.rs, Palette::dark().
BG = (0x0A, 0x0A, 0x0A)
FG = (0xE8, 0xE8, 0xE8)
MUTED = (0x88, 0x88, 0x88)
ACCENT = (0xFF, 0x33, 0x47)

# size -> (rule_y, base_y, thickness, bar_width, gap, (h1, h2)), in pixels.
# The red bar's height is not in the table: it is whatever reaches from the
# baseline to the underside of the upper rule, so the two cannot drift apart.
# Written out per size rather than computed, so what lands in each entry is
# readable here and a rounding change is a visible diff.
GEOMETRY = {
    16: (4, 12, 1, 3, 1, (3, 5)),
    24: (6, 18, 1, 4, 2, (4, 8)),
    32: (8, 24, 1, 6, 2, (6, 10)),
    48: (12, 36, 1, 9, 3, (9, 15)),
    64: (16, 48, 1, 12, 4, (12, 20)),
    128: (32, 96, 2, 24, 8, (24, 40)),
    256: (64, 192, 3, 48, 16, (48, 80)),
}

# PNG-compress at and above this size; BMP below it. See the module docstring.
PNG_FROM = 256


def heights(size: int) -> tuple[int, int, int]:
    rule_y, base_y, thick, _, _, (h1, h2) = GEOMETRY[size]
    return h1, h2, base_y - (rule_y + thick)


def draw(size: int) -> Image.Image:
    """The mark at one size, in whole pixels, with no resampling."""
    rule_y, base_y, thick, bar_w, gap, _ = GEOMETRY[size]
    image = Image.new("RGBA", (size, size), BG + (0xFF,))
    # `paste` with a colour and a box fills exact integer bounds; `ImageDraw`
    # would be one more place for an off-by-one on the inclusive rectangle.
    image.paste(FG + (0xFF,), (0, rule_y, size, rule_y + thick))
    image.paste(FG + (0xFF,), (0, base_y, size, base_y + thick))
    bars = heights(size)
    for i, height in enumerate(bars):
        left = i * (bar_w + gap)
        colour = ACCENT if i == len(bars) - 1 else MUTED
        image.paste(colour + (0xFF,), (left, base_y - height, left + bar_w, base_y))
    return image


def check() -> None:
    """The geometry's promises, asserted before anything is written."""
    for size, (rule_y, base_y, thick, bar_w, gap, _) in GEOMETRY.items():
        bars = heights(size)
        assert gap >= 1, f"{size}: bars touching read as one block"
        assert list(bars) == sorted(bars) and len(set(bars)) == 3, f"{size}: the bars must ascend"
        assert base_y - bars[-1] == rule_y + thick, f"{size}: the red bar must meet the upper rule"
        assert base_y + thick <= size, f"{size}: the baseline leaves the tile"
        right = 3 * bar_w + 2 * gap
        assert abs(right - size * 11 / 16) <= 1, f"{size}: bars end at {right}"


def as_bmp(image: Image.Image) -> bytes:
    """A 32-bit BGRA DIB with the AND mask an ICO entry still has to carry.

    The header declares twice the real height: the format inherited that from
    BMP, where the XOR bitmap and the 1-bit AND mask are stacked. The mask is
    all zeroes — every pixel opaque, the alpha channel does the work — but it
    must be present and 4-byte row aligned or the shell reads the next entry's
    bytes as image data.
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
