"""Builds the COLR test fonts the closure capture draws (render-cases.ts
`--set closure`): Georgia with a `COLR` v0 table (each letter two layers:
itself in a palette red, then `o` in a half-transparent blue) and with a
`COLR` v1 table (each letter a linear gradient). Derived from a system font,
so the files stay in the private corpus directory, never in the repository.

    python3 engine/crates/pm-text/capture/colr-fonts.py <out-dir>

Needs fontTools (4.40 or later)."""
import os
import sys

from fontTools.colorLib.builder import buildCOLR, buildCPAL
from fontTools.ttLib import TTFont

SOURCE = "/System/Library/Fonts/Supplemental/Georgia.ttf"
LETTERS = [chr(c) for c in range(ord("A"), ord("Z") + 1)] + [chr(c) for c in range(ord("a"), ord("z") + 1)]


def rename(font, family):
    for rec in font["name"].names:
        if rec.nameID in (1, 16):
            rec.string = family
        elif rec.nameID in (4,):
            rec.string = family
        elif rec.nameID == 6:
            rec.string = family.replace(" ", "")


def glyphs(font):
    cmap = font.getBestCmap()
    return {ch: cmap[ord(ch)] for ch in LETTERS if ord(ch) in cmap}


def v0(out):
    font = TTFont(SOURCE)
    g = glyphs(font)
    layers = {name: [(name, 0), (g["o"], 1)] for name in g.values()}
    font["COLR"] = buildCOLR(layers, version=0)
    font["CPAL"] = buildCPAL([[(0.9, 0.1, 0.15, 1.0), (0.1, 0.3, 0.95, 0.5)]])
    rename(font, "PM COLR Zero")
    font.save(os.path.join(out, "pm-colr-v0.ttf"))


def v1(out):
    font = TTFont(SOURCE)
    g = glyphs(font)
    units = font["head"].unitsPerEm
    paints = {}
    for name in g.values():
        paints[name] = {
            "Format": 10,  # PaintGlyph
            "Glyph": name,
            "Paint": {
                "Format": 4,  # PaintLinearGradient
                "ColorLine": {"ColorStop": [(0.0, 0), (1.0, 1)]},
                "x0": 0, "y0": 0, "x1": units // 2, "y1": units, "x2": units, "y2": 0,
            },
        }
    font["COLR"] = buildCOLR(paints, version=1)
    font["CPAL"] = buildCPAL([[(1.0, 0.6, 0.0, 1.0), (0.5, 0.0, 0.9, 1.0)]])
    rename(font, "PM COLR One")
    font.save(os.path.join(out, "pm-colr-v1.ttf"))


if __name__ == "__main__":
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    v0(out)
    v1(out)
    print("wrote", out)
