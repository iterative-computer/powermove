import { afterEach, describe, expect, it, vi } from 'vitest';

import { makePM } from '../legacy/__tests__/make-pm';
import { addKeyCommand, levelCommand, MIXER_ORIGIN, muteCommand, nextSolo } from './edits';
import { levelControl } from './level-control';

const previousWindow = (globalThis as any).window;
afterEach(() => { (globalThis as any).window = previousWindow; });

function editor() {
  (globalThis as any).window = { addEventListener() {}, removeEventListener() {} };
  const PM: any = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'core/capabilities');
  PM.proj = PM.mkProject({ w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 2;
  const add = (type: string, id: string, d: any = {}) => {
    const layer = PM.mkLayer(type, { name: id, from: 0, dur: 10, d });
    layer.id = id;
    Object.assign(layer.d, d);
    PM.proj.layers.push(layer);
    return layer;
  };
  const music = add('audio', 'music', { asset: 'a', gain: 1 });
  const clip = add('video', 'clip', { asset: 'v', embeddedAudio: true });
  PM.hist.clear?.();
  const apply = (commands: any, label = 'Mixer') => PM.Edit.apply(commands, { label, origin: MIXER_ORIGIN });
  return { PM, music, clip, apply };
}

const strip = (id: string, kind: 'audio' | 'video' | 'precomp', extra: any = {}) => ({
  id, kind, name: id, gainKey: kind === 'audio' ? 'gain' as const : 'audioGain' as const, animated: false, muted: false, ...extra
});

describe('mixer edits', () => {
  it('writes a static level to the layer content and the master to the composition', () => {
    const { PM, music, clip, apply } = editor();
    expect(apply(levelCommand(strip('music', 'audio'), 0.5, 2)).ok).toBe(true);
    expect(apply(levelCommand(strip('clip', 'video'), 2, 2)).ok).toBe(true);
    expect(apply(levelCommand('master', 0.25, 2)).ok).toBe(true);
    expect(music.d.gain).toBe(0.5);
    expect(clip.d.audioGain).toBe(2);
    expect(PM.proj.audioGain).toBe(0.25);
    expect(apply(levelCommand('master', 9, 2)).ok).toBe(true);
    expect(PM.proj.audioGain).toBe(4);
  });

  it('normalises a soundtrack level written through set_content', () => {
    const { PM } = editor();
    // A refused edit rolls the project back to a copy, so always re-read the layer.
    const clip = () => PM.L('clip');
    const set = (patch: any) => PM.Edit.apply({ type: 'set_content', target: 'clip', patch }, { label: 'Agent', origin: 'agent' });
    expect(set({ audioGain: 100 }).ok).toBe(true);
    expect(clip().d.audioGain).toBe(4);
    expect(set({ audioGain: -1 }).ok).toBe(true);
    expect(clip().d.audioGain).toBe(0);
    expect(set({ audioGain: '0.5' }).ok).toBe(false);
    expect(clip().d.audioGain).toBe(0);
    expect(set({ audioGain: null }).ok).toBe(true);
    expect(Object.hasOwn(clip().d, 'audioGain')).toBe(false);
    expect(set({ audioMuted: 'yes' }).ok).toBe(false);
    expect(set({ audioMuted: true }).ok).toBe(true);
    expect(clip().d.audioMuted).toBe(true);
  });

  it('writes an animated level as a keyframe at the gesture time, not the moving playhead', () => {
    const { PM, music, apply } = editor();
    expect(apply(addKeyCommand(strip('music', 'audio'), 1, 1)).ok).toBe(true);
    expect(music.d.gain.kf.map((key: any) => key.t)).toEqual([1]);
    PM.time = 7;
    expect(apply(levelCommand(strip('music', 'audio', { animated: true }), 0.25, 3)).ok).toBe(true);
    expect(music.d.gain.kf.map((key: any) => [key.t, key.v])).toEqual([[1, 1], [3, 0.25]]);
  });

  it('keyframes a soundtrack level that was never set, starting from unity', () => {
    const { clip, apply } = editor();
    expect(clip.d.audioGain).toBeUndefined();
    expect(apply(addKeyCommand(strip('clip', 'video'), 1, 2)).ok).toBe(true);
    expect(clip.d.audioGain.kf).toHaveLength(1);
    expect(clip.d.audioGain.kf[0].v).toBe(1);
  });

  it('mutes through each layer kind\'s own switch, even when the layer is locked', () => {
    const { music, clip, apply } = editor();
    music.lock = true;
    expect(apply(muteCommand(strip('music', 'audio'))).ok).toBe(true);
    expect(music.on).toBe(false);
    expect(apply(muteCommand(strip('music', 'audio', { muted: true }))).ok).toBe(true);
    expect(music.on).toBe(true);
    expect(apply(muteCommand(strip('clip', 'video'))).ok).toBe(true);
    expect(clip.d.audioMuted).toBe(true);
  });

  it('refuses a level change on a locked layer', () => {
    const { music, apply } = editor();
    music.lock = true;
    expect(apply(levelCommand(strip('music', 'audio'), 0.5, 2)).ok).toBe(false);
    expect(music.d.gain).toBe(1);
  });

  it('toggles monitor solo, with Option for exclusive listening', () => {
    expect([...nextSolo(new Set(), 'a', false)]).toEqual(['a']);
    expect([...nextSolo(new Set(['a']), 'b', false)]).toEqual(['a', 'b']);
    expect([...nextSolo(new Set(['a', 'b']), 'a', false)]).toEqual(['b']);
    expect([...nextSolo(new Set(['a', 'b']), 'c', true)]).toEqual(['c']);
    expect([...nextSolo(new Set(['c']), 'c', true)]).toEqual([]);
  });
});

describe('level gestures', () => {
  function kernelLike(PM: any) {
    return {
      edit: {
        begin: (...args: any[]) => PM.Edit.begin(...args), dispatch: (command: any) => PM.Edit.dispatch(command),
        commit: (label: string) => PM.Edit.commit(label), cancel: () => PM.Edit.cancel(), apply: (...args: any[]) => PM.Edit.apply(...args)
      },
      transport: { time: () => PM.time },
      ui: { toast: vi.fn() }
    } as any;
  }

  it('coalesces a whole drag into one Undo step and re-tunes audio on every move', () => {
    const { PM, music } = editor();
    const retune = vi.fn();
    const control = levelControl(kernelLike(PM), () => ({ retune }), () => strip('music', 'audio'));
    const before = PM.hist.list().length;
    expect(control.begin()).toBe(true);
    for (const gain of [0.9, 0.8, 0.7, 0.6]) control.write(gain);
    control.commit();
    expect(music.d.gain).toBe(0.6);
    expect(retune).toHaveBeenCalledTimes(5);
    expect(PM.hist.list().length).toBe(before + 1);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L('music').d.gain).toBe(1);
  });

  it('cancels a gesture without leaving an Undo step', () => {
    const { PM } = editor();
    const control = levelControl(kernelLike(PM), () => undefined, () => strip('music', 'audio'));
    const before = PM.hist.list().length;
    control.begin(); control.write(0.3); control.cancel();
    expect(PM.L('music').d.gain).toBe(1);
    expect(PM.hist.list().length).toBe(before);
  });

  it('reports a refused edit once per gesture', () => {
    const { PM, music } = editor();
    music.lock = true;
    const api = kernelLike(PM);
    const control = levelControl(api, () => undefined, () => strip('music', 'audio'));
    control.begin(); control.write(0.5); control.write(0.4); control.cancel();
    expect(api.ui.toast).toHaveBeenCalledTimes(1);
  });
});
