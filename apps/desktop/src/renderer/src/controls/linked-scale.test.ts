import { expect, it } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { EditGesture } from './gesture';
import { channelBinding } from './binding';

const apiFor = (registry: Record<string, any>) => ({
  edit: registry.Edit,
  history: registry.hist,
  project: { get: () => registry.proj },
  transport: { time: () => registry.time },
  model: { layer: (id: string) => registry.L(id) },
  anim: { ev: (layer: any, key: string, time: number) => registry.ev(layer, key, time) },
  util: { clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)) }
}) as any;

it('keys the unchanged axis too when editing an unlinked animated Scale, and cancels both together', () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 });
  const layer = PM.mkLayer('solid'); PM.proj.layers = [layer];
  layer.scaleLinked = false;
  layer.p['scale.y'].v = 50;
  PM.setKey(layer, 'scale.x', 0, 100);
  const api = apiFor(PM);
  const gesture = new EditGesture(api, channelBinding(api, layer.id, 'scale.x', { time: 1 }));
  gesture.begin(); gesture.write(180); gesture.commit();
  expect(PM.ev(layer, 'scale.y', 1)).toBe(50);
  expect(layer.p['scale.y'].kf[0]).toMatchObject({ t: 1, v: 50, eo: [0, 0], ei: [1, 1] });
  PM.hist.undo();
  expect(PM.L(layer.id).p['scale.y'].kf).toHaveLength(0);
  const cancelled = new EditGesture(api, channelBinding(api, layer.id, 'scale.x', { time: 1 }));
  cancelled.begin(); cancelled.write(200); cancelled.cancel();
  expect(PM.L(layer.id).p['scale.x'].kf).toHaveLength(1);
  expect(PM.L(layer.id).p['scale.y'].kf).toHaveLength(0);
});

it('keeps the starting scale ratio through zero, keys both axes, and undoes once', () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 });
  const layer = PM.mkLayer('solid'); PM.proj.layers = [layer];
  layer.scaleLinked = true; layer.p['scale.y'].v = 50;
  PM.setKey(layer, 'scale.x', 0, 100, 'linear'); PM.setKey(layer, 'scale.y', 0, 50, 'linear');
  const api = apiFor(PM);
  const gesture = new EditGesture(api, channelBinding(api, layer.id, 'scale.x', { time: 1 }));
  gesture.begin(); gesture.write(0); gesture.write(200); gesture.commit();
  expect(PM.ev(layer, 'scale.x', 1)).toBe(200);
  expect(PM.ev(layer, 'scale.y', 1)).toBe(100);
  expect(layer.p['scale.x'].kf).toHaveLength(2);
  expect(layer.p['scale.y'].kf).toHaveLength(2);
  PM.hist.undo();
  expect(PM.L(layer.id).p['scale.x'].kf).toHaveLength(1);
  expect(PM.L(layer.id).p['scale.y'].kf).toHaveLength(1);
});
