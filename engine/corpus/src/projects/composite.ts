/**
 * Compositing parity cases for pm-render's Phase 3a compositor
 * (docs/engine/composite.md): every blend mode over opaque and transparent
 * stacks, parametric masks in every mode, shape, feather, rotation, and
 * animation, track mattes in the four modes with today's visibility rules,
 * styled groups as compositing boundaries, precomps, adjustment layers
 * without a rendering effect, and output scaling, built from the layer
 * types the native renderer draws (solids, images, precomps, groups, the 3D
 * primitive) so the comparison isolates compositing.
 */
import { newProject, type Builder, type Doc, type Layer } from '../doc';
import { ref } from '../media';
import { GRADIENT, project, type CorpusProject } from './common';

const NONE = { type: 'none', angle: 0, stops: [{ id: 'stop-1', color: '#000000', position: 0 }] };
const MODES = ['normal', 'add', 'screen', 'multiply', 'overlay', 'softlight', 'difference', 'lighten', 'darken'];

/** A base for one cell: a photo with a translucent band across it. */
function cellBase(b: Builder, x: number, y: number, w: number, h: number, translucent: boolean): void {
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'stretch', w, h }, p: { 'position.x': x + w / 2, 'position.y': y + h / 2, opacity: translucent ? 70 : 100 } }));
  b.add(b.layer('solid', { name: 'Band', d: { color: '#20406080', w, h: h / 3 }, p: { 'position.x': x, 'position.y': y + h / 3 } }));
}

/** The blended content of one cell: a solid ramp of colors and an image with soft alpha. */
function cellTop(b: Builder, mode: string, x: number, y: number, w: number, h: number, opacity: number): void {
  const colors = ['#FF6B1A', '#4C8DFF', '#E8E2CF', '#101010', '#7F7F7F'];
  colors.forEach((color, i) => b.add(b.layer('solid', {
    name: `${mode} ${color}`, blend: mode,
    d: { color, w: w / colors.length, h: h * .45 }, p: { 'position.x': x + i * w / colors.length, 'position.y': y + 4, opacity },
  })));
  b.add(b.layer('image', {
    name: `${mode} alpha image`, blend: mode,
    d: { asset: ref('corpus-gray-alpha.png'), fit: 'stretch', w: w * .8, h: h * .45 },
    p: { 'position.x': x + w / 2, 'position.y': y + h * .72, rotation: 7, opacity },
  }));
}

function blendModes(alpha: boolean): CorpusProject {
  const b = newProject(alpha ? 'composite/blend-modes-alpha' : 'composite/blend-modes', {
    name: alpha ? 'Blend modes over transparency' : 'Blend modes', backgroundFill: alpha ? NONE : GRADIENT, w: 480, h: 300,
  });
  MODES.forEach((mode, index) => {
    const w = 150, h = 92, x = 8 + (index % 3) * 158, y = 8 + Math.floor(index / 3) * 98;
    cellBase(b, x, y, w, h, alpha);
    cellTop(b, mode, x, y, w, h, index % 2 ? 80 : 100);
  });
  return project(b, {
    group: 'Compositing', title: alpha ? 'Nine blend modes over a translucent stack' : 'Nine blend modes',
    covers: ['blend modes: ' + MODES.join(', '), 'blend with layer opacity', 'blend over partial alpha', 'blend of soft-alpha images'],
    samples: [0], alpha,
  });
}

function blendAnimated(): CorpusProject {
  const b = newProject('composite/blend-animated', { name: 'Animated and unknown blend modes', backgroundFill: GRADIENT, w: 320, h: 180 });
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 320, h: 180 }, p: { 'position.x': 160, 'position.y': 90 } }));
  b.add(b.layer('solid', {
    name: 'Stepped blend', blend: b.ch('normal', [[0, 'normal', { hold: true }], [1, 'multiply', { hold: true }], [2, 'screen', { hold: true }]]),
    d: { color: '#4C8DFF', w: 140, h: 120 }, p: { 'position.x': 20, 'position.y': 30, opacity: 90 },
  }));
  b.add(b.layer('solid', { name: 'Add at 60%', blend: 'add', d: { color: '#FF6B1A', w: 100, h: 60 }, p: { 'position.x': 190, 'position.y': 20, opacity: 60 } }));
  b.add(b.layer('solid', { name: 'Difference', blend: 'difference', d: { color: '#E8E2CF', w: 120, h: 70 }, p: { 'position.x': 170, 'position.y': 95, rotation: b.ch(0, [[0, 0], [2.5, 30]]) } }));
  return project(b, { group: 'Compositing', title: 'Keyframed and unknown blend modes', covers: ['keyframed blend mode (hold steps)', 'blend with opacity', 'rotated blend layer edges'], samples: [.5, 1.5, 2.5] });
}

function masks(): CorpusProject {
  const b = newProject('composite/masks', { name: 'Parametric masks', backgroundFill: GRADIENT, w: 480, h: 300, dur: 3 });
  const photo = (name: string, x: number, y: number, extra: Doc = {}): Layer => b.add(b.layer('image', {
    name, d: { asset: ref('corpus-photo.tiff'), fit: 'stretch', w: 140, h: 84 }, p: { 'position.x': x, 'position.y': y, ...extra },
  }));
  // Row 1: shapes, feather, and hard edges.
  photo('Hard rect', 80, 52).masks.push(b.mask('rect', { x: 0, y: 0, w: 100, h: 60, feather: 0 }));
  photo('Feathered ellipse', 240, 52).masks.push(b.mask('ellipse', { x: 10, y: 0, w: 110, h: 70, feather: 30 }));
  photo('Rotated rect, subtract', 400, 52).masks.push(
    b.mask('rect', { x: 0, y: 0, w: 120, h: 70, feather: 6 }),
    b.mask('rect', { x: 10, y: 5, w: 50, h: 30, rotation: 35, feather: 0 }, { mode: 'subtract' }),
  );
  // Row 2: subtract only, negative size, tiny mask, feather below the threshold.
  photo('Subtract only', 80, 150).masks.push(b.mask('ellipse', { x: 0, y: 0, w: 60, h: 40, feather: 12 }, { mode: 'subtract' }));
  photo('Negative size', 240, 150).masks.push(b.mask('rect', { x: -20, y: 0, w: -70, h: -50, feather: 3 }), b.mask('rect', { x: 40, y: 0, w: .005, h: 50, feather: 0 }));
  photo('Feather at the threshold', 400, 150, { rotation: -12, 'scale.x': 110, skew: 14 }).masks.push(b.mask('ellipse', { x: 0, y: 0, w: 90, h: 64, feather: .01 }));
  // Row 3: animation and the 8-mask limit.
  photo('Animated', 80, 248).masks.push(
    b.mask('ellipse', { x: b.ch(-40, [[0, -40], [3, 40]]), y: 0, w: b.ch(40, [[0, 40], [3, 110]]), h: 60, rotation: b.ch(0, [[0, 0], [3, 90]]), feather: b.ch(0, [[0, 0], [3, 40]]) }),
    b.mask('rect', { x: 0, y: 0, w: 30, h: 80, feather: 0 }, { mode: b.ch('add', [[0, 'add', { hold: true }], [1.5, 'subtract', { hold: true }]]), on: b.ch(true, [[0, true, { hold: true }], [1, false, { hold: true }], [2, true, { hold: true }]]) }),
  );
  const many = b.add(b.layer('solid', { name: 'Ten masks', d: { color: '#FF6B1A', w: 150, h: 84 }, p: { 'position.x': 165, 'position.y': 206 } }));
  for (let i = 0; i < 10; i++) many.masks.push(b.mask(i % 2 ? 'ellipse' : 'rect', { x: 10 + i * 14, y: 42 + (i % 2 ? -18 : 18), w: 12, h: 12, feather: 2 }));
  photo('Disabled mask', 400, 248, { opacity: 70 }).masks.push(b.mask('rect', { x: 0, y: 0, w: 20, h: 20 }, { on: false }), b.mask('ellipse', { x: 0, y: 0, w: 120, h: 60, feather: 18 }));
  return project(b, {
    group: 'Masks and mattes', title: 'Parametric masks on images and solids',
    covers: ['rect and ellipse masks', 'hard and feathered edges', 'feather at the 0.01 threshold', 'rotation', 'add and subtract', 'subtract-only coverage', 'negative and tiny sizes', 'masks on rotated, scaled, and skewed layers', 'animated mask channels', 'animated enable and mode', 'more than 8 masks', 'disabled masks', 'masks with layer opacity'],
    samples: [0, 1.25, 2.5], alpha: true,
  });
}

function mattes(): CorpusProject {
  const b = newProject('composite/mattes', { name: 'Track mattes', backgroundFill: GRADIENT, w: 480, h: 300, dur: 3 });
  const modes = ['alpha', 'alpha-inverted', 'luma', 'luma-inverted'];
  modes.forEach((mode, i) => {
    const x = 60 + i * 120, y = 60;
    const target = b.add(b.layer('image', { name: `Target ${mode}`, d: { asset: ref('corpus-photo.tiff'), fit: 'stretch', w: 110, h: 100 }, p: { 'position.x': x, 'position.y': y } }));
    // The source: a soft-alpha gray image (luma and alpha both vary), switched off, as the editor leaves it.
    const source = b.add(b.layer('image', { name: `Matte ${mode}`, on: false, d: { asset: ref('corpus-gray-alpha.png'), fit: 'stretch', w: 100, h: 90 }, p: { 'position.x': x, 'position.y': y, rotation: b.ch(-8, [[0, -8], [3, 8]]) } }));
    target.matteSource = source.id; target.matteMode = b.ch(mode);
  });
  // A visible solid source with opacity and its own mask; the target is a solid.
  const s1 = b.add(b.layer('solid', { name: 'Masked matte', d: { color: '#FFFFFF', w: 100, h: 80 }, p: { 'position.x': 10, 'position.y': 130, opacity: 70 } }));
  s1.masks.push(b.mask('ellipse', { x: 50, y: 40, w: 90, h: 70, feather: 20 }));
  const t1 = b.add(b.layer('solid', { name: 'Target of masked matte', d: { color: '#3FCF8E', w: 110, h: 100 }, p: { 'position.x': 5, 'position.y': 125 } }));
  t1.matteSource = s1.id; t1.matteMode = b.ch('alpha');
  // A nested matte: the source is itself matted.
  const inner = b.add(b.layer('solid', { name: 'Inner matte', d: { color: '#FFFFFF', w: 60, h: 90 }, p: { 'position.x': b.ch(130, [[0, 130], [3, 190]]), 'position.y': 125 } }));
  const outer = b.add(b.layer('image', { name: 'Outer matte (matted)', d: { asset: ref('still-alpha.png'), fit: 'stretch', w: 110, h: 100 }, p: { 'position.x': 185, 'position.y': 175 } }));
  outer.matteSource = inner.id; outer.matteMode = b.ch('alpha');
  const t2 = b.add(b.layer('image', { name: 'Target of nested matte', d: { asset: ref('corpus-photo.tiff'), fit: 'stretch', w: 120, h: 100 }, p: { 'position.x': 185, 'position.y': 175 } }));
  t2.matteSource = outer.id; t2.matteMode = b.ch('alpha', [[0, 'alpha', { hold: true }], [1.5, 'luma-inverted', { hold: true }]]);
  // A matte with a blend mode, a matte that is out of its time span, a cycle.
  const s3 = b.add(b.layer('solid', { name: 'Blended matte', blend: 'multiply', d: { color: '#C0C0C0', w: 70, h: 70 }, p: { 'position.x': 265, 'position.y': 140 } }));
  const t3 = b.add(b.layer('solid', { name: 'Target of blended matte', d: { color: '#F2C14E', w: 90, h: 90 }, p: { 'position.x': 255, 'position.y': 130 } }));
  t3.matteSource = s3.id; t3.matteMode = b.ch('luma');
  const late = b.add(b.layer('solid', { name: 'Late matte', from: 2, dur: 1, d: { color: '#FFFFFF', w: 50, h: 50 }, p: { 'position.x': 370, 'position.y': 140 } }));
  const t4 = b.add(b.layer('solid', { name: 'Target of late matte', d: { color: '#A73366', w: 90, h: 70 }, p: { 'position.x': 360, 'position.y': 130 } }));
  t4.matteSource = late.id; t4.matteMode = b.ch('alpha-inverted');
  const a = b.add(b.layer('solid', { name: 'Cycle A', d: { color: '#FF6B1A', w: 60, h: 50 }, p: { 'position.x': 20, 'position.y': 240 } }));
  const c = b.add(b.layer('solid', { name: 'Cycle B', d: { color: '#E8E2CF', w: 60, h: 50 }, p: { 'position.x': 50, 'position.y': 230 } }));
  a.matteSource = c.id; c.matteSource = a.id; a.matteMode = b.ch('alpha'); c.matteMode = b.ch('luma');
  return project(b, {
    group: 'Masks and mattes', title: 'Track mattes: modes and visibility rules',
    covers: ['track matte alpha, alpha-inverted, luma, luma-inverted', 'hidden matte source', 'matte source opacity and masks', 'nested mattes', 'keyframed matte mode', 'matte with a blend mode', 'matte source out of its time span', 'matte cycle (depth-limited)'],
    samples: [0, 1.25, 2.5], alpha: true,
  });
}

function groups(): CorpusProject {
  const b = newProject('composite/groups', { name: 'Styled groups', backgroundFill: GRADIENT, w: 480, h: 300 });
  const member = (name: string, color: string, x: number, y: number, group: string, extra: Doc = {}) =>
    b.layer('solid', { name, group, d: { color, w: 80, h: 70 }, p: { 'position.x': x, 'position.y': y, opacity: 85 }, ...extra });
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 480, h: 300 }, p: { 'position.x': 240, 'position.y': 150 } }));
  // Blend mode and opacity on a group: the members composite first.
  const screen = b.layer('group', { name: 'Screen at 60%', blend: 'screen', p: { opacity: 60 } });
  b.add(member('S1', '#FF6B1A', 20, 20, screen.id)); b.add(member('S2', '#4C8DFF', 60, 50, screen.id)); b.add(screen);
  // Masked group.
  const masked = b.layer('group', { name: 'Masked group' });
  masked.masks.push(b.mask('ellipse', { x: 230, y: 70, w: 160, h: 100, feather: 16 }));
  b.add(member('M1', '#3FCF8E', 170, 20, masked.id)); b.add(member('M2', '#F2C14E', 220, 60, masked.id, { blend: 'multiply' })); b.add(masked);
  // Matted group: a hidden source.
  const source = b.add(b.layer('image', { name: 'Group matte', on: false, d: { asset: ref('still-alpha.png'), fit: 'stretch', w: 150, h: 110 }, p: { 'position.x': 400, 'position.y': 75 } }));
  const matted = b.layer('group', { name: 'Matted group', matteSource: source.id, matteMode: 'luma' });
  b.add(member('T1', '#A73366', 330, 20, matted.id)); b.add(member('T2', '#E8E2CF', 380, 60, matted.id)); b.add(matted);
  // Nested boundaries: a difference group inside a group at 70% with an overlay member.
  const outer = b.layer('group', { name: 'Outer 70%', p: { opacity: 70 } });
  const inner = b.layer('group', { name: 'Inner difference', group: outer.id, blend: 'difference' });
  b.add(member('I1', '#FFFFFF', 40, 170, inner.id)); b.add(member('I2', '#4C8DFF', 90, 200, inner.id, { blend: 'overlay' })); b.add(inner);
  b.add(member('O1', '#FF6B1A', 150, 180, outer.id)); b.add(outer);
  // A transform-only group with blended members (flattened: members blend with the stack).
  const plain = b.layer('group', { name: 'Transform-only', p: { rotation: 6 } });
  b.add(member('P1', '#3FCF8E', 300, 170, plain.id, { blend: 'softlight' })); b.add(member('P2', '#F2C14E', 350, 200, plain.id, { blend: 'darken' })); b.add(plain);
  return project(b, {
    group: 'Compositing', title: 'Styled groups as compositing boundaries',
    covers: ['group blend mode and opacity', 'masked group', 'matted group with a hidden source', 'nested boundaries', 'blended members inside a boundary', 'transform-only group with blended members'],
    samples: [0], alpha: true,
  });
}

function precomps(): CorpusProject {
  const b = newProject('composite/precomps', { name: 'Compositing in precomps', backgroundFill: GRADIENT, w: 480, h: 270 });
  const inner = b.composition({ name: 'Inner', w: 200, h: 120, dur: 3, bg: '#000000' });
  b.within(inner, () => {
    b.add(b.layer('image', { name: 'Inner photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 200, h: 120 }, p: { 'position.x': 100, 'position.y': 60 } }));
    const src = b.add(b.layer('solid', { name: 'Inner matte', on: false, d: { color: '#FFFFFF', w: 120, h: 80 }, p: { 'position.x': 40, 'position.y': 20, rotation: 10 } }));
    const t = b.add(b.layer('solid', { name: 'Inner matted overlay', blend: 'overlay', d: { color: '#FF6B1A', w: 200, h: 120 } }));
    t.matteSource = src.id; t.matteMode = b.ch('alpha');
  });
  const p1 = b.add(b.layer('precomp', { name: 'Precomp multiply', blend: 'multiply', d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 20, 'position.y': 20 } }));
  p1.masks.push(b.mask('ellipse', { x: 100, y: 60, w: 180, h: 110, feather: 20 }));
  b.add(b.layer('precomp', { name: 'Precomp screen scaled', blend: 'screen', d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 250, 'position.y': 30, 'scale.x': 110, 'scale.y': 130, rotation: -5, opacity: 85 } }));
  const src = b.add(b.layer('precomp', { name: 'Precomp as matte', on: false, d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 140, 'position.y': 140 } }));
  const t = b.add(b.layer('solid', { name: 'Matted by a precomp', d: { color: '#4C8DFF', w: 220, h: 120 }, p: { 'position.x': 130, 'position.y': 140 } }));
  t.matteSource = src.id; t.matteMode = b.ch('luma');
  // An adjustment layer with no effect that renders is a no-op.
  b.add(b.layer('adjustment', { name: 'Empty adjustment', p: { opacity: 50 } }));
  return project(b, {
    group: 'Compositing', title: 'Blend modes, masks, and mattes with precomps',
    covers: ['precomp with blend and mask', 'blend and matte inside a precomp', 'precomp as a luma matte', 'adjustment layer without a rendering effect'],
    samples: [0, 1.5], alpha: true,
  });
}

function scaled(): CorpusProject {
  const b = newProject('composite/scaled-capture', { name: 'Compositing at a different output size', backgroundFill: GRADIENT, w: 480, h: 270 });
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 480, h: 270 }, p: { 'position.x': 240, 'position.y': 135 } }));
  const m = b.add(b.layer('solid', { name: 'Masked overlay', blend: 'overlay', d: { color: '#4C8DFF', w: 300, h: 180 }, p: { 'position.x': 40, 'position.y': 30, rotation: 4 } }));
  m.masks.push(b.mask('ellipse', { x: 150, y: 90, w: 260, h: 150, feather: 30 }));
  const src = b.add(b.layer('image', { name: 'Matte', on: false, d: { asset: ref('corpus-gray-alpha.png'), fit: 'stretch', w: 181, h: 103 }, p: { 'position.x': 360, 'position.y': 190 } }));
  const t = b.add(b.layer('solid', { name: 'Matted', d: { color: '#FF6B1A', w: 200, h: 120 }, p: { 'position.x': 260, 'position.y': 130 } }));
  t.matteSource = src.id; t.matteMode = b.ch('luma-inverted');
  return project(b, { group: 'Compositing', title: 'Masks, mattes, and blends at another output scale', covers: ['output scale with masks', 'output scale with mattes', 'output scale with blend modes'], samples: [0], size: [397, 223] });
}

function threeD(blend: boolean, mask: boolean): CorpusProject {
  const slug = `composite/3d-${[blend ? 'blend' : '', mask ? 'mask' : ''].filter(Boolean).join('-')}`;
  const b = newProject(slug, { name: '3D layer composited', backgroundFill: GRADIENT, w: 480, h: 270 });
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 480, h: 270 }, p: { 'position.x': 240, 'position.y': 135 } }));
  const object = b.add(b.object3d({ source: { primitive: 'sphere', parameters: { radius: 1, segments: 48 } }, material: { p: { color: '#4C8DFF', roughness: .5, metalness: .1 } } }, { name: 'Sphere', ...(blend ? { blend: 'multiply' } : {}) }));
  // Layer pixels of a 3D layer map through its projected plane (`is3DLayer`).
  if (mask) object.masks.push(b.mask('ellipse', { x: 200, y: 135, w: 160, h: 300, feather: 30 }));
  b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 2.5, color: '#FFFFFF' } }, { position: [-4, 6, 4] }));
  b.add(b.camera3d({ camera: { p: { targetX: 0, targetY: 0, targetZ: 0 } }, environment: { shadows: false } }, { position: [0, 1, 4] }));
  return project(b, { group: 'Compositing', title: 'A composited 3D layer', covers: [blend ? 'blend mode on a 3D layer' : '', mask ? 'mask on a 3D layer (projected plane inverse)' : ''].filter(Boolean), samples: [0] });
}

export function compositeProjects(): CorpusProject[] {
  return [blendModes(false), blendModes(true), blendAnimated(), masks(), mattes(), groups(), precomps(), scaled(), threeD(true, false), threeD(false, true), threeD(true, true)];
}
