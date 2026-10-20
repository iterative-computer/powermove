import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { animatedGlyphs, animatorWeight, countTextUnits, isIdentityGlyph, textAnimationKey, textAnimationState, unitRank } from './text-animation';
import { ANIMATOR_PRESETS, makeAnimator, outDelay, setAnimatorMode } from '../../../../extensions/inspector/text-animators';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: 'Text animators', w: 1000, h: 600, dur: 5 });
  const layer = PM.mkLayer('text', { d: { text: 'ab cd', size: 20 } }, PM.proj);
  layer.from = 0;
  PM.proj.layers = [layer];
  PM.ProjectIndex.invalidate();
  return { PM, layer };
}

/* Four 10px characters in two words on one line. */
const layout = {
  characters: [0, 1, 2, 3].map((index) => ({ text: 'x', x: index * 10 + (index > 1 ? 5 : 0), y: 0, w: 10, index, line: 0, lineUnit: 0, word: index > 1 ? 1 : 0, sourceStart: index + (index > 1 ? 1 : 0) })),
  words: [{ x: 0, w: 20, line: 0 }, { x: 25, w: 20, line: 0 }],
  lines: [{ x: 0, w: 45, line: 0 }]
};
const preset = (id: string) => ANIMATOR_PRESETS.find((candidate) => candidate.id === id)!;

afterEach(() => vi.unstubAllGlobals());

describe('text animators', () => {
  it('staggers units in over time without keyframes', () => {
    const { PM, layer } = editor();
    const animator = makeAnimator(PM.P, 'a', 'Fade', preset('custom-stagger'), 20);
    animator.easing = 'linear';
    animator.p.duration.v = 1;
    animator.p.stagger.v = 0.5;
    layer.d.animators = [animator];
    const opacity = (time: number) => animatedGlyphs(PM, layer, time, { size: 20, color: '#ffffff' }, layout).map((glyph: any) => glyph.opacity);
    expect(opacity(0)).toEqual([0, 0, 0, 0]);
    expect(opacity(0.5)).toEqual([0.5, 0, 0, 0]);
    expect(opacity(1)).toEqual([1, 0.5, 0, 0]);
    expect(opacity(3)).toEqual([1, 1, 1, 1]);
    expect(animatedGlyphs(PM, layer, 3, { size: 20, color: '#ffffff' }, layout).every(isIdentityGlyph)).toBe(true);
  });

  it('plays Out animations from rest toward the animated values', () => {
    const { PM, layer } = editor();
    const animator = makeAnimator(PM.P, 'a', 'Fade', preset('custom-stagger'), 20);
    Object.assign(animator, { direction: 'out', easing: 'linear' });
    animator.p.duration.v = 1;
    animator.p.stagger.v = 0;
    animator.p.delay.v = 2;
    layer.d.animators = [animator];
    const opacity = (time: number) => animatedGlyphs(PM, layer, time, { size: 20 }, layout)[0].opacity;
    expect(opacity(1)).toBe(1);
    expect(opacity(2.5)).toBeCloseTo(0.5);
    expect(opacity(4)).toBe(0);
    expect(outDelay({ duration: 1, stagger: 0.1 }, [{ text: 'ab cd' }], 'characters', 'forward', 5)).toBe(3.7);
    // Wrapped paragraph lines count as separate lines.
    expect(outDelay({ duration: 1, stagger: 0.5 }, [{ text: 'one two' }, { text: 'three' }], 'lines', 'forward', 5)).toBe(3.5);
  });

  it('orders units forward, reversed, from the center and randomly but deterministically', () => {
    expect([0, 1, 2, 3, 4].map((i) => unitRank('reverse', i, 5))).toEqual([4, 3, 2, 1, 0]);
    expect([0, 1, 2, 3, 4].map((i) => unitRank('center', i, 5))).toEqual([2, 1, 0, 1, 2]);
    expect([0, 1, 2, 3, 4].map((i) => unitRank('edges', i, 5))).toEqual([0, 1, 2, 1, 0]);
    const random = [0, 1, 2, 3, 4, 5, 6, 7].map((i) => unitRank('random', i, 8, 7));
    expect([...random].sort()).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    expect([0, 1, 2, 3, 4, 5, 6, 7].map((i) => unitRank('random', i, 8, 7))).toEqual(random);
  });

  it('scales and rotates words around their own center', () => {
    const { PM, layer } = editor();
    const animator = makeAnimator(PM.P, 'a', 'Pop', preset('custom-range'), 20);
    animator.unit = 'words';
    delete animator.p.opacity;
    animator.p.scale = PM.P(200);
    layer.d.animators = [animator];
    const glyphs = animatedGlyphs(PM, layer, 0, { size: 20 }, layout);
    // The word "ab" spans 0…20, so its center (x = 10) stays in place.
    const at = (glyph: any, x: number, y: number) => glyph.x + glyph.matrix[0] * x + glyph.matrix[2] * y + glyph.matrix[4];
    const pivotY = -20 * 0.35;
    expect(at(glyphs[0], 10 - glyphs[0].x, pivotY)).toBeCloseTo(10);
    expect(at(glyphs[1], 10 - glyphs[1].x, pivotY)).toBeCloseTo(10);
    expect(at(glyphs[1], 20 - glyphs[1].x, pivotY)).toBeCloseTo(30);
  });

  it('accumulates tracking along the line and keeps centered text centered', () => {
    const { PM, layer } = editor();
    const animator = makeAnimator(PM.P, 'a', 'Track', preset('custom-range'), 20);
    delete animator.p.opacity;
    animator.p.tracking = PM.P(10);
    layer.d.animators = [animator];
    const left = animatedGlyphs(PM, layer, 0, { size: 20, align: 'left' }, layout).map((glyph: any) => glyph.trackingShift);
    expect(left).toEqual([0, 10, 20, 30]);
    const center = animatedGlyphs(PM, layer, 0, { size: 20, align: 'center' }, layout).map((glyph: any) => glyph.trackingShift);
    expect(center).toEqual([-15, -5, 5, 15]);
  });

  it('keeps legacy range selectors, shapes and the unit channel working', () => {
    expect(animatorWeight({ start: 0, end: 100 }, 0, 4, 'rampUp')).toBeCloseTo(0.125);
    expect(animatorWeight({ start: 0, end: 100 }, 3, 4, 'rampDown')).toBeCloseTo(0.125);
    expect(animatorWeight({ start: 0, end: 100 }, 1.5, 4, 'triangle')).toBeCloseTo(1);
    const { PM, layer } = editor();
    layer.d.animators = [{ id: 'legacy', name: 'Animator 1', p: Object.fromEntries(Object.entries({ unit: 'words', start: 0, end: 50, offset: 0, smoothness: 0, x: 0, y: 0, rotation: 0, scale: 100, opacity: 0, tracking: 0 }).map(([key, value]) => [key, PM.P(value)])) }];
    expect(animatedGlyphs(PM, layer, 0, { size: 20 }, layout).map((glyph: any) => glyph.opacity)).toEqual([0, 0, 1, 1]);
    setAnimatorMode(PM.P, layer.d.animators[0], 'stagger');
    expect(layer.d.animators[0]).toMatchObject({ mode: 'stagger', unit: 'words' });
    expect(layer.d.animators[0].p.unit).toBeUndefined();
    expect(layer.d.animators[0].p.start).toBeUndefined();
    expect(layer.d.animators[0].p.duration.v).toBeGreaterThan(0);
  });

  it('skips disabled animators and changes its cache key as time-based animation plays', () => {
    const { PM, layer } = editor();
    layer.d.animators = [makeAnimator(PM.P, 'a', 'Rise', preset('rise'), 20)];
    const key = (time: number) => textAnimationKey(textAnimationState(PM, layer, time, layout));
    expect(key(0.1)).not.toBe(key(0.2));
    expect(key(10)).toBe(key(20));
    layer.d.animators[0].enabled = false;
    expect(animatedGlyphs(PM, layer, 0, { size: 20 }, layout).every(isIdentityGlyph)).toBe(true);
  });

  it('waves continuously and blends colors by weight', () => {
    const { PM, layer } = editor();
    const wave = makeAnimator(PM.P, 'w', 'Wave', preset('wave'), 20);
    const color = makeAnimator(PM.P, 'c', 'Tint', preset('custom-range'), 20);
    delete color.p.opacity;
    color.p.color = PM.P('#ff0000');
    color.p.amount.v = 50;
    layer.d.animators = [wave, color];
    const glyphs = animatedGlyphs(PM, layer, 0.25, { size: 20, color: '#000000' }, layout);
    expect(glyphs[0].matrix[5]).toBeCloseTo(-3 * Math.sin(Math.PI / 2));
    expect(glyphs[0].color).toBe('#800000');
  });

  it('counts Unicode units using one segmenter across every line', () => {
    const NativeSegmenter = Intl.Segmenter;
    let constructions = 0;
    class CountingSegmenter extends NativeSegmenter {
      constructor() {
        super(undefined, { granularity: 'grapheme' });
        constructions++;
      }
    }
    vi.stubGlobal('Intl', { Segmenter: CountingSegmenter });
    const lines = Array.from({ length: 100 }, () => ({ text: '👨‍👩‍👧‍👦 e\u0301\tA' }));
    expect(countTextUnits(lines, 'characters')).toBe(300);
    expect(constructions).toBe(1);
    expect(countTextUnits(lines, 'words')).toBe(300);
    expect(countTextUnits([...lines, { text: '' }], 'lines')).toBe(100);
    expect(constructions).toBe(1);
  });

  it('keeps code-point counting when Unicode segmentation is unavailable', () => {
    vi.stubGlobal('Intl', { Segmenter: undefined });
    expect(countTextUnits([{ text: '😀e\u0301 \t' }], 'characters')).toBe(3);
  });

  it('skips sparse input holes and avoids Unicode setup for an empty unit list', () => {
    const lines = new Array<{ text: string }>(3);
    lines[1] = { text: '😀A' };
    expect(countTextUnits(lines, 'characters')).toBe(2);
    vi.stubGlobal('Intl', { Segmenter: class { constructor() { throw new Error('Unexpected Unicode setup'); } } });
    expect(countTextUnits([], 'characters')).toBe(0);
    const characters = new Array<any>(3);
    characters[1] = { text: 'A', line: 0, x: 10, w: 10, index: 1 };
    const result = animatedGlyphs({}, {}, 0, { size: 20 }, { characters, words: [], lines: [] }, { animators: [], styles: [] });
    expect(result).toHaveLength(3);
    expect(0 in result).toBe(false);
    expect(2 in result).toBe(false);
    expect(result[1].x).toBe(10);
    expect(result[1].trackingShift).toBe(0);
  });

  it('keeps tracking within each line even when glyphs are interleaved', () => {
    const input = {
      characters: [
        { line: 2, x: 0, index: 0 }, { line: 1, x: 0, index: 1 },
        { line: 2, x: 10, index: 2 }, { line: 1, x: 10, index: 3 },
        { line: 2, x: 20, index: 4 },
      ].map(glyph => ({ ...glyph, text: 'x', w: 10, y: glyph.line * 20 })),
      words: [], lines: [],
    };
    const state = {
      styles: [], animators: [{ id: 'track', mode: 'range' as const, unit: 'characters' as const,
        values: { tracking: 10 }, weights: [1, -0.5, 0.25, 0.75, 2] }],
    };
    const glyphs = animatedGlyphs({}, {}, 0, { size: 20, align: 'center' }, input, state);
    expect(glyphs.map((glyph: any) => glyph.trackingShift)).toEqual([-6.25, 2.5, 3.75, -2.5, 6.25]);
    expect(glyphs.map((glyph: any) => glyph.x)).toEqual([-6.25, 2.5, 13.75, 7.5, 26.25]);
    expect(input.characters.map(glyph => glyph.x)).toEqual([0, 0, 10, 10, 20]);
  });
});
