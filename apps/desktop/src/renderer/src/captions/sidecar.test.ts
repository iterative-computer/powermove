import { describe, expect, it } from 'vitest';
import { captionSidecars } from './sidecar';
import { parseSrt } from './formats';
import { presetStyle } from './model';

const layer = (id: string, name: string, language: string | undefined, cues: Array<[number, number, string]>, from = 0) => ({
  id, name, type: 'captions', from, dur: 20, on: true,
  d: { cues: cues.map(([start, end, text], index) => ({ id: `${id}${index}`, start, end, text })), style: presetStyle('classic'), ...(language ? { language } : {}) }
});

describe('captionSidecars', () => {
  it('writes one unsuffixed file for a single layer, rebased to the export range', () => {
    const files = captionSidecars({ layers: [layer('a', 'English', 'en', [[1, 2, 'one'], [5, 6, 'two']])] }, 'srt', { from: 4, to: 10 });
    expect(files).toHaveLength(1);
    expect(files[0]!.suffix).toBeNull();
    expect(parseSrt(files[0]!.text).cues.map(cue => [cue.start, cue.end, cue.text])).toEqual([[1, 2, 'two']]);
  });

  it('names several layers by language, then by name, without collisions', () => {
    const files = captionSidecars({ layers: [
      layer('a', 'English', 'en', [[0, 1, 'hi']]),
      layer('b', 'Français', 'fr', [[0, 1, 'salut']]),
      layer('c', 'Notes', undefined, [[0, 1, 'x']]),
      layer('d', 'English copy', 'en', [[0, 1, 'hey']]),
      { ...layer('e', 'Hidden', 'de', [[0, 1, 'aus']]), on: false }
    ] }, 'vtt', { from: 0, to: 20 });
    expect(files.map(file => file.suffix)).toEqual(['en', 'fr', 'Notes', 'en-2']);
    expect(files[1]!.text.startsWith('WEBVTT\nLanguage: fr')).toBe(true);
  });
});
