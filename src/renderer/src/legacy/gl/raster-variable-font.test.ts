import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { formatFontVariationSettings, install, textVariationEntries } from './raster';

afterEach(() => vi.unstubAllGlobals());

describe('variable-font text raster', () => {
  it('formats registered and arbitrary axis tags deterministically', () => {
    const content = { 'fontAxis.wght': 620, 'fontAxis.GRAD': 40, ignored: 1 };
    expect(textVariationEntries(content)).toEqual([['GRAD', 40], ['wght', 620]]);
    expect(formatFontVariationSettings(content)).toBe('"GRAD" 40, "wght" 620');
  });

  it('evaluates axis channels at render time and applies them to the FontFace used by canvas', async () => {
    const contexts: any[] = [];
    const descriptors = new Map<string, string>();
    const rule = { style: { setProperty: (key: string, value: string) => descriptors.set(key, value) } };
    const sheet = { cssRules: [] as any[], insertRule: vi.fn(() => { sheet.cssRules.push(rule); return 0; }) };
    vi.stubGlobal('window', {
      queryLocalFonts: vi.fn(async () => [{
        family: 'Test Variable Render', fullName: 'Test Variable Render Regular',
        postscriptName: 'TestVariableRender-Regular',
      }]),
      document: {
        head: { append: vi.fn() },
        getElementById: vi.fn(() => null),
        fonts: { load: vi.fn(async () => []), ready: Promise.resolve() },
        createElement: (tag: string) => {
          if (tag === 'style') return { id: '', sheet };
          const context: any = {
            font: '', letterSpacing: '', textBaseline: '', textAlign: '', fillStyle: '',
            scale() {}, fillText() {},
            measureText: (text: string) => ({
              width: text.length * 20, actualBoundingBoxLeft: 0,
              actualBoundingBoxRight: text.length * 20,
              actualBoundingBoxAscent: 30, actualBoundingBoxDescent: 8,
            }),
          };
          contexts.push(context);
          return { width: 0, height: 0, getContext: () => context };
        },
      },
    });
    const PM: PMRegistry = {
      clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
      evP: vi.fn((_layer: any, property: any, time: number) => property.v + time),
      invalidate: vi.fn(),
    };
    install(PM);
    const layer = {
      type: 'text',
      d: {
        text: 'Axis', font: 'Test Variable Render', weight: 400, size: 48,
        tracking: 0, leading: 1, color: '#fff', align: 'left', italic: false,
        'fontAxis.wdth': { v: 75, kf: [{ t: 0, v: 75 }], expr: null },
        'fontAxis.GRAD': { v: 20, kf: [], expr: null },
      },
    };
    const first = PM.raster(layer, 1, 2);
    expect(sheet.insertRule).toHaveBeenCalledOnce();
    expect(descriptors.get('font-variation-settings')).toBe('"GRAD" 22, "wdth" 77');
    expect(contexts.some((context) => context.font.includes('Powermove Variable'))).toBe(true);

    const second = PM.raster(layer, 1, 3);
    expect(second.key).not.toBe(first.key);
    expect(descriptors.get('font-variation-settings')).toBe('"GRAD" 23, "wdth" 78');
    await vi.waitFor(() => expect(descriptors.get('src')).toContain('local("TestVariableRender-Regular")'));
    expect(PM.invalidate).toHaveBeenCalled();
  });
});
