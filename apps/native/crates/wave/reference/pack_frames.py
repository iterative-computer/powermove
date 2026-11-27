"""Pack synthetic reference PNGs into 18 small 11-frame strips (Pillow required)."""
from pathlib import Path
import sys
from PIL import Image

source, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
for kind in ["vertical", "horizontal", "center"]:
    for palette in ["colorful", "green", "ocean", "sunset", "mono", "528dff"]:
        atlas = Image.new("RGBA", (155 * 11, 100))
        for index in range(11):
            image = Image.open(source / f"{kind}-{palette}-{index}.png").convert("RGBA")
            pixels = image.load()
            small = Image.new("RGBA", (155, 100))
            dest = small.load()
            for y in range(100):
                for x in range(155):
                    dest[x, y] = tuple(
                        (sum(pixels[x * 4 + dx, y * 4 + dy][c]
                             for dy in range(4) for dx in range(4)) + 8) // 16
                        for c in range(4)
                    )
            atlas.paste(small, (index * 155, 0))
        atlas.save(output / f"{kind}-{palette}.png", optimize=True)
