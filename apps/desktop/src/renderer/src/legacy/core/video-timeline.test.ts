import { expect, it } from 'vitest';
import { videoClipsAt } from './video-timeline';

function registry() {
  const video: any = { id: 'video', type: 'video', from: 0, dur: 5, d: { asset: 'media', trim: 0, speed: 1 } };
  const asset = { el: {}, dur: 10 };
  const PM: any = {
    proj: { fps: 30, layers: [video], comps: {} }, scope: [],
    assets: new Map([['media', asset]]),
    active: (layer: any, time: number) => layer.on !== false && time >= layer.from && time < layer.from + layer.dur,
    clamp: (n: number, min: number, max: number) => Math.max(min, Math.min(max, n)),
    evP: (_: any, p: any) => p.v, animVersion: () => 1,
  };
  return { PM, video };
}

it('maps nested instances to independent source times and composed playback rates', () => {
  const { PM, video } = registry();
  PM.proj.comps.sub = { layers: [video] };
  PM.proj.layers = [0, 1].map(i => ({ id: `instance${i}`, type: 'precomp', from: 0, dur: 5, d: { comp: 'sub', trim: i, speed: 2 } }));
  const clips = videoClipsAt(PM, .25);
  expect(clips.map(({ id, at, rate }) => ({ id, at, rate }))).toEqual([
    { id: 'instance0/video', at: .5, rate: 2 }, { id: 'instance1/video', at: 1.5, rate: 2 },
  ]);
  expect(PM.scope).toEqual([]);
});

it('does not start invisible or out-of-range media', () => {
  const { PM, video } = registry();
  expect(videoClipsAt(PM, 5)).toEqual([]);
  video.on = false;
  expect(videoClipsAt(PM, 1)).toEqual([]);
});

it('uses seek-driven playback for animated and expression-driven rates', () => {
  const { PM, video } = registry();
  video.d.speed = { v: 1, kf: [], expr: '1' } as any;
  expect(videoClipsAt(PM, .2)[0]?.rate).toBeNull();
});

it('restores composition scope even when a frame mapping fails', () => {
  const { PM } = registry();
  PM.assets.get = () => { throw new Error('bad asset'); };
  expect(() => videoClipsAt(PM, 0)).toThrow('bad asset');
  expect(PM.scope).toEqual([]);
});
