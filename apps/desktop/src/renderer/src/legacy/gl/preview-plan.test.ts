import { expect, it } from 'vitest';
import { previewPlan } from './preview-plan';

const layer = (extra = {}): any => ({ type: 'shape', from: 0, dur: 10, d: {}, p: {}, ...extra });
it('shares static spans but separates clip entrances and exits', () => {
  const plan = previewPlan({ layers: [layer(), layer({ from: 3, dur: 2 })] });
  expect(plan.timeKey(1)).toBe(plan.timeKey(2));
  expect(plan.timeKey(2)).not.toBe(plan.timeKey(4));
  expect(plan.timeKey(4)).not.toBe(plan.timeKey(6));
  expect(plan.timeKey(3)).toBe(3);
  expect(plan.timeKey(3 - 1e-6)).toBe(3 - 1e-6);
});
it('renders changing keyframes normally and reuses their settled holds', () => {
  const plan = previewPlan({ layers: [layer({ from: 1, p: { x: { v: 0, kf: [{ t: 1, v: 0 }, { t: 3, v: 10 }] } } })] });
  expect(plan.timeKey(1.2)).toBe(plan.timeKey(1.8));
  expect(plan.timeKey(2.5)).toBe(2.5);
  expect(plan.timeKey(5)).toBe(plan.timeKey(6));
  expect(plan.timeKey(1.99, .02)).toBe(1.99);
  expect(plan.timeKey(4.01, .02)).toBe(4.01);
});
it('keeps expressions, custom renderers, effects and video on the live path', () => {
  for (const extra of [{ p: { x: { v: 0, kf: [], expr: 'T' } } }, { type: 'video' }, { type: 'precomp' }, { fx: [{}] }, { d: { animators: [{}] } }]) {
    const plan = previewPlan({ layers: [layer(extra)] });
    expect(plan.timeKey(1)).toBe(1);
    expect(plan.timeKey(2)).toBe(2);
  }
});
it('only admits independent static layers to the reusable bottom stack', () => {
  const staticLayer = layer();
  const plan = previewPlan({ layers: [staticLayer, layer({ parent: 'moving' }), layer({ group: 'moving' }), layer({ p: { x: { v: 0, kf: [{ t: 0, v: 0 }, { t: 1, v: 1 }] } } })] });
  expect([...plan.staticLayers]).toEqual([staticLayer]);
});
it('retains static grouped artwork but follows animated ancestors and rejects cycles', () => {
  const group = layer({ id: 'group', type: 'group' });
  const child = layer({ group: 'group' });
  expect(previewPlan({ layers: [group, child] }).staticLayers.has(child)).toBe(true);
  group.p = { x: { v: 0, kf: [], expr: 'T' } };
  expect(previewPlan({ layers: [group, child] }).staticLayers.has(child)).toBe(false);
  expect(previewPlan({ layers: [layer({ id: 'a', parent: 'b' }), layer({ id: 'b', parent: 'a' })] }).staticLayers.size).toBe(0);
});
it('reuses flat and held sections of animation without freezing native speed-handle overshoots', () => {
  const a = { t: 0, v: 20, outInterp: 'bezier', outEase: { speed: 0, influence: 33 } };
  const b = { t: 2, v: 20, inInterp: 'bezier', inEase: { speed: 0, influence: 33 }, outInterp: 'hold' };
  const c = { t: 4, v: 40 };
  const project = { layers: [layer({ p: { x: { v: 20, kf: [a, b, c] } } }), layer({ type: 'audio' })] };
  expect(previewPlan(project).timeKey(.5)).toBe(previewPlan(project).timeKey(1.5));
  expect(previewPlan(project).timeKey(2.5)).toBe(previewPlan(project).timeKey(3.5));
  a.outEase.speed = 10;
  expect(previewPlan(project).timeKey(.5)).toBe(.5);
});

it('retains settled background animation past inactive clips and sound', () => {
  const background = layer({ p: { x: { v: 0, kf: [{ t: 0, v: 0 }, { t: 1, v: 10 }, { t: 4, v: 10, outInterp: 'hold' }, { t: 6, v: 20 }] } } });
  const foreground = layer({ p: { x: { v: 0, kf: [{ t: 0, v: 0 }, { t: 10, v: 100 }] } } });
  const layers = [foreground, background, layer({ type: 'video', from: 8, dur: 2 }), layer({ type: 'audio' })];
  const plan = previewPlan({ layers });
  expect(plan.prefixStart(layers, 2)).toBe(1);
  expect(plan.prefixStart(layers, .5)).toBe(layers.length - 2);
  expect(plan.prefixStart(layers, 5)).toBe(1);
  expect(plan.prefixStart(layers, 8.5)).toBe(layers.length - 1);
});

it('follows keyed parent and group motion before retaining settled descendants', () => {
  const parent = layer({ id: 'parent', type: 'null', p: { x: { v: 0, kf: [{ t: 0, v: 0 }, { t: 3, v: 100 }] } } });
  const group = layer({ id: 'group', type: 'group', parent: 'parent' });
  const child = layer({ group: 'group' });
  const project = { layers: [child, group, parent] };
  const plan = previewPlan(project);
  expect(plan.prefixStart([child, parent], 2)).toBe(1);
  expect(plan.prefixStart([child, parent], 4)).toBe(0);
  expect(plan.prefixStart([child, parent], 3.01, .02)).toBe(2);
  expect(plan.prefixStart([group, child], 4)).toBe(1);
});

it('requires proven stationary effects and never admits unknown pictures or speed overshoots', () => {
  const effect = { type: 'gradient', on: true };
  const painted = layer({ fx: [effect] });
  const layers = [painted];
  expect(previewPlan({ layers }).prefixStart(layers, 2)).toBe(1);
  const admitted = previewPlan({ layers }, candidate => candidate === effect);
  expect(admitted.prefixStart(layers, 2)).toBe(0);
  painted.fx[0].p = { amount: { v: 0, kf: [{ t: 0, v: 0 }, { t: 4, v: 100 }] } };
  expect(previewPlan({ layers }, () => true).prefixStart(layers, 2)).toBe(1);
  for (const extra of [{ type: 'video' }, { type: 'precomp' }, { type: 'shader' }, { masks: [{}] }, { transitionIn: {} },
    { p: { x: { v: 10, kf: [{ t: 0, v: 10, outEase: { speed: 12 } }, { t: 4, v: 10 }] } } },
    { p: { x: { v: 0, kf: [], expr: 'T' } } }]) {
    const candidate = layer(extra);
    expect(previewPlan({ layers: [candidate] }, () => true).prefixStart([candidate], 2)).toBe(1);
  }
});

it('skips invisible unknown layers only during proven constant visibility spans', () => {
  const background = layer();
  const invisible = layer({ type: 'shader', p: { opacity: { v: 0, kf: [{ t: 0, v: 0, hold: true }, { t: 4, v: 100 }] } } });
  const layers = [background, invisible];
  const plan = previewPlan({ layers });
  expect(plan.prefixStart(layers, 2)).toBe(0);
  expect(plan.prefixStart(layers, 5)).toBe(2);
  expect(plan.prefixStart(layers, 4 - 1e-6)).toBe(2);
  invisible.p.opacity.kf[0].hold = false;
  expect(previewPlan({ layers }).prefixStart(layers, 2)).toBe(2);
  invisible.p.opacity.kf[1].v = 0;
  invisible.p.opacity.kf[0].outEase = { speed: 5 };
  expect(previewPlan({ layers }).prefixStart(layers, 2)).toBe(2);
  invisible.p = {}; invisible.on = { v: false };
  // Malformed switches are truthy objects in the live engine, not properties.
  expect(previewPlan({ layers }).prefixStart(layers, 2)).toBe(2);
});
