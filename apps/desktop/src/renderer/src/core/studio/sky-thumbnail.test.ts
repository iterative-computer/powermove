import { describe, expect, it } from 'vitest';
import { skyThumbnailPixels } from './sky-thumbnail';

const mean = (px: Uint8ClampedArray) => { let s = 0; for (let i = 0; i < px.length; i += 4) s += px[i]! + px[i + 1]! + px[i + 2]!; return s / (px.length / 4) / 3; };
const top = (px: Uint8ClampedArray, w: number) => [px[(2 * w + 4) * 4]!, px[(2 * w + 4) * 4 + 1]!, px[(2 * w + 4) * 4 + 2]!];

describe('sky thumbnails', () => {
  it('keeps the presets in their real brightness order', () => {
    const noon = mean(skyThumbnailPixels({ elevation: 45, haze: 2, overcast: 0, sunIntensity: 1 }, 24, 24));
    const blue = mean(skyThumbnailPixels({ elevation: -5, haze: 2, overcast: 0, sunIntensity: 0 }, 24, 24));
    const night = mean(skyThumbnailPixels({ elevation: -20, haze: 2, overcast: 0, sunIntensity: 0 }, 24, 24));
    expect(noon).toBeGreaterThan(blue);
    expect(blue).toBeGreaterThanOrEqual(night);
  });
  it('draws a blue clear sky and an opaque image', () => {
    const px = skyThumbnailPixels({ elevation: 45, haze: 2, overcast: 0, sunIntensity: 1 }, 24, 24);
    const [r, , b] = top(px, 24);
    expect(b).toBeGreaterThan(r!);
    for (let i = 3; i < px.length; i += 4) expect(px[i]).toBe(255);
  });
});

it('shows the same sunlit ground in the preset thumbnail', () => {
  const px = skyThumbnailPixels({ elevation: 45, haze: 2, overcast: 0, sunIntensity: 1 }, 24, 24);
  const ground = [...px.slice((22 * 24 + 12) * 4, (22 * 24 + 12) * 4 + 3)];
  expect(Math.min(...ground)).toBeGreaterThan(100);
});
