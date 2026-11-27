import { blendModes, matteModes } from '../catalog';
import { newProject, type Builder, type Doc } from '../doc';
import { ref } from '../media';
import { GRADIENT, project, testCard, type CorpusProject } from './common';

function blendProject(alpha: boolean): CorpusProject {
  const b = newProject(alpha ? 'compositing/blend-modes-alpha' : 'compositing/blend-modes', {
    name: alpha ? 'Blend modes over transparency' : 'Blend modes',
    backgroundFill: alpha ? { type: 'none', angle: 0, stops: [{ id: 'stop-1', color: '#000000', position: 0 }] } : GRADIENT,
  });
  b.add(b.layer('image', { name: 'Artwork', d: { asset: ref('corpus-art.svg'), fit: 'cover', w: 640, h: 360 }, p: alpha ? { opacity: 75 } : {} }));
  if (blendModes.length !== 9) throw new Error(`Expected 9 blend modes, found ${blendModes.length}`);
  blendModes.forEach((mode, index) => {
    const column = index % 3, row = Math.floor(index / 3);
    b.add(b.layer('shape', {
      name: `Blend ${mode}`, blend: mode,
      d: { shape: 'rect', color: ['#C08040', '#40A0C0', '#E0E0E0'][row]!, w: 150, h: 84, radius: 10 },
      p: { 'position.x': 120 + column * 200, 'position.y': 64 + row * 116, opacity: 85 },
    }));
  });
  b.add(b.layer('text', {
    name: 'Animated blend', blend: b.ch('normal', [[0, 'normal', { hold: true }], [1, 'difference', { hold: true }], [2, 'screen', { hold: true }]]),
    d: { text: 'BLEND', size: 60, weight: 800, color: '#FF6B1A' }, p: { 'position.x': 320, 'position.y': 300 },
  }));
  return project(b, {
    group: 'Compositing', title: alpha ? 'Nine blend modes over transparency' : 'Nine blend modes',
    covers: ['blend modes: ' + blendModes.join(', '), 'keyframed blend mode (string steps)', 'opacity'],
    samples: [.5, 1.5, 2.5], alpha,
  });
}

function maskProjects(): CorpusProject[] {
  const parametric = newProject('masks/parametric', { name: 'Parametric masks', backgroundFill: GRADIENT });
  const [art, , , , , , title] = testCard(parametric);
  art!.masks.push(
    parametric.mask('ellipse', { x: -40, y: 0, w: parametric.ch(160, [[0, 160], [2.5, 360]]), h: 220, feather: 30 }),
    parametric.mask('rect', { x: 60, y: -30, w: 90, h: 70, rotation: 25, feather: 0 }, { mode: 'subtract' }),
    parametric.mask('rect', { x: 120, y: 60, w: 120, h: 50, feather: parametric.ch(0, [[0, 0], [2.5, 60]]) }, { on: parametric.ch(true, [[0, true], [1.2, false], [2, true]]) }),
  );
  title!.masks.push(parametric.mask('rect', { x: 0, y: 0, w: 140, h: 120, feather: 8 }, { mode: parametric.ch('add', [[0, 'add'], [1.5, 'subtract']]) }));
  const many = newProject('masks/more-than-eight', { name: 'More than eight masks', backgroundFill: GRADIENT });
  const solid = many.add(many.layer('solid', { name: 'Masked solid', d: { color: '#FF6B1A' } }));
  for (let i = 0; i < 10; i++) solid.masks.push(many.mask(i % 2 ? 'ellipse' : 'rect', { x: 50 + i * 58, y: 180 + (i % 2 ? -50 : 50), w: 46, h: 46, feather: 2 }));
  const paths = newProject('masks/path', { name: 'Bezier path masks', backgroundFill: GRADIENT });
  const [pathArt] = testCard(paths);
  const blob = paths.path('Blob', [[-150, -40, 0, 0, 60, -80], [60, -110, -40, 0, 80, 30], [160, 30, 0, -60, -20, 90], [-30, 120, 80, 0, -100, 0]], {
    closed: true, rotation: paths.ch(0, [[0, 0], [2.5, 25]]),
  });
  blob.vertices[2]!.p.x = paths.ch(160, [[0, 160], [2.5, 220]]);
  pathArt!.masks.push(paths.mask('rect', { feather: 12 }, { path: blob }));
  pathArt!.masks.push(paths.mask('ellipse', { x: 20, y: 0, w: 70, h: 70, feather: 0 }, { mode: 'subtract' }));
  return [
    project(parametric, { group: 'Masks and mattes', title: 'Parametric masks', covers: ['rect and ellipse masks', 'feather', 'rotation', 'add and subtract', 'animated mask channels', 'animatable mask enable', 'animatable mask mode'], samples: [0, 1.25, 2.5], alpha: true }),
    project(many, { group: 'Masks and mattes', title: 'Ten masks on one layer', covers: ['more than 8 parametric masks (GPU path drops extras)'], samples: [0] }),
    project(paths, { group: 'Masks and mattes', title: 'Bezier path masks', covers: ['bezier path masks', 'animated path vertices', 'path plus parametric masks'], samples: [0, 1.25, 2.5] }),
  ];
}

function matteProjects(): CorpusProject[] {
  const b = newProject('mattes/modes', { name: 'Track matte modes', backgroundFill: GRADIENT });
  if (matteModes.length !== 4) throw new Error(`Expected 4 matte modes, found ${matteModes.length}`);
  matteModes.forEach((mode, index) => {
    const cx = 160 + (index % 2) * 320, cy = 90 + Math.floor(index / 2) * 180;
    const target = b.add(b.layer('image', { name: `Target ${mode}`, d: { asset: ref('corpus-art.svg'), fit: 'cover', w: 300, h: 168 }, p: { 'position.x': cx, 'position.y': cy } }));
    const source = b.add(b.layer('text', {
      name: `Matte ${mode}`, d: { text: 'MATTE', size: 64, weight: 900, color: index % 2 ? '#808080' : '#FFFFFF' },
      p: { 'position.x': cx, 'position.y': cy, rotation: b.ch(-10, [[0, -10], [2.5, 10]]) },
    }));
    target.matteSource = source.id; target.matteMode = b.ch(mode);
  });
  const animated = newProject('mattes/nested-and-animated', { name: 'Nested and animated mattes', backgroundFill: GRADIENT });
  const inner = animated.add(animated.layer('shape', { name: 'Inner matte', d: { shape: 'ellipse', w: 260, h: 260, color: '#FFFFFF' }, p: { 'position.x': animated.ch(220, [[0, 220], [2.5, 420]]) } }));
  const outer = animated.add(animated.layer('shape', { name: 'Outer matte (itself matted)', d: { shape: 'star', w: 340, h: 340, color: '#FFFFFF', points: 6 }, matteSource: inner.id, matteMode: 'alpha' }));
  const target = animated.add(animated.layer('image', { name: 'Target', d: { asset: ref('corpus-art.svg'), fit: 'cover', w: 640, h: 360 } }));
  target.matteSource = outer.id;
  target.matteMode = animated.ch('alpha', [[0, 'alpha'], [1.25, 'luma-inverted']]);
  const cycle = newProject('mattes/cycle', { name: 'Matte cycle', backgroundFill: GRADIENT });
  const a = cycle.add(cycle.layer('shape', { name: 'A', d: { color: '#FF6B1A', w: 200, h: 200 }, p: { 'position.x': 250 } }));
  const c = cycle.add(cycle.layer('shape', { name: 'B', d: { shape: 'ellipse', color: '#4C8DFF', w: 200, h: 200 }, p: { 'position.x': 390 } }));
  a.matteSource = c.id; c.matteSource = a.id;
  return [
    project(b, { group: 'Masks and mattes', title: 'Four track matte modes', covers: matteModes.map(mode => `track matte ${mode}`), samples: [0, 1.25, 2.5] }),
    project(animated, { group: 'Masks and mattes', title: 'Matte nesting and keyframed matte mode', covers: ['matte nesting', 'keyframed matte mode'], samples: [0, 1, 2], alpha: true }),
    project(cycle, { group: 'Masks and mattes', title: 'Matte cycle', covers: ['matte cycle (depth-limited at render)'], samples: [0], notes: 'The editor rejects cycles; a loaded document can still contain one.' }),
  ];
}

function adjustmentProject(): CorpusProject {
  const b = newProject('compositing/adjustment', { name: 'Adjustment layers', backgroundFill: GRADIENT });
  testCard(b);
  const adjustment = b.add(b.layer('adjustment', {
    name: 'Adjust', from: .5, dur: 2,
    fx: [b.effect('color', { saturation: -80, contrast: 30 }), b.effect('blur', { amount: 6 })],
    p: { opacity: b.ch(100, [[.5, 100], [2.5, 40]]) },
  }));
  adjustment.masks.push(b.mask('rect', { x: 320, y: 180, w: 400, h: 240, feather: 20 }));
  b.add(b.layer('shape', { name: 'Above adjustment', d: { shape: 'star', w: 120, h: 120, color: '#F2C14E' }, p: { 'position.x': 560, 'position.y': 70 } }));
  return project(b, { group: 'Compositing', title: 'Adjustment layer with effects, mask, and opacity', covers: ['adjustment layer', 'adjustment effects on the accumulated stack', 'adjustment mask', 'adjustment opacity', 'partial duration'], samples: [.25, 1, 2], alpha: true });
}

function motionBlurProjects(): CorpusProject[] {
  const make = (slug: string, shutter: number) => {
    const b = newProject(slug, { name: `Motion blur shutter ${shutter}`, backgroundFill: GRADIENT, shutter });
    b.add(b.layer('shape', { name: 'Fast slide', mblur: true, d: { w: 80, h: 80, color: '#FF6B1A', radius: 8 }, p: { 'position.x': b.ch(60, [[0, 60], [1.5, 580]]), 'position.y': 90 } }));
    b.add(b.layer('shape', { name: 'Spinner', mblur: true, d: { shape: 'rect', w: 200, h: 24, color: '#4C8DFF' }, p: { 'position.x': 320, 'position.y': 200, rotation: b.ch(0, [[0, 0], [3, 720]]) } }));
    b.add(b.layer('text', { name: 'Zooming text', mblur: b.ch(true, [[0, true], [2, false]]), d: { text: 'BLUR', size: 70, weight: 800, color: '#E8E2CF' }, p: { 'position.x': 320, 'position.y': 300, 'scale.x': b.ch(50, [[0, 50], [2.5, 200]]), 'scale.y': b.ch(50, [[0, 50], [2.5, 200]]) } }));
    b.add(b.layer('shape', { name: 'No blur reference', d: { shape: 'ellipse', w: 60, h: 60, color: '#3FCF8E' }, p: { 'position.x': b.ch(580, [[0, 580], [1.5, 60]]), 'position.y': 140 } }));
    return b;
  };
  return [
    project(make('compositing/motion-blur', .5), { group: 'Compositing', title: 'Motion blur, 180° shutter', covers: ['motion blur', 'translation, rotation, and scale blur', 'keyframed motion blur switch'], samples: [.5, 1, 2.2], alpha: true }),
    project(make('compositing/motion-blur-shutter-360', 1), { group: 'Compositing', title: 'Motion blur, 360° shutter', covers: ['composition shutter'], samples: [.5, 1] }),
  ];
}

function backgroundProjects(): CorpusProject[] {
  const fills: Array<[string, Doc]> = [
    ['none', { type: 'none', angle: 0, stops: [{ id: 'stop-1', color: '#000000', position: 0 }] }],
    ['solid', { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#275BA3', position: 0 }] }],
    ['linear-8-stops', { type: 'linear', angle: -60, stops: ['#0C0D12', '#1B2A4A', '#4C8DFF', '#3FCF8E', '#F2C14E', '#FF6B1A', '#A73366', '#E8E2CF'].map((color, i) => ({ id: `stop-${i + 1}`, color, position: Math.round(i * 100 / 7) })) }],
    ['radial', { type: 'radial', angle: 0, stops: [{ id: 'stop-1', color: '#FFD66B', position: 0 }, { id: 'stop-2', color: '#1B2A4A', position: 70 }, { id: 'stop-3', color: '#0C0D12', position: 100 }] }],
    ['radial-angle', { type: 'radial', angle: 45, stops: [{ id: 'stop-1', color: '#3FCF8E', position: 10 }, { id: 'stop-2', color: '#0C0D12', position: 90 }] }],
  ];
  return fills.map(([name, fill]) => {
    const b = newProject(`backgrounds/${name}`, { name: `Background ${name}`, backgroundFill: fill });
    b.add(b.layer('shape', { name: 'Marker', d: { shape: 'ellipse', w: 60, h: 60, color: '#FFFFFF' }, p: { 'position.x': 590, 'position.y': 50, opacity: 60 } }));
    return project(b, { group: 'Composition', title: `Background ${name}`, covers: [`background ${name}`], samples: [0], alpha: name === 'none' });
  });
}

function groupProjects(): CorpusProject[] {
  const b = newProject('groups/transforms-and-styles', { name: 'Groups', backgroundFill: GRADIENT });
  const member = (bb: Builder, name: string, color: string, x: number, y: number, group: string) =>
    bb.layer('shape', { name, group, d: { w: 90, h: 90, color, radius: 12 }, p: { 'position.x': x, 'position.y': y, opacity: 85 } });
  const plain = b.layer('group', { name: 'Transform-only group', p: { 'position.x': b.ch(0, [[0, 0], [2.5, 60]]), rotation: 8 } });
  const styled = b.layer('group', { name: 'Styled group (compositing boundary)', blend: 'screen', p: { opacity: 60 } });
  const effected = b.layer('group', { name: 'Group with effects', fx: [b.effect('blur', { amount: 8 }), b.effect('shadow', { distance: 12 })] });
  const nested = b.layer('group', { name: 'Nested group', group: effected.id, p: { 'scale.x': 80, 'scale.y': 80, 'position.x': 470, 'position.y': 260, 'anchor.x': 470, 'anchor.y': 260 } });
  // Children first so each group header sits above its members (index 0 is the top).
  b.add(member(b, 'Plain A', '#FF6B1A', 90, 90, plain.id)); b.add(member(b, 'Plain B', '#4C8DFF', 150, 120, plain.id)); b.add(plain);
  b.add(member(b, 'Styled A', '#3FCF8E', 300, 90, styled.id)); b.add(member(b, 'Styled B', '#F2C14E', 350, 130, styled.id)); b.add(styled);
  b.add(member(b, 'Nested A', '#A73366', 440, 250, nested.id)); b.add(member(b, 'Nested B', '#E8E2CF', 500, 280, nested.id)); b.add(nested);
  b.add(member(b, 'Effect A', '#FF6B1A', 120, 270, effected.id)); b.add(effected);
  const solo = newProject('groups/solo-and-hidden', { name: 'Solo and hidden layers', backgroundFill: GRADIENT });
  const card = testCard(solo);
  card[1]!.solo = true; card[6]!.solo = true; card[2]!.on = false;
  card[3]!.on = solo.ch(true, [[0, true], [1, false]]);
  return [
    project(b, { group: 'Compositing', title: 'Group transforms, styled groups, group effects, nesting', covers: ['group transform space', 'styled group compositing boundary', 'group effects', 'nested groups', 'group opacity and blend'], samples: [0, 1.25, 2.5], alpha: true }),
    project(solo, { group: 'Layers', title: 'Solo and hidden layers', covers: ['solo', 'visibility', 'animatable visibility'], samples: [0, 1.5] }),
  ];
}

export function compositingProjects(): CorpusProject[] {
  return [blendProject(false), blendProject(true), ...maskProjects(), ...matteProjects(), adjustmentProject(), ...motionBlurProjects(), ...backgroundProjects(), ...groupProjects()];
}
