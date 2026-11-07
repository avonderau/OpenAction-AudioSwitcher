#!/usr/bin/env python3
"""Draws the key and plugin icons into com.avonderau.audioswitcher.sdPlugin/imgs.

Run from the repository root: python3 scripts/make-icons.py (needs Pillow).
Each image is written at 72 px (name.png) and 144 px (name@2x.png).
"""

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "com.avonderau.audioswitcher.sdPlugin" / "imgs"

BG = (24, 27, 33, 255)
FG = (230, 233, 238, 255)
BLUE = (46, 134, 222, 255)
GREEN = (46, 204, 113, 255)
DIM = (120, 126, 136, 255)

S = 576  # draw big, then downscale for smooth edges


def canvas(bg=BG):
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((0, 0, S - 1, S - 1), radius=72, fill=bg)
    return img, d


def speaker(d, color, waves=True):
    d.polygon([(130, 230), (200, 230), (290, 150), (290, 420), (200, 340), (130, 340)], fill=color)
    if waves:
        for r in (70, 130):
            d.arc((290 - r, 285 - r, 290 + r, 285 + r), -50, 50, fill=color, width=26)


def mic(d, color, cx=S // 2, cy=230):
    w, h = 90, 160
    d.rounded_rectangle((cx - w // 2, cy - h // 2, cx + w // 2, cy + h // 2), radius=w // 2, fill=color)
    r, lw = 95, 22
    d.arc((cx - r, cy - r + 20, cx + r, cy + r + 20), 20, 160, fill=color, width=lw)
    d.line((cx, cy + r + 20, cx, cy + r + 70), fill=color, width=lw)
    d.line((cx - 55, cy + r + 70, cx + 55, cy + r + 70), fill=color, width=lw)


def swap(d, color):
    """Two arrows, for the plugin icon."""
    d.line((150, 200, 400, 200), fill=color, width=34)
    d.polygon([(400, 150), (460, 200), (400, 250)], fill=color)
    d.line((176, 376, 426, 376), fill=color, width=34)
    d.polygon([(176, 326), (116, 376), (176, 426)], fill=color)


def save(name, img):
    OUT.mkdir(parents=True, exist_ok=True)
    img.resize((144, 144), Image.LANCZOS).save(OUT / f"{name}@2x.png")
    img.resize((72, 72), Image.LANCZOS).save(OUT / f"{name}.png")


def main():
    img, d = canvas(); speaker(d, DIM); save("output-off", img)
    img, d = canvas(); speaker(d, GREEN); save("output-on", img)
    img, d = canvas(); mic(d, DIM); save("input-off", img)
    img, d = canvas(); mic(d, GREEN); save("input-on", img)
    img, d = canvas(BLUE); swap(d, FG); save("plugin", img)


if __name__ == "__main__":
    main()
