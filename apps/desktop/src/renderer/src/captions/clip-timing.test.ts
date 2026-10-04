// @ts-nocheck -- drives the legacy PM registry like the shortcut suites.
import { beforeEach, describe, expect, it } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { editSelectedLayerTiming, splitLayers } from '../legacy/ui/shortcuts';
import { clipCuesToEnd, rebaseCuesForStart } from './model';

/* Cues live in layer time, so every edit that moves a captions clip's In
   point (split, trim-in) has to re-base them or each subtitle drifts. */

let PM: any;
const SRT = [
  '1\n00:00:01,000 --> 00:00:02,000\nOne',
  '2\n00:00:03,000 --> 00:00:05,000\nTwo spans the cut',
  '3\n00:00:06,000 --> 00:00:07,000\nThree'
].join('\n\n');

/** Cues of a layer in composition time. */
const shown = (layer: any) => layer.d.cues.map((cue: any) => [cue.text, layer.from + cue.start, layer.from + cue.end]);
const captionsLayers = () => PM.proj.layers.filter((layer: any) => layer.type === 'captions');

beforeEach(() => {
  PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'core/capabilities', 'core/media', 'ui/shortcuts', 'gl/shaders');
  PM.proj = PM.mkProject({ w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 0;
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
  PM.Edit.apply({ type: 'add_captions', text: SRT, from: 0 });
  PM.hist.clear?.();
});

describe('captions clip timing', () => {
  it('re-bases cues when the In point moves, clipping the cue spanning it', () => {
    const cues = [{ id: 'a', start: 1, end: 2, text: 'A' }, { id: 'b', start: 3, end: 5, text: 'B', words: [{ text: 'B', start: 3, end: 5 }] }];
    expect(rebaseCuesForStart(cues, 4)).toEqual([{ id: 'b', start: 0, end: 1, text: 'B', words: [{ text: 'B', start: 0, end: 1 }] }]);
    expect(rebaseCuesForStart(cues, -1).map(cue => [cue.start, cue.end])).toEqual([[2, 3], [4, 6]]);
    expect(clipCuesToEnd(cues, 4).map(cue => [cue.id, cue.start, cue.end])).toEqual([['a', 1, 2], ['b', 3, 4]]);
  });

  it('splits a captions clip so both halves keep their cues where they were heard', () => {
    const [layer] = captionsLayers();
    const before = shown(layer);
    PM.time = 4;
    PM.selectLayers(layer.id);
    expect(splitLayers(PM)).toBeTruthy();
    const [tail, head] = captionsLayers();
    expect(head.id).toBe(layer.id);
    expect(tail.from).toBe(4);
    expect(shown(head)).toEqual([['One', 1, 2], ['Two spans the cut', 3, 4]]);
    expect(shown(tail)).toEqual([['Two spans the cut', 4, 5], ['Three', 6, 7]]);
    PM.hist.undo();
    expect(captionsLayers()).toHaveLength(1);
    expect(shown(captionsLayers()[0])).toEqual(before);
  });

  it('splits captions in a blade-all split with nothing selected', () => {
    PM.selectLayers([]);
    PM.time = 4;
    expect(splitLayers(PM)).toBeTruthy();
    const [tail] = captionsLayers();
    expect(shown(tail)[0]).toEqual(['Two spans the cut', 4, 5]);
  });

  it('keeps cues in place when the In point is trimmed to the playhead', () => {
    const [layer] = captionsLayers();
    PM.selectLayers(layer.id);
    PM.time = 2.5;
    expect(editSelectedLayerTiming(PM, 'trimIn')).toMatchObject({ ok: true });
    expect(layer.from).toBe(2.5);
    expect(shown(layer)).toEqual([['Two spans the cut', 3, 5], ['Three', 6, 7]]);
    PM.hist.undo();
    expect(shown(captionsLayers()[0])).toEqual([['One', 1, 2], ['Two spans the cut', 3, 5], ['Three', 6, 7]]);
  });
});
