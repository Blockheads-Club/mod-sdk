#!/usr/bin/env python3
"""Draws the example textures (original art, CC0). Needs Pillow: python3 art/make_art.py

The game maps these onto the blockhead's head cube, so every texture is a pattern that looks right on any face.
Hat textures are 32x16 (the linen cap) and 32x32 (the santa hat); hair is 32x16. The game multiplies a hat by the
colour of the item it re-skins and hair by the hair colour, so light textures keep their colour best."""
from PIL import Image

OUT = "assets"


def save(img, name):
    img.save(f"{OUT}/{name}.png")


def crown():
    img = Image.new("RGBA", (32, 16))
    for y in range(16):
        for x in range(32):
            band = (y // 4) % 2 == 0
            gold = (255, 214, 64, 255) if band else (255, 236, 130, 255)
            if x % 8 in (3, 4) and y % 8 in (3, 4):
                gold = (214, 40, 60, 255)  # a ruby on every face
            if y in (0, 15) or x % 8 == 0:
                gold = (190, 140, 20, 255)  # rim and seams
            img.putpixel((x, y), gold)
    return img


def party():
    img = Image.new("RGBA", (32, 32))
    colours = [(255, 120, 200, 255), (255, 255, 255, 255), (120, 200, 255, 255)]
    for y in range(32):
        for x in range(32):
            c = colours[((x + y) // 4) % 3]
            if (x * 7 + y * 3) % 23 == 0:
                c = (255, 230, 90, 255)  # confetti
            img.putpixel((x, y), c)
    return img


def ember():
    img = Image.new("RGBA", (32, 16))
    for y in range(16):
        for x in range(32):
            heat = (x * 5 + y * 11) % 7
            img.putpixel((x, y), (255, 150 + heat * 14, 40 + heat * 20, 255))
    return img


save(crown(), "crown")
save(party(), "party")
save(ember(), "ember")
