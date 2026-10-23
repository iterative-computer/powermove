import { afterEach, describe, expect, it, vi } from 'vitest';
import { glyphLayout } from './text-layout';

afterEach(() => vi.unstubAllGlobals());

describe('glyph layout metrics', () => {
  it('keeps incoming kerning and alignment exact when glyph widths repeat', () => {
    const measureText = vi.fn((text: string) => ({
      width: text.length * 10 - (text.match(/AV/g)?.length ?? 0) * 2,
    }));
    for (const [align, positions] of [
      ['left', [0, 8]], ['center', [-9, -1]], ['right', [-18, -10]],
    ] as const) {
      const layout = glyphLayout({ text: 'AV\nAV', size: 20, leading: 1, align }, { measureText });
      expect(layout.characters.map(glyph => glyph.x)).toEqual([...positions, ...positions]);
      expect(layout.characters.map(glyph => glyph.w)).toEqual([10, 10, 10, 10]);
      expect(layout.characters.map(glyph => glyph.sourceStart)).toEqual([0, 1, 3, 4]);
      expect(layout.characters.map(glyph => glyph.y)).toEqual([0, 0, 20, 20]);
      expect(layout.words.map(word => word.w)).toEqual([18, 18]);
    }
  });

  it('preserves emoji and combining marks as whole glyphs and UTF-16 source offsets', () => {
    const family = '👨‍👩‍👧‍👦';
    const layout = glyphLayout({ text: `A${family}e\u0301\n${family} A`, size: 10 }, {
      measureText: (text: string) => ({ width: Array.from(text).length * 10 }),
    });
    expect(layout.characters.map(glyph => glyph.text)).toEqual(['A', family, 'e\u0301', family, 'A']);
    expect(layout.characters.map(glyph => glyph.sourceStart)).toEqual([0, 1, 12, 15, 27]);
    expect(layout.characters.map(glyph => glyph.x)).toEqual([0, 10, 80, 0, 80]);
    expect(layout.characters.map(glyph => glyph.w)).toEqual([10, 70, 20, 70, 10]);
    expect(layout.characters.map(glyph => glyph.word)).toEqual([0, 0, 0, 1, 2]);
  });

  it('remeasures after font metrics change instead of retaining widths between layouts', () => {
    let advance = 10;
    const measureText = (text: string) => ({ width: text.length * advance });
    const first = glyphLayout({ text: 'AAA', size: 20 }, { measureText });
    advance = 15;
    const second = glyphLayout({ text: 'AAA', size: 20 }, { measureText });
    expect(first.characters.map(glyph => glyph.w)).toEqual([10, 10, 10]);
    expect(second.characters.map(glyph => glyph.w)).toEqual([15, 15, 15]);
    expect(second.characters.map(glyph => glyph.x)).toEqual([0, 15, 30]);
  });

  it('measures repeated isolated glyphs once while still measuring joined runs', () => {
    const measureText = vi.fn((text: string) => ({ width: text.length * 10 }));
    const layout = glyphLayout({ text: Array(100).fill('AVATAR').join('\n'), size: 20 }, { measureText });
    expect(layout.characters).toHaveLength(600);
    expect(measureText.mock.calls.filter(([text]) => text === 'T')).toHaveLength(1);
    expect(measureText.mock.calls.filter(([text]) => text === 'AV')).toHaveLength(100);
    expect(measureText.mock.calls.length).toBeLessThan(1000);
  });

  it('retains code-point fallback when Unicode segmentation is unavailable', () => {
    vi.stubGlobal('Intl', { Segmenter: undefined });
    const layout = glyphLayout({ text: '😀e\u0301', size: 20 }, {
      measureText: (text: string) => ({ width: Array.from(text).length * 10 }),
    });
    expect(layout.characters.map(glyph => glyph.text)).toEqual(['😀', 'e', '\u0301']);
    expect(layout.characters.map(glyph => glyph.sourceStart)).toEqual([0, 2, 3]);
  });

  it('retains nonmonotonic prefix metrics from negative tracking', () => {
    const layout = glyphLayout({ text: 'AVAV', size: 20 }, {
      measureText: (text: string) => ({ width: text.length * 10 - Math.max(0, text.length - 1) * 12 }),
    });
    expect(layout.characters.map(glyph => glyph.x)).toEqual([0, -2, -4, -6]);
    expect(layout.lines[0].w).toBe(4);
  });
});
