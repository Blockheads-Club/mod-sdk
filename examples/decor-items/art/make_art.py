#!/usr/bin/env python3
"""Draws the example icons and block texture (original art, CC0). Needs Pillow: python3 art/make_art.py

Item icons are square pictures (the game's atlas cells are 16 px, 64 px on HD devices; the loader scales them), block
textures are square tiles. Everything here is drawn from shapes, no game assets."""
from PIL import Image, ImageDraw

OUT = "assets"
S = 64


def lamp():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.polygon([(22, 22), (42, 22), (48, 44), (16, 44)], fill=(232, 176, 52, 255), outline=(120, 76, 12, 255))
    d.ellipse([24, 14, 40, 30], fill=(255, 236, 150, 255), outline=(190, 140, 20, 255))
    d.rectangle([29, 44, 35, 54], fill=(120, 76, 12, 255))
    d.rectangle([20, 54, 44, 58], fill=(160, 104, 20, 255))
    return img


def plaque():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([8, 14, 56, 50], radius=6, fill=(196, 150, 84, 255), outline=(96, 60, 24, 255), width=2)
    for y in (24, 32, 40):
        d.line([(16, y), (48, y)], fill=(96, 60, 24, 255), width=2)
    for x, y in ((12, 18), (52, 18), (12, 46), (52, 46)):
        d.ellipse([x - 2, y - 2, x + 2, y + 2], fill=(240, 220, 160, 255))
    return img


def warm_brick():
    img = Image.new("RGBA", (S, S), (150, 70, 40, 255))
    d = ImageDraw.Draw(img)
    for row in range(4):
        y = row * 16
        d.line([(0, y), (S, y)], fill=(70, 30, 20, 255), width=2)
        off = 0 if row % 2 == 0 else 16
        for x in range(off, S + 32, 32):
            d.line([(x, y), (x, y + 16)], fill=(70, 30, 20, 255), width=2)
            d.rectangle([x + 4, y + 4, x + 12, y + 8], fill=(226, 130, 60, 255))
    d.line([(0, S - 1), (S, S - 1)], fill=(70, 30, 20, 255), width=1)
    return img


lamp().save(f"{OUT}/lamp.png")
plaque().save(f"{OUT}/plaque.png")
warm_brick().save(f"{OUT}/warm-brick.png")
