import { describe, expect, it } from 'vitest';
import { makePM } from './make-pm';
import { editVideo } from '../assistant/video-editing';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: 'Video edit', fps: 30, dur: 30 });
  PM.time = 0;
  PM.syncShaderUniforms = () => {};
  PM.mkEffect = () => null;
  const video = PM.mkLayer('video', { from: 2, dur: 8, d: { asset: 'footage', trim: PM.P(1), speed: PM.P(2), embeddedAudio: true } });
  PM.proj.layers.push(video);
  const run = (args: any) => editVideo(PM, { layerId: video.id, ...args }, { label: 'Edit video', origin: 'agent', historyGroup: 'video-run' });
  return { PM, video, run };
}

describe('agent video editing', () => {
  it('splits source-continuous footage and preserves effects, masks, and animation through Undo/Redo', () => {
    const { PM, video, run } = editor();
    PM.setKeyOn(video.p['position.x'], 5, 100, 'linear', PM.proj.fps);
    video.fx = [{ id: 'fx', type: 'blur', p: {}, on: true }];
    video.masks = [{ id: 'mask', p: {} }];
    const before = JSON.stringify(PM.proj.layers);
    const mark = PM.hist.mark();
    const split = run({ operation: 'split', at: 5 });
    expect(split.ok).toBe(true);
    const tail = PM.L(split.data.result.tailId);
    expect(tail.from).toBe(5);
    expect(tail.dur).toBe(5);
    expect(tail.d.trim.v).toBe(7);
    expect(tail.p['position.x'].kf[0].t).toBe(2);
    expect(tail.fx).toEqual(video.fx);
    expect(tail.masks).toEqual(video.masks);
    expect(run({ operation: 'move', layerId: tail.id, from: 12 }).ok).toBe(true);
    expect(PM.hist.squash(mark, 'Agent video', 'video-run')).toBeTruthy();
    expect(PM.hist.list()).toHaveLength(1);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.proj.layers)).toBe(before);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.L(tail.id).from).toBe(12);
  });

  it('trims source time at speed and retimes the retained source range', () => {
    const { video, run } = editor();
    expect(run({ operation: 'trim', start: 4, end: 8 }).ok).toBe(true);
    expect(video.d.trim.v).toBe(5);
    expect(video.dur).toBe(4);
    video.d.embeddedAudio = false;
    expect(run({ operation: 'speed', speed: 1 }).ok).toBe(true);
    expect(video.dur).toBe(8);
    expect(video.d.trim.v).toBe(5);
  });

  it('separates audio as an editable layer with matching trim and placement', () => {
    const { PM, video, run } = editor();
    video.d.speed = PM.P(1);
    const mark = PM.hist.mark();
    const result = run({ operation: 'separate_audio' });
    expect(result.ok).toBe(true);
    const audio = PM.L(result.data.result.audioId);
    expect(audio).toMatchObject({ type: 'audio', from: 2, dur: 8, d: { asset: 'footage', trim: { v: 1 } } });
    expect(video.d.embeddedAudio).toBe(false);
    expect(run({ operation: 'audio', layerId: audio.id, gain: .5, fadeIn: .2 }).ok).toBe(true);
    expect(audio.d.gain.v).toBe(.5);
    PM.hist.squash(mark, 'Agent audio', 'video-run');
    PM.hist.undo();
    expect(PM.proj.layers).toHaveLength(1);
    expect(PM.L(video.id).d.embeddedAudio).toBe(true);
  });

  it('rejects invalid operations atomically, locked clips, and animated source timing', () => {
    const { PM, video, run } = editor();
    const before = JSON.stringify(PM.proj);
    for (const args of [{ operation: 'split', at: 2 }, { operation: 'trim', start: 0, end: 6 }, { operation: 'speed', speed: 0 }, { operation: 'separate_audio' }]) {
      expect(run(args).ok).toBe(false);
      expect(JSON.stringify(PM.proj)).toBe(before);
    }
    PM.L(video.id).lock = true;
    expect(() => run({ operation: 'move', from: 0 })).toThrow('locked');
    PM.L(video.id).lock = false;
    PM.L(video.id).d.speed.kf.push({ i: 'rate', t: 0, v: 2 });
    expect(run({ operation: 'split', at: 5 }).ok).toBe(false);
  });

  it('inserts a bounded source segment from the media library', () => {
    const { PM, run } = editor();
    PM.proj.assets.footage = { id: 'footage', kind: 'video', name: 'Shot', dur: 20 };
    const result = run({ operation: 'insert', assetId: 'footage', from: 10, sourceIn: 4, duration: 3 });
    expect(result.ok).toBe(true);
    expect(PM.L(result.data.result.layerId)).toMatchObject({ from: 10, dur: 3, d: { asset: 'footage', trim: { v: 4 } } });
    expect(run({ operation: 'insert', assetId: 'footage', from: 10, sourceIn: 19, duration: 3 }).ok).toBe(false);
  });
});
