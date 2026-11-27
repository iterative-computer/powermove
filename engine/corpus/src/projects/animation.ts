import { newProject } from '../doc';
import { GRADIENT, project, type CorpusProject } from './common';

function interpolation(): CorpusProject {
  const b = newProject('animation/interpolation', { name: 'Keyframe interpolation', backgroundFill: GRADIENT, dur: 3 });
  const row = (name: string, y: number, keys: Parameters<typeof b.ch>[1]) =>
    b.add(b.layer('shape', { name, d: { shape: 'ellipse', w: 28, h: 28, color: '#E8E2CF' }, p: { 'position.x': b.ch(60, keys), 'position.y': y } }));
  const bezier = { inInterp: 'bezier' as const, outInterp: 'bezier' as const, inEase: { speed: 0, influence: 75 }, outEase: { speed: 0, influence: 75 } };
  row('Linear', 30, [[0, 60], [2.5, 580]]);
  row('Native bezier ease', 70, [[0, 60, { ease: bezier }], [2.5, 580, { ease: bezier }]]);
  row('Hold', 110, [[0, 60, { hold: true }], [1, 320, { hold: true }], [2, 580]]);
  // Auto-bezier speeds are derived from the neighbours (AE: (next - previous) / span); the
  // sanitizer recomputes them, so author the derived values.
  const auto = (speed: number) => ({ ease: { inInterp: 'bezier' as const, outInterp: 'bezier' as const, autoBezier: true, inEase: { speed, influence: 100 / 6 }, outEase: { speed, influence: 100 / 6 } } });
  row('Auto bezier', 150, [[0, 60, auto(0)], [1, 400, auto((580 - 60) / 2.5)], [2.5, 580, auto(0)]]);
  row('Continuous with speed', 190, [[0, 60, { ease: { inInterp: 'bezier', outInterp: 'bezier', continuous: true, inEase: { speed: 0, influence: 33 }, outEase: { speed: 300, influence: 50 } } }],
    [1.2, 350, { ease: { inInterp: 'bezier', outInterp: 'bezier', continuous: true, inEase: { speed: 150, influence: 40 }, outEase: { speed: 150, influence: 40 } } }],
    [2.5, 580, { ease: { inInterp: 'bezier', outInterp: 'linear', inEase: { speed: 0, influence: 80 }, outEase: { speed: 0, influence: 100 / 3 } } }]]);
  row('Spring', 230, [[0, 60, { spring: { damping: 10, stiffness: 120, mass: 1 } }], [2.5, 580]]);
  row('Legacy power preset', 270, [[0, 60, { legacy: 'power' }], [2.5, 580, { legacy: 'power' }]]);
  row('Legacy backOut preset', 310, [[0, 60, { legacy: 'backOut' }], [2.5, 580, { legacy: 'backOut' }]]);
  b.add(b.layer('text', {
    name: 'Steps and color', d: {
      text: b.ch('A', [[0, 'A'], [.8, 'B'], [1.6, 'C']]), italic: b.ch(false, [[0, false], [1.2, true]]),
      color: b.ch('#FF6B1A', [[0, '#FF6B1A'], [2.5, '#4C8DFF80']]), size: 36,
    },
    p: { 'position.x': 600, 'position.y': 340 },
  }));
  b.add(b.layer('shape', {
    name: 'Negative key times', from: 1,
    d: { w: 30, h: 30, color: '#3FCF8E' }, p: { 'position.x': 600, 'position.y': 60, rotation: b.ch(0, [[-.5, -90], [1, 90]]) },
  }));
  return project(b, {
    group: 'Animation', title: 'Keyframe interpolation types',
    covers: ['linear', 'bezier (native speed and influence)', 'hold', 'auto-bezier', 'continuous', 'springs', 'legacy easing presets', 'color interpolation with alpha', 'string steps', 'boolean steps', 'keys before the layer start'],
    samples: [0, .4, .8, 1.25, 1.7, 2.1, 2.5, 2.9],
  });
}

function expressions(): CorpusProject {
  const b = newProject('animation/expressions', { name: 'Expressions', backgroundFill: GRADIENT, dur: 3 });
  b.root.params = {
    spread: { name: 'spread', label: 'Spread', control: 'num', min: 0, max: 200, value: 80 },
    tint: { name: 'tint', label: 'Tint', control: 'color', value: '#FF6B1A' },
    show: { name: 'show', label: 'Show', control: 'toggle', value: true },
    mode: { name: 'mode', label: 'Mode', control: 'select', options: [{ v: 'a', label: 'A' }, { v: 'b', label: 'B' }], value: 'b' },
  };
  const exprs: Array<[string, string]> = [
    ['time', '60 + time * 180'],
    ['T and t', '60 + (T + t) * 90'],
    ['fps and value', 'value + fps * 2'],
    ['layer and comp', '60 + comp.w * 0.1 + layer.from * 0'],
    ['clamp lerp', 'lerp(60, 580, clamp(time / 2.5, 0, 1))'],
    ['linear ease', 'linear(time, 0, 2.5, 60, 580) * 0.5 + ease(time, 0, 2.5, 60, 580) * 0.5'],
    ['random', '60 + random(7) * 500'],
    ['wiggle', '320 + wiggle(2, 120)'],
    ['bounce', '60 + bounce(time / 2.5) * 500'],
    ['loop', '60 + loop(1, time) * 500'],
    ['pingpong', '60 + pingpong(1, time) * 500'],
    ['param and Math', '60 + param("spread") * Math.abs(Math.sin(time * PI))'],
    ['idx and ch', '60 + idx * 40 + (ch == "position.x" ? 100 : 0)'],
    ['layer fields and loose equality', '60 + (layer.name == "expr layer fields and loose equality" ? 200 : 0) + ("2" == 2 ? 100 : 0) + (-7 % 3) * 10'],
    ['Math.round half up', '300 + Math.round(-2.5) * 40 + Math.round(2.5) * 10'],
  ];
  exprs.forEach(([name, expr], index) => b.add(b.layer('shape', {
    name: `expr ${name}`, d: { shape: 'ellipse', w: 22, h: 22, color: '#E8E2CF' },
    p: { 'position.x': b.ch(100, [], expr), 'position.y': 20 + index * 26, rotation: b.ch(15) },
  })));
  b.add(b.layer('shape', { name: 'Broken expression keeps value', d: { w: 30, h: 30, color: '#A73366' }, p: { 'position.x': b.ch(600, [], 'this is not valid ('), 'position.y': 340 } }));
  return project(b, {
    group: 'Animation', title: 'Expressions and scene parameters',
    covers: ['expressions: time, T, t, fps, value, layer, comp, ch, idx, PI, Math', 'clamp, lerp, linear, ease, random, wiggle, bounce, loop, pingpong, param', 'scene parameters', 'expression errors'],
    samples: [0, .5, 1.25, 2, 2.75],
  });
}

function transforms(): CorpusProject {
  const b = newProject('animation/transforms', { name: 'Transform channels', backgroundFill: GRADIENT });
  b.add(b.layer('shape', {
    name: 'All 2D channels', d: { w: 120, h: 80, color: '#FF6B1A', radius: 10 },
    p: {
      'anchor.x': b.ch(0, [[0, 0], [2.5, 40]]), 'anchor.y': -20, 'position.x': b.ch(160, [[0, 160], [2.5, 480]]), 'position.y': 180,
      'scale.x': b.ch(100, [[0, 100], [2.5, 60]]), 'scale.y': b.ch(100, [[0, 100], [2.5, 140]]), rotation: b.ch(0, [[0, 0], [2.5, -200]]),
      opacity: b.ch(100, [[0, 100], [2.5, 30]]), skew: b.ch(-80, [[0, -80], [2.5, 80]]),
    },
  }));
  b.add(b.layer('shape', { name: 'Negative scale (mirror)', d: { shape: 'star', w: 90, h: 90, color: '#4C8DFF' }, p: { 'position.x': 320, 'position.y': 300, 'scale.x': -100 } }));
  return project(b, { group: 'Transforms', title: 'Transform channels', covers: ['anchor', 'position', 'scale', 'rotation', 'opacity', 'skew (±80°)', 'negative scale'], samples: [0, 1.25, 2.5] });
}

function compositionMetadata(): CorpusProject {
  const b = newProject('composition/metadata', { name: 'Composition metadata', backgroundFill: GRADIENT, dur: 4 });
  b.root.markers = [{ id: b.id('m'), t: 1, name: 'Beat one' }, { id: b.id('m'), t: 2.5, name: 'Beat two' }];
  b.root.work = [.5, 3.5];
  b.root.audioGain = .8;
  const card = b.add(b.layer('shape', { name: 'Marker-driven', d: { w: 100, h: 100, color: '#3FCF8E' }, p: { rotation: b.ch(0, [[1, 0], [2.5, 90]]) } }));
  void card;
  return project(b, { group: 'Composition', title: 'Markers, work area, master gain', covers: ['markers', 'work area', 'master audio gain'], samples: [.5, 2, 3.5] });
}

function frameRates(): CorpusProject[] {
  return [24000 / 1001, 25, 30000 / 1001, 60].map(fps => {
    const label = fps === 25 || fps === 60 ? String(fps) : fps.toFixed(3);
    const b = newProject(`composition/fps-${label.replace('.', '_')}`, { name: `${label} fps`, fps, dur: 2, backgroundFill: GRADIENT });
    b.add(b.layer('shape', { name: 'Frame counter bar', d: { w: 20, h: 300, color: '#E8E2CF' }, p: { 'position.x': b.ch(20, [[0, 20], [2, 620]]) } }));
    b.add(b.layer('text', { name: 'Frame expression', d: { text: 'fps', size: 40 }, p: { 'position.y': b.ch(180, [], '180 + Math.round(time * fps) % 10 * 8') } }));
    const frame = 1 / fps;
    return project(b, { group: 'Composition', title: `Composition at ${label} fps`, covers: ['rational frame rates', `fps ${label}`], samples: [0, frame * 7, frame * 30, 2 - frame] });
  });
}

export function animationProjects(): CorpusProject[] {
  return [interpolation(), expressions(), transforms(), compositionMetadata(), ...frameRates()];
}
