import { describe, expect, it } from 'vitest';
import {
  CAPTION_PRESETS, cueIndexAt, cueRange, cueWords, deleteCues, insertCues, mergeCues, moveCues, moveLimits,
  normalizeCaptionStyle, normalizeCaptionsContent, normalizeCues, patchStyle, presetStyle, splitCue, trimCue, updateCues,
  type CaptionCue
} from './model';

const ids = () => { let n = 0; return () => `n${++n}`; };
const cue = (id: string, start: number, end: number, text = id, extra: Partial<CaptionCue> = {}): CaptionCue => ({ id, start, end, text, ...extra });
const spans = (cues: CaptionCue[]) => cues.map(item => [item.id, item.start, item.end]);

describe('normalisation', () => {
  it('drops malformed cues, fixes reversed times and fills ids', () => {
    const cues = normalizeCues([
      { start: 2, end: 1, text: ' b ' },
      { id: 'x', start: 'a', end: 3, text: 'bad' },
      { id: 'y', start: 4, end: 4.01, text: 'too short' },
      { id: 'z', start: 5, end: 6, text: '   ' },
      null,
      { id: 'k', start: -1, end: 0.5, text: 'clamped' }
    ], ids());
    expect(cues.map(item => [item.id, item.start, item.end, item.text])).toEqual([['k', 0, 0.5, 'clamped'], ['n1', 1, 2, 'b']]);
  });

  it('replaces duplicate ids and keeps words inside their cue', () => {
    const cues = normalizeCues([
      cue('a', 0, 1, 'one two', { words: [{ text: 'one', start: -1, end: 0.4 }, { text: 'two', start: 0.5, end: 9 }] }),
      cue('a', 2, 3)
    ], ids());
    expect(cues.map(item => item.id)).toEqual(['a', 'n1']);
    expect(cues[0]!.words).toEqual([{ text: 'one', start: 0, end: 0.4 }, { text: 'two', start: 0.5, end: 1 }]);
  });

  it('normalises content and style for old or hand-written projects', () => {
    const content = normalizeCaptionsContent({ cues: [cue('a', 0, 1)], style: { size: 'huge', placement: 'sideways', fill: 'red', maxLines: 99 }, language: 'en-US', extra: 1 });
    expect(content.style.size).toBe(54);
    expect(content.style.placement).toBe('bottom');
    expect(content.style.fill).toBe('#FFFFFF');
    expect(content.style.maxLines).toBe(6);
    expect(content.language).toBe('en-US');
    expect(content.extra).toBe(1);
    expect(normalizeCaptionsContent(undefined).cues).toEqual([]);
  });
});

describe('lookups', () => {
  const cues = Array.from({ length: 2000 }, (_, index) => cue(`c${index}`, index, index + 0.8));
  it('finds the active cue by binary search', () => {
    expect(cueIndexAt(cues, 0)).toBe(0);
    expect(cueIndexAt(cues, 0.85)).toBe(-1);
    expect(cueIndexAt(cues, 1500.5)).toBe(1500);
    expect(cueIndexAt(cues, 99999)).toBe(-1);
  });
  it('returns the cues intersecting a range', () => {
    expect(cueRange(cues, 10.9, 13.1)).toEqual([11, 14]);
    expect(cueRange(cues, -5, 0.1)).toEqual([0, 1]);
    expect(cueRange(cues, 5000, 6000)).toEqual([2000, 2000]);
  });
});

describe('operations', () => {
  const base = [cue('a', 0, 1, 'alpha beta'), cue('b', 2, 3, 'gamma'), cue('c', 4, 5, 'delta')];

  it('trims within neighbours and keeps a minimum duration', () => {
    expect(spans(trimCue(base, 'b', 'start', 0.5))).toEqual([['a', 0, 1], ['b', 1, 3], ['c', 4, 5]]);
    expect(spans(trimCue(base, 'b', 'end', 10))).toEqual([['a', 0, 1], ['b', 2, 4], ['c', 4, 5]]);
    expect(trimCue(base, 'b', 'end', 1)[1]!.end).toBeCloseTo(2.04);
  });

  it('moves a group without crossing unmoved neighbours or zero', () => {
    expect(moveLimits(base, ['b'])).toEqual([-1, 1]);
    expect(spans(moveCues(base, ['b'], 5))).toEqual([['a', 0, 1], ['b', 3, 4], ['c', 4, 5]]);
    expect(spans(moveCues(base, ['a', 'b'], -3))).toEqual([['a', 0, 1], ['b', 2, 3], ['c', 4, 5]]);
    expect(spans(moveCues(base, ['b', 'c'], 0.5))).toEqual([['a', 0, 1], ['b', 2.5, 3.5], ['c', 4.5, 5.5]]);
  });

  it('splits text at the time proportion, or at word timings', () => {
    const plain = splitCue([cue('x', 0, 4, 'one two three four')], 'x', 2, ids());
    expect(plain.cues.map(item => [item.id, item.start, item.end, item.text])).toEqual([['x', 0, 2, 'one two'], ['n1', 2, 4, 'three four']]);
    const timed = splitCue([cue('x', 0, 4, 'a b c', { words: [{ text: 'a', start: 0, end: 0.5 }, { text: 'b', start: 0.6, end: 0.9 }, { text: 'c', start: 3, end: 3.5 }] })], 'x', 1, ids());
    expect(timed.cues.map(item => item.text)).toEqual(['a b', 'c']);
    expect(timed.cues[1]!.words).toEqual([{ text: 'c', start: 3, end: 3.5 }]);
    expect(splitCue(base, 'a', 0.01).tailId).toBeNull();
  });

  it('merges the run between the first and last chosen cue', () => {
    const merged = mergeCues(base, ['c', 'a']);
    expect(merged.id).toBe('a');
    expect(merged.cues.map(item => [item.id, item.start, item.end, item.text])).toEqual([['a', 0, 5, 'alpha beta gamma delta']]);
    expect(mergeCues(base, ['a']).id).toBeNull();
  });

  it('updates text, drops stale word timings and deletes', () => {
    const timed = [cue('a', 0, 1, 'hi', { words: [{ text: 'hi', start: 0, end: 1 }] })];
    expect(updateCues(timed, [{ id: 'a', text: 'hello' }])[0]).toEqual({ id: 'a', start: 0, end: 1, text: 'hello' });
    expect(updateCues(timed, [{ id: 'a', text: '  ' }])).toEqual([]);
    expect(spans(deleteCues(base, ['b']))).toEqual([['a', 0, 1], ['c', 4, 5]]);
  });

  it('carries word timings when a cue moves as a whole', () => {
    const timed = [cue('a', 1, 2, 'hi there', { words: [{ text: 'hi', start: 1, end: 1.4 }, { text: 'there', start: 1.5, end: 2 }] })];
    expect(updateCues(timed, [{ id: 'a', start: 3, end: 4 }])[0]!.words).toEqual([{ text: 'hi', start: 3, end: 3.4 }, { text: 'there', start: 3.5, end: 4 }]);
  });

  it('inserts into gaps without moving existing cues', () => {
    const out = insertCues(base, [{ start: 0.5, end: 2.5, text: 'new' }, { start: 6, end: 7, text: 'tail' }], ids());
    expect(out.map(item => [item.start, item.end, item.text])).toEqual([[0, 1, 'alpha beta'], [1, 2, 'new'], [2, 3, 'gamma'], [4, 5, 'delta'], [6, 7, 'tail']]);
  });

  it('estimates word timings for highlight on untimed cues', () => {
    const words = cueWords(cue('a', 0, 2, 'aa b'));
    expect(words.map(word => word.text)).toEqual(['aa', 'b']);
    expect(words[0]!.end).toBeCloseTo(1.2);
    expect(words[1]!.end).toBeCloseTo(2);
  });
});

describe('styles', () => {
  it('ships six presets that each normalise to themselves', () => {
    expect(CAPTION_PRESETS).toHaveLength(6);
    for (const preset of CAPTION_PRESETS) expect(normalizeCaptionStyle(presetStyle(preset.id))).toEqual(presetStyle(preset.id));
  });

  it('scales preset sizes to the composition and marks edited styles custom', () => {
    expect(presetStyle('classic', 2160).size).toBe(108);
    const boxed = patchStyle(presetStyle('classic'), { preset: 'boxed' });
    expect(boxed.box).toBe(true);
    expect(boxed.preset).toBe('boxed');
    expect(patchStyle(boxed, { fill: '#FF0000' }).preset).toBe('custom');
    expect(patchStyle(boxed, { box: true }).preset).toBe('boxed');
    // Position is the layer's own: moving it keeps the look's name.
    expect(patchStyle(boxed, { placement: 'top', offsetY: 0 }).preset).toBe('boxed');
    expect(patchStyle(patchStyle(boxed, { placement: 'top' }), { preset: 'pop' })).toMatchObject({ preset: 'pop', placement: 'top', textCase: 'upper' });
    expect(patchStyle(boxed, { preset: 'pop' }).placement).toBe('middle');
  });
});
