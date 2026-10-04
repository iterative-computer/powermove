import { describe, expect, it } from 'vitest';
import { layoutCaption, placeBlock, wordStates } from './layout';
import { presetStyle, type CaptionCue } from './model';
import { createCaptionRasterizer } from './render';

/** Monospace-ish fake metrics: every character is half the font size wide. */
const measure = (text: string, size: number) => text.length * size * 0.5;

const cue = (text: string, extra: Partial<CaptionCue> = {}): CaptionCue => ({ id: 'c', start: 0, end: 4, text, ...extra });

describe('layoutCaption', () => {
  const style = { ...presetStyle('classic'), size: 40, maxWidth: 50, maxLines: 2 };

  it('wraps at the max width and balances two lines', () => {
    // Column: 50% of 1000px = 500px = 25 characters at 40px.
    const layout = layoutCaption(cue('the quick brown fox jumps over the lazy dog'), style, measure, { compositionWidth: 1000 });
    expect(layout.lines).toHaveLength(2);
    const [a, b] = layout.lines.map(line => line.words.map(word => word.text).join(' '));
    expect(Math.abs(a!.length - b!.length)).toBeLessThanOrEqual(6);
    expect(layout.width).toBeLessThanOrEqual(500);
    expect(layout.size).toBe(40);
  });

  it('shrinks text that would need more lines than allowed', () => {
    const long = 'word '.repeat(30).trim();
    const layout = layoutCaption(cue(long), style, measure, { compositionWidth: 1000 });
    expect(layout.size).toBeLessThan(40);
    expect(layout.size).toBeGreaterThanOrEqual(24);
  });

  it('keeps authored line breaks, centres lines and applies text case', () => {
    const layout = layoutCaption(cue('Hi\nthere friend'), { ...style, textCase: 'upper' }, measure, { compositionWidth: 1000 });
    expect(layout.lines.map(line => line.words.map(word => word.text).join(' '))).toEqual(['HI', 'THERE FRIEND']);
    const first = layout.lines[0]!;
    expect(first.words[0]!.x).toBeCloseTo((layout.width - first.width) / 2);
    expect(layout.words.map(word => word.text)).toEqual(['HI', 'THERE', 'FRIEND']);
  });
});

describe('placement', () => {
  const base = presetStyle('classic');
  it('sits inside the title-safe area', () => {
    const bottom = placeBlock({ width: 400, height: 100 }, base, 1920, 1080);
    expect(bottom.y1).toBeCloseTo(540 - 1080 * 0.08);
    expect(bottom.x0).toBeCloseTo(-200);
    const top = placeBlock({ width: 400, height: 100 }, { ...base, placement: 'top', offsetY: 10 }, 1920, 1080);
    expect(top.y0).toBeCloseTo(-540 + 1080 * 0.08 + 10);
    const left = placeBlock({ width: 400, height: 100 }, { ...base, align: 'left', box: true, boxPadding: 10 }, 1920, 1080);
    expect(left.x0).toBeCloseTo(-1920 * 0.4);
    expect(left.x1 - left.x0).toBeCloseTo(420);
  });
});

describe('word states', () => {
  const words = [{ text: 'a', start: 0, end: 1 }, { text: 'b', start: 1, end: 2 }];
  it('highlights the spoken word and grows it', () => {
    const states = wordStates(words, { ...presetStyle('spotlight') }, 1.5);
    expect(states.map(state => state.highlighted)).toEqual([false, true]);
    expect(states[1]!.scale).toBeCloseTo(1.08);
  });
  it('pops and fades words in as they are spoken', () => {
    const pop = wordStates(words, presetStyle('pop'), 0.5);
    expect(pop[1]).toMatchObject({ alpha: 0, scale: 0.7 });
    const fade = wordStates(words, { ...presetStyle('classic'), wordAnimation: 'fade' }, 1.05);
    expect(fade[0]!.alpha).toBe(1);
    expect(fade[1]!.alpha).toBeGreaterThan(0);
    expect(fade[1]!.alpha).toBeLessThan(1);
  });
});

describe('createCaptionRasterizer', () => {
  const calls: string[] = [];
  const context: any = {
    font: '', fillStyle: '', strokeStyle: '', lineWidth: 0, globalAlpha: 1, letterSpacing: '0px',
    scale() {}, translate() {}, save() {}, restore() {}, beginPath() {}, moveTo() {}, arcTo() {}, closePath() {},
    fill() { calls.push(`box ${this.fillStyle}`); },
    measureText(text: string) { const size = Number(/(\d+(?:\.\d+)?)px/.exec(this.font)?.[1] ?? 10); return { width: measure(text, size) }; },
    fillText(text: string) { calls.push(`fill ${text} ${this.fillStyle}`); },
    strokeText(text: string) { calls.push(`stroke ${text}`); }
  };
  const rasterizer = createCaptionRasterizer({
    canvas: (width, height) => ({ width: Math.ceil(width), height: Math.ceil(height), getContext: () => context }) as any,
    font: style => `${style.weight} ${style.size}px "${style.font}"`,
    fontReady: () => true
  });
  const content = {
    cues: [cue('hello world', { start: 1, end: 3, words: [{ text: 'hello', start: 1, end: 2 }, { text: 'world', start: 2, end: 3 }] })],
    style: { ...presetStyle('boxed'), highlight: true, highlightColor: '#FF0000' }
  };

  it('shows nothing between cues and the active cue otherwise', () => {
    expect(rasterizer.frame(content, 0.5, 1920, 1080)).toBeNull();
    expect(rasterizer.frame(content, 3, 1920, 1080)).toBeNull();
    const frame = rasterizer.frame(content, 2.5, 1920, 1080)!;
    expect(frame.cue.text).toBe('hello world');
    expect(frame.states.map(state => state.highlighted)).toEqual([false, true]);
  });

  it('paints the box, strokes and the highlighted word, anchored at the frame centre', () => {
    calls.length = 0;
    const frame = rasterizer.frame(content, 2.5, 1920, 1080)!;
    const raster = rasterizer.draw(frame, 1);
    expect(calls[0]).toMatch(/^box rgba\(0,0,0,0\.72\)/);
    expect(calls).toContain('fill world #FF0000');
    expect(calls).toContain('fill hello #FFFFFF');
    expect(raster.anchorX - frame.margin).toBeCloseTo(-frame.block.x0);
    expect(raster.selection.y1).toBeLessThan(540);
    expect(raster.blank).toBe(false);
  });

  it('changes its cache key only when what is drawn changes', () => {
    const a = rasterizer.frame(content, 1.2, 1920, 1080)!.key;
    const b = rasterizer.frame(content, 1.5, 1920, 1080)!.key;
    const c = rasterizer.frame(content, 2.5, 1920, 1080)!.key;
    expect(a).toBe(b);
    expect(c).not.toBe(a);
    const still = { ...content, style: presetStyle('classic') };
    expect(rasterizer.frame(still, 1.2, 1920, 1080)!.key).toBe(rasterizer.frame(still, 2.9, 1920, 1080)!.key);
  });

  it('gives idle bounds between cues for selection', () => {
    const bounds = rasterizer.idleBounds(content, 1920, 1080);
    expect(bounds.x1 - bounds.x0).toBeCloseTo(1920 * 0.8);
  });
});
