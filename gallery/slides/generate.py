#!/usr/bin/env python3
"""Draws the pictures of the slide deck (deterministic; needs numpy and Pillow).

    python3 gallery/slides/generate.py

writes dusk.jpg (a mountain range under a sunset sky), julia.jpg (a Julia set) and mesh.jpg
(a soft mesh of colour) next to this file.
"""
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

HERE = Path(__file__).resolve().parent
rng = np.random.default_rng(7)


def mix(a, b, t):
    t = np.clip(t, 0, 1)[..., None]
    return a * (1 - t) + b * t


def noise1d(n, octaves=6):
    """Fractal noise along a line, between 0 and 1."""
    out = np.zeros(n)
    amp, total = 1.0, 0.0
    for o in range(octaves):
        points = 2 ** (o + 2)
        values = rng.random(points + 1)
        out += amp * np.interp(np.linspace(0, points, n), np.arange(points + 1), values)
        total += amp
        amp *= 0.5
    return out / total


def dusk(width=1920, height=1080):
    y = np.linspace(0, 1, height)[:, None] * np.ones((1, width))
    x = np.linspace(0, 1, width)[None, :] * np.ones((height, 1))
    stops = [
        (0.00, (14, 12, 54)),
        (0.35, (72, 32, 120)),
        (0.55, (214, 74, 120)),
        (0.68, (255, 150, 90)),
        (0.78, (255, 214, 140)),
    ]
    sky = np.zeros((height, width, 3))
    for (p0, c0), (p1, c1) in zip(stops, stops[1:]):
        mask = (y >= p0) & (y < p1)
        t = (y - p0) / (p1 - p0)
        sky[mask] = mix(np.array(c0, float), np.array(c1, float), t)[mask]
    sky[y >= stops[-1][0]] = stops[-1][1]
    # the sun and its glow
    dx, dy = (x - 0.68) * width, (y - 0.70) * height
    distance = np.sqrt(dx**2 + dy**2)
    sky += np.exp(-(distance / 260) ** 2)[..., None] * np.array([255, 190, 120]) * 0.55
    sky = np.where((distance < 70)[..., None], np.array([255, 244, 214]), sky)
    # stars in the upper sky
    stars = rng.random((height, width)) > 0.9993
    stars &= y < 0.45
    sky[stars] = 255
    image = Image.fromarray(np.clip(sky, 0, 255).astype("uint8"))
    image = image.filter(ImageFilter.GaussianBlur(0.8))
    array = np.asarray(image).astype(float)
    # ridges, from far (pale, blue) to near (dark)
    layers = [
        (0.62, 0.10, (120, 70, 140)),
        (0.70, 0.12, (84, 48, 112)),
        (0.78, 0.14, (52, 30, 84)),
        (0.88, 0.16, (24, 16, 48)),
    ]
    columns = np.arange(width)
    for base, amplitude, color in layers:
        ridge = (base - (noise1d(width) - 0.5) * 2 * amplitude) * height
        mask = np.arange(height)[:, None] >= ridge[None, :]
        # mist: the ridge fades into the haze
        shade = np.array(color, float) + 40 * np.exp(-((np.arange(height)[:, None] - ridge[None, :]) / 90) ** 2)[..., None] * 0.0
        depth = np.clip((np.arange(height)[:, None] - ridge[None, :]) / 260, 0, 1)[..., None]
        fill = mix(shade * np.ones((height, width, 1)), np.array(color, float) * 0.55, depth[..., 0])
        array = np.where(mask[..., None], fill, array)
    grain = rng.normal(0, 2.2, array.shape)
    Image.fromarray(np.clip(array + grain, 0, 255).astype("uint8")).save(HERE / "dusk.jpg", quality=88)


def julia(size=(1400, 1400)):
    w, h = size
    re = np.linspace(-1.55, 1.55, w)[None, :] * np.ones((h, 1))
    im = np.linspace(-1.55, 1.55, h)[:, None] * np.ones((1, w))
    z = re + 1j * im
    c = -0.7269 + 0.1889j
    steps = np.zeros(z.shape)
    alive = np.ones(z.shape, bool)
    for i in range(160):
        z = np.where(alive, z * z + c, z)
        escaped = alive & (np.abs(z) > 4)
        steps[escaped] = i + 1 - np.log2(np.log(np.abs(z[escaped]) + 1e-9) / np.log(4))
        alive &= ~escaped
    t = np.where(alive, 0, steps / 160)
    t = np.sqrt(t)
    palette = np.array([(10, 8, 40), (60, 20, 120), (210, 60, 130), (255, 170, 60), (255, 245, 200)], float)
    pos = t * (len(palette) - 1)
    lo = np.clip(pos.astype(int), 0, len(palette) - 2)
    rgb = palette[lo] * (1 - (pos - lo))[..., None] + palette[lo + 1] * (pos - lo)[..., None]
    rgb[alive] = (6, 4, 24)
    Image.fromarray(np.clip(rgb, 0, 255).astype("uint8")).save(HERE / "julia.jpg", quality=90)


def mesh(size=(1600, 900)):
    w, h = size
    x = np.linspace(0, 1, w)[None, :] * np.ones((h, 1))
    y = np.linspace(0, 1, h)[:, None] * np.ones((1, w))
    out = np.zeros((h, w, 3))
    blobs = [
        (0.15, 0.2, (255, 110, 150)),
        (0.85, 0.15, (120, 90, 255)),
        (0.7, 0.85, (40, 200, 190)),
        (0.2, 0.9, (255, 190, 70)),
        (0.5, 0.5, (90, 150, 255)),
    ]
    weight = np.zeros((h, w))
    for bx, by, color in blobs:
        d2 = (x - bx) ** 2 * 1.8 + (y - by) ** 2 * 3.2
        k = np.exp(-d2 * 6.0)
        out += k[..., None] * np.array(color, float)
        weight += k
    out /= np.maximum(weight, 1e-6)[..., None]
    Image.fromarray(np.clip(out, 0, 255).astype("uint8")).save(HERE / "mesh.jpg", quality=90)


if __name__ == "__main__":
    dusk()
    julia()
    mesh()
