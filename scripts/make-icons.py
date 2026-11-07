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


def headphones(d, color, boom=False):
    d.arc((130, 110, 446, 426), 180, 360, fill=color, width=30)
    for x in (120, 372):
        d.rounded_rectangle((x, 260, x + 84, 400), radius=28, fill=color)
    if boom:
        d.arc((150, 250, 330, 470), 90, 180, fill=color, width=20)
        d.ellipse((222, 438, 270, 486), fill=color)


def monitor(d, color):
    d.rounded_rectangle((110, 130, 466, 370), radius=20, outline=color, width=28)
    d.rectangle((268, 370, 308, 430), fill=color)
    d.rounded_rectangle((200, 420, 376, 450), radius=12, fill=color)


def webcam(d, color, bg=BG):
    d.ellipse((168, 100, 408, 340), fill=color)
    d.ellipse((228, 160, 348, 280), fill=bg)
    d.ellipse((258, 190, 318, 250), fill=color)
    d.polygon([(250, 340), (326, 340), (360, 410), (216, 410)], fill=color)
    d.rounded_rectangle((170, 404, 406, 436), radius=12, fill=color)


DEVICES = {
    "speakers": speaker,
    "headphones": headphones,
    "headset": lambda d, c: headphones(d, c, boom=True),
    "monitor": monitor,
    "microphone": mic,
    "webcam": webcam,
}


def save_device(name, img):
    """Device icons are set on the key by the plugin, at 144 px only."""
    out = OUT / "devices"
    out.mkdir(parents=True, exist_ok=True)
    img.resize((144, 144), Image.LANCZOS).save(out / f"{name}.png")


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
    for name, draw in DEVICES.items():
        img, d = canvas(); draw(d, DIM); save_device(f"{name}-off", img)
        img, d = canvas(); draw(d, GREEN); save_device(f"{name}-on", img)


if __name__ == "__main__":
    main()
