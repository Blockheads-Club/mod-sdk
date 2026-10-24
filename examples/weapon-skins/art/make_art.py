#!/usr/bin/env python3
"""Draws the example weapon skins (original art, CC0). Needs Pillow: python3 art/make_art.py

A weapon texture is the whole picture of the held item (the loader maps all of it onto the quad the game draws for
the item), so the picture is a square with a transparent background. The game's own tools are drawn diagonally in
their square (handle in the lower left, working end in the upper right), and so are these: 32x32 pixels.

Two skins: a golden spade and a pickaxe whose head is a flame."""
import math

from PIL import Image

OUT = "assets"
N = 32
S2 = math.sqrt(2.0)
CLEAR = (0, 0, 0, 0)


def axes(x, y):
    """u runs along the tool (lower left to upper right), v across it (positive to the lower right)."""
    a, b = x + 0.5, N - (y + 0.5)
    return (a + b) / S2, (a - b) / S2


def outline(img, colour):
    """A one pixel dark edge around the opaque pixels."""
    src = img.copy()
    for y in range(N):
        for x in range(N):
            if src.getpixel((x, y))[3]:
                continue
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                nx, ny = x + dx, y + dy
                if 0 <= nx < N and 0 <= ny < N and src.getpixel((nx, ny))[3]:
                    img.putpixel((x, y), colour)
                    break


def paint(fn):
    img = Image.new("RGBA", (N, N), CLEAR)
    for y in range(N):
        for x in range(N):
            u, v = axes(x, y)
            c = fn(u, v)
            if c:
                img.putpixel((x, y), c)
    return img


def golden_spade():
    wood = [(122, 78, 38, 255), (150, 100, 52, 255)]

    def px(u, v):
        if 4 <= u < 23 and abs(v) < 1.2:  # handle
            return wood[0] if v > 0 else wood[1]
        if 3 <= u < 6 and abs(v) < 3.6:  # T grip
            return wood[0] if v > 0.3 else wood[1]
        if 21.5 <= u < 24.5 and abs(v) < 2.4:  # collar
            return (176, 118, 20, 255)
        if 24.5 <= u < 39.5:  # blade: wide shoulders, rounded to a point
            t = (u - 24.5) / 15
            half = 6.2 * (1 - t**2.2)
            if abs(v) <= half:
                if abs(v) < 0.55 and u < 35:  # rib
                    return (255, 247, 190, 255)
                shade = (v + half) / (2 * half + 1e-6)
                if shade < 0.22:
                    return (255, 233, 120, 255)
                if shade < 0.62:
                    return (255, 200, 40, 255)
                if shade < 0.86:
                    return (226, 158, 14, 255)
                return (176, 112, 10, 255)
        return None

    img = paint(px)
    for p in ((19, 9), (22, 7)):  # sparkles
        img.putpixel(p, (255, 255, 235, 255))
    outline(img, (84, 48, 8, 255))
    return img


def flame_pickaxe():
    def px(u, v):
        if 4 <= u < 30 and abs(v) < 1.2:  # charred handle with embers
            if (int(u) * 7 + 3) % 11 == 0:
                return (255, 140, 30, 255)
            return (62, 36, 28, 255) if v > 0 else (92, 54, 36, 255)
        reach = abs(v)
        if reach > 10.5:
            return None
        du = u - (28.5 - 0.055 * v * v)  # the head: a curved bar
        thick = 2.6 - 1.6 * (reach / 10.5)
        heat = 1 - reach / 10.5
        if -thick <= du <= thick:
            if abs(du) < thick * 0.35 and heat > 0.35:
                return (255, 248, 190, 255)
            if heat > 0.62:
                return (255, 214, 70, 255)
            if heat > 0.32:
                return (255, 140, 24, 255)
            return (214, 56, 16, 255)
        # flame tongues rising from the head, away from the handle
        tongue = (2.2 + 2.6 * abs(math.sin(v * 1.35)) + 1.6 * max(0.0, 1 - reach / 8)) * (1 - reach / 12) * 0.8
        if thick < du <= thick + tongue:
            t = (du - thick) / max(tongue, 0.1)
            if abs(((v * 1.7) % 3.4) - 1.7) < 1.7 - t * 1.4:
                return (255, 170, 30, 255) if t < 0.35 else (255, 118, 20, 255) if t < 0.7 else (230, 56, 16, 255)
        return None

    img = paint(px)
    outline(img, (70, 14, 8, 255))
    return img


golden_spade().save(f"{OUT}/golden-spade.png")
flame_pickaxe().save(f"{OUT}/flame-pickaxe.png")
