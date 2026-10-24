import { expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { resolveContent } from './content-properties';
import { local3D } from './space-3d';

const benchmark = process.env.PM_ANIMATION_THROUGHPUT === '1' ? it : it.skip;

benchmark('measures repeated-frame transform and content evaluation for a heavy composition', () => {
  const PM = makePM('core/easing', 'core/model', 'core/anim');
  PM.proj = PM.mkProject({ dur: 30 });
  const layers = Array.from({ length: 5000 }, (_, index) => PM.mkLayer('shape', {
    d: { w: 200, h: 100, color: '#ed5b32', paths: [], metadata: { label: `Shape ${index}` } },
    p: { 'position.x': index, 'position.y': index % 100, rotation: index % 360 },
  }));
  PM.proj.layers = layers;
  let checksum = 0;
  const measure = (action: (time: number) => void) => {
    const samples: number[] = [];
    for (let run = 0; run < 9; run++) {
      const started = performance.now();
      for (let frame = 0; frame < 30; frame++) {
        const time = (run * 30 + frame) / 30;
        PM.beginEval(time); action(time);
      }
      if (run > 1) samples.push(performance.now() - started);
    }
    samples.sort((a, b) => a - b);
    return samples[Math.floor(samples.length / 2)];
  };
  const transformMs = measure(time => {
    for (const layer of layers) checksum += PM.worldMatrix(layer, time)[4];
  });
  const contentMs = measure(time => {
    for (const layer of layers) checksum += resolveContent(PM, layer, time).w;
  });
  process.stdout.write('ANIMATION_THROUGHPUT ' + JSON.stringify({ layers: layers.length, frames: 30, transformMs, contentMs }) + '\n');
  expect(checksum).toBeGreaterThan(0);
});

benchmark('measures shared nested group transforms and fully rotated 3D transforms', () => {
  const PM = makePM('core/easing', 'core/model', 'core/anim');
  PM.proj = PM.mkProject({ dur: 30 });
  const groups = Array.from({ length: 12 }, (_, index) => {
    const group = PM.mkLayer('group', { p: { 'position.x': index, 'position.y': index, rotation: index } });
    return group;
  });
  groups.forEach((group, index) => { if (index) group.group = groups[index - 1].id; });
  const layers = Array.from({ length: 2000 }, (_, index) => {
    const layer = PM.mkLayer('solid', { p: {
      'position.x': index, 'position.y': index % 100,
      'anchor.x': 30, 'anchor.y': -20, 'anchor.z': 5,
      'rotation.x': 20, 'rotation.y': 35, rotation: index % 360,
      'orientation.x': -10, 'orientation.y': 15, 'orientation.z': 30,
      'scale.x': 75, 'scale.y': 110, 'scale.z': -80, skew: 12,
    } });
    layer.group = groups[groups.length - 1].id;
    layer.threeD = true;
    return layer;
  });
  PM.proj.layers = [...groups, ...layers];
  let checksum = 0;
  const measure = (action: (time: number) => void) => {
    const samples: number[] = [];
    for (let run = 0; run < 9; run++) {
      const started = performance.now();
      for (let frame = 0; frame < 30; frame++) {
        const time = (run * 30 + frame) / 30;
        PM.beginEval(time); action(time);
      }
      if (run > 1) samples.push(performance.now() - started);
    }
    samples.sort((a, b) => a - b);
    return samples[Math.floor(samples.length / 2)];
  };
  const groupedTransformMs = measure(time => {
    for (const layer of layers) checksum += PM.worldMatrix(layer, time)[4];
  });
  const local3DMs = measure(time => {
    for (const layer of layers) checksum += local3D(PM, layer, time)[12];
  });
  process.stdout.write('GROUP_3D_THROUGHPUT ' + JSON.stringify({ layers: layers.length, groupDepth: groups.length,
    frames: 30, groupedTransformMs, local3DMs, checksum }) + '\n');
  expect(checksum).toBeGreaterThan(0);
});
