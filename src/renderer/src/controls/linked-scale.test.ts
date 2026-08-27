import { expect, it } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { EditGesture } from './gesture';
import { channelBinding } from './binding';

it('keeps the starting scale ratio through zero, keys both axes, and undoes once', () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 });
  const layer = PM.mkLayer('solid'); PM.proj.layers = [layer];
  layer.scaleLinked = true; layer.p['scale.y'].v = 50;
  PM.setKey(layer, 'scale.x', 0, 100, 'linear'); PM.setKey(layer, 'scale.y', 0, 50, 'linear');
  const gesture = new EditGesture(PM, channelBinding(PM, layer.id, 'scale.x', { time: 1 }));
  gesture.begin(); gesture.write(0); gesture.write(200); gesture.commit();
  expect(PM.ev(layer, 'scale.x', 1)).toBe(200);
  expect(PM.ev(layer, 'scale.y', 1)).toBe(100);
  expect(layer.p['scale.x'].kf).toHaveLength(2);
  expect(layer.p['scale.y'].kf).toHaveLength(2);
  PM.hist.undo();
  expect(PM.L(layer.id).p['scale.x'].kf).toHaveLength(1);
  expect(PM.L(layer.id).p['scale.y'].kf).toHaveLength(1);
});
