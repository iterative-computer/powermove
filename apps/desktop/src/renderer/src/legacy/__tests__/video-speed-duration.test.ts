import { describe, expect, it } from 'vitest';
import { makePM } from './make-pm';
import { sourceTime } from '../core/retiming';

function editor(wrapped = true) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ fps: 30, dur: 30 });
  PM.time = 2;
  const video = PM.mkLayer('video', { from: 2, dur: 8, d: { trim: 3, speed: 1 } });
  if (wrapped) video.d.speed = PM.P(1);
  else video.d.speed = 1;
  PM.proj.layers.push(video);
  const command = (speed: number) => ({ type: 'set_content', target: video.id, patch: { speed } });
  const apply = (speed: number) => PM.Edit.apply(command(speed), { label: 'Speed', origin: 'inspector' });
  return { PM, video, command, apply };
}

describe('video speed and strip duration', () => {
  for (const wrapped of [false, true]) {
    it(`preserves the trimmed source range when changing ${wrapped ? 'animated-property' : 'plain'} speed`, () => {
      const { PM, video, apply } = editor(wrapped);
      const sourceEnd = sourceTime(PM, video, video.from + video.dur);
      expect(apply(2).ok).toBe(true);
      expect(video.dur).toBe(4);
      expect(video.from).toBe(2);
      expect(sourceTime(PM, video, video.from + video.dur)).toBe(sourceEnd);
      expect(apply(.5).ok).toBe(true);
      expect(video.dur).toBe(16);
      expect(sourceTime(PM, video, video.from + video.dur)).toBe(sourceEnd);
    });
  }

  it('also retimes direct speed property edits', () => {
    const { PM, video } = editor();
    expect(PM.Edit.apply({ type: 'set_property', target: video.id, path: 'c.speed', value: 6.71 }, { origin: 'timeline' }).ok).toBe(true);
    expect(video.dur).toBeCloseTo(8 / 6.71);
  });

  it('undoes and redoes speed and duration together', () => {
    const { PM, video, apply } = editor();
    expect(apply(2).ok).toBe(true);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(video.id)).toMatchObject({ dur: 8, d: { speed: { v: 1 } } });
    expect(PM.hist.redo()).toBe(true);
    expect(PM.L(video.id)).toMatchObject({ dur: 4, d: { speed: { v: 2 } } });
  });

  it('keeps live scrubbing proportional and restores duration on cancellation', () => {
    const { PM, video, command } = editor();
    PM.Edit.begin('Speed', { origin: 'inspector' });
    for (const speed of [2, 3, 6.71, .5]) {
      PM.Edit.dispatch(command(speed));
      expect(video.dur).toBeCloseTo(8 / speed);
    }
    PM.Edit.cancel();
    expect(PM.L(video.id)).toMatchObject({ dur: 8, d: { speed: { v: 1 } } });
  });

  it('preserves a manually trimmed range at a non-default speed', () => {
    const { video, apply } = editor();
    expect(apply(2).ok).toBe(true);
    video.dur = 1.5;
    expect(apply(1).ok).toBe(true);
    expect(video.dur).toBe(3);
  });

  it('handles replacement speed properties without resizing twice', () => {
    const { PM, video } = editor();
    expect(PM.Edit.apply({ type: 'set_content', target: video.id, patch: { speed: PM.P(2) } }).ok).toBe(true);
    expect(video.dur).toBe(4);
  });

  it('retains at least one timeline frame', () => {
    const { video, apply } = editor();
    video.dur = 1 / 30;
    expect(apply(2).ok).toBe(true);
    expect(video.dur).toBe(1 / 30);
  });

  it('commits a scrub as one undoable speed and duration edit', () => {
    const { PM, video, command } = editor();
    PM.Edit.begin('Speed', { origin: 'inspector' });
    PM.Edit.dispatch(command(2));
    PM.Edit.dispatch(command(4));
    PM.Edit.commit('Speed');
    expect(PM.hist.list()).toHaveLength(1);
    expect(video.dur).toBe(2);
    PM.hist.undo();
    expect(PM.L(video.id)).toMatchObject({ dur: 8, d: { speed: { v: 1 } } });
  });

  for (const timing of ['keyframes', 'expression', 'timeRemap', 'new keyframe']) {
    it(`leaves duration alone for ${timing}`, () => {
      const { PM, video, apply } = editor();
      if (timing === 'keyframes') PM.setKeyOn(video.d.speed, 0, 1, 'linear', 30);
      if (timing === 'expression') video.d.speed.expr = '1';
      if (timing === 'timeRemap') video.d.timeRemap = PM.P(true);
      const result = timing === 'new keyframe'
        ? PM.Edit.apply({ type: 'set_property', target: video.id, path: 'c.speed', value: 2, mode: 'keyframe' })
        : apply(2);
      expect(result.ok).toBe(true);
      expect(video.dur).toBe(8);
    });
  }
});
