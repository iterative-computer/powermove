import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './raster';

function rasterRegistry(): PMRegistry {
  const context2d: any = {
    font: '', letterSpacing: '', textBaseline: '', textAlign: '', fillStyle: '',
    scale() {}, fillText() {},
    measureText(text: string) {
      const width = Math.max(1, text.length * 48);
      const left = this.textAlign === 'center' ? width / 2 : this.textAlign === 'right' ? width : 0;
      return {
        width,
        actualBoundingBoxLeft: left,
        actualBoundingBoxRight: width - left,
        actualBoundingBoxAscent: 78,
        actualBoundingBoxDescent: 18,
      };
    },
  };
  vi.stubGlobal('window', {
    document: {
      createElement(_tag: string) {
        return { width: 0, height: 0, getContext: () => context2d };
      },
    },
  });
  const PM: PMRegistry = {
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
  };
  install(PM);
  return PM;
}

afterEach(() => vi.unstubAllGlobals());

describe('legacy raster install', () => {
  it('keeps padded raster textures and tight text selection bounds', () => {
    const PM = rasterRegistry();
    const text = { text: 'Powermove', font: 'Geist', weight: 650, size: 100, tracking: 0, leading: 1, color: '#fff', align: 'center', italic: false };
    const raster = PM.raster({ type: 'text', d: text });

    expect(raster.h).toBeGreaterThan(250);
    expect(raster.selection.h).toBeLessThan(raster.h * .5);
    expect(raster.selection.w).toBeLessThan(raster.w);
    expect(raster.selection.w).toBeGreaterThan(350);
    expect(Math.abs(raster.selection.x0 + raster.selection.x1)).toBeLessThan(1);

    const left = PM.raster({ type: 'text', d: { ...text, align: 'left' } }).selection;
    const right = PM.raster({ type: 'text', d: { ...text, align: 'right' } }).selection;
    expect(left.x0 < 0 && left.x0 > -12 && left.x1 > 350).toBe(true);
    expect(right.x1 > 0 && right.x1 < 12 && right.x0 < -350).toBe(true);

    const layout = PM.textLayout({ ...text, text: 'AB\nC', leading: 1.1 });
    expect(layout.characters.map((piece: any) => piece.text)).toEqual(['A', 'B', 'C']);
  });
});
