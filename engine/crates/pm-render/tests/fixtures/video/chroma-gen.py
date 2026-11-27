# Ground truth for tests/chroma_truth.rs (see README.md):
# writes the 4:4:4 truth and its [1 2 1]/4 co-sited decimations as raw yuv.

import math, sys, random

W, H = 256, 128
out = sys.argv[1]
random.seed(7)

def fbm_tables(seed):
    rnd = random.Random(seed)
    return [[rnd.random() for _ in range(64)] for _ in range(64)]

T = [fbm_tables(s) for s in (1, 2, 3)]

def noise(t, x, y):
    xi, yi = int(math.floor(x)), int(math.floor(y))
    fx, fy = x - xi, y - yi
    fx, fy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)
    a = t[yi % 64][xi % 64]; b = t[yi % 64][(xi + 1) % 64]
    c = t[(yi + 1) % 64][xi % 64]; d = t[(yi + 1) % 64][(xi + 1) % 64]
    return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy

def fbm(t, x, y):
    v, a, f = 0.0, 0.5, 1.0 / 24
    for _ in range(5):
        v += a * noise(t, x * f, y * f); f *= 2.0; a *= 0.5
    return v

def natural(x, y):
    # Smooth, band-limited colour like a photograph: hue and lightness fields.
    h = fbm(T[0], x, y) * 6.28 * 1.5
    l = 0.25 + 0.55 * fbm(T[1], x + 50, y)
    s = 0.15 + 0.6 * fbm(T[2], x, y + 70)
    return [min(1, max(0, l + s * 0.5 * math.cos(h + k * 2.094))) for k in range(3)]

def graphics(x, y):
    # Hard-edged motion-graphics content: shapes, thin lines, glyph strokes.
    x0 = x - 128
    c = [0.12, 0.13, 0.18]
    if 8 <= x0 < 60 and 10 <= y < 58: c = [0.95, 0.20, 0.25]
    if (x0 - 92) ** 2 + (y - 34) ** 2 < 22 ** 2: c = [0.15, 0.55, 0.95]
    if 70 <= y < 118 and 10 <= x0 < 118 and (x0 + y) % 14 < 7: c = [0.98, 0.85, 0.10]
    if y in (64, 65) or x0 in (64,): c = [0.10, 0.95, 0.40]
    # glyph-like strokes
    gx, gy = x0 % 16, y % 20
    if 90 <= y < 128 and 0 <= x0 < 60 and (gx in (3, 4) or (gy in (6, 7) and 3 <= gx < 12)): c = [1, 1, 1]
    return c

KR, KB = 0.2126, 0.0722
KG = 1 - KR - KB

def ycc(rgb):
    r, g, b = rgb
    y = KR * r + KG * g + KB * b
    return y, (b - y) / (2 * (1 - KB)), (r - y) / (2 * (1 - KR))

Y = [[0.0] * W for _ in range(H)]; CB = [[0.0] * W for _ in range(H)]; CR = [[0.0] * W for _ in range(H)]
for y in range(H):
    for x in range(W):
        rgb = natural(x, y) if x < 128 else graphics(x, y)
        Y[y][x], CB[y][x], CR[y][x] = ycc(rgb)

qy = lambda v: max(0, min(255, round(16 + 219 * v)))
qc = lambda v: max(0, min(255, round(128 + 224 * v)))

def plane(rows, q):
    return bytes(q(v) for row in rows for v in row)

def h121(rows):  # co-sited [1 2 1]/4 at even columns
    w = len(rows[0])
    return [[(row[max(2 * i - 1, 0)] + 2 * row[2 * i] + row[min(2 * i + 1, w - 1)]) / 4 for i in range(w // 2)] for row in rows]

def v11(rows):  # interstitial rows
    return [[(rows[2 * j][i] + rows[2 * j + 1][i]) / 2 for i in range(len(rows[0]))] for j in range(len(rows) // 2)]

open(f"{out}/truth444.yuv", "wb").write(plane(Y, qy) + plane(CB, qc) + plane(CR, qc))
open(f"{out}/dec422.yuv", "wb").write(plane(Y, qy) + plane(h121(CB), qc) + plane(h121(CR), qc))
open(f"{out}/dec420.yuv", "wb").write(plane(Y, qy) + plane(v11(h121(CB)), qc) + plane(v11(h121(CR)), qc))
