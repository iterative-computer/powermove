/**
 * Render parity cases for pm-render's Phase 2 renderer (docs/engine/pm-render.md):
 * every feature the minimal renderer supports, alone and combined, with the
 * edge cases today's compositor has (shared quad edges, hard UV edges under
 * rotation, minified images without mipmaps, precomp target rounding, group
 * compositing boundaries, solo, background fills, transparent output, scaled
 * captures), plus the 3D primitive lit by a sun without shadows.
 */
import { newProject, type Builder, type Doc } from '../doc';
import { ref } from '../media';
import { GRADIENT, project, type CorpusProject } from './common';

const DARK = { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#14161C', position: 0 }] };

function transforms(): CorpusProject {
  const b = newProject('render/transforms', { name: 'Transforms and sampling', backgroundFill: GRADIENT });
  b.add(b.layer('solid', { name: 'Back', d: { color: '#1F2433' } }));
  // Abutting quads: pixel centers on shared edges (y = 67.5) go to one quad.
  b.add(b.layer('solid', { name: 'Top half', d: { color: '#FF6B1A', w: 100, h: 90 }, p: { 'position.x': 10, 'position.y': -22.5 } }));
  b.add(b.layer('solid', { name: 'Bottom half', d: { color: '#3FCF8E', w: 100, h: 90 }, p: { 'position.x': 10, 'position.y': 67.5 } }));
  // Fractional positions, negative scale (flip), skew, anchor, rotation.
  b.add(b.layer('solid', { name: 'Fractional', d: { color: '#4C8DFF', w: 33.3, h: 21.7 }, p: { 'position.x': 130.25, 'position.y': 20.75, rotation: 17.5 } }));
  b.add(b.layer('solid', { name: 'Flipped', d: { color: '#F2C14E', w: 60, h: 40 }, p: { 'position.x': 250, 'position.y': 30, 'scale.x': -100, 'anchor.x': 30, 'anchor.y': 20 } }));
  b.add(b.layer('solid', { name: 'Skew and scale', d: { color: '#A73366', w: 70, h: 50 }, p: { 'position.x': 300, 'position.y': 60, skew: -25, 'scale.x': 130, 'scale.y': 70, opacity: 85 } }));
  // Parenting: a null drives a rotating chain.
  const root = b.add(b.layer('null', { name: 'Rig', p: { 'position.x': 470, 'position.y': 90, rotation: b.ch(-20, [[0, -20], [2, 40]]) } }));
  const arm = b.add(b.layer('solid', { name: 'Arm', parent: root.id, d: { color: '#E8E2CF', w: 120, h: 16 }, p: { 'position.x': 0, 'position.y': 0, 'anchor.y': 8 } }));
  b.add(b.layer('solid', { name: 'Hand', parent: arm.id, d: { color: '#7DD3FC', w: 30, h: 30 }, p: { 'position.x': 120, 'position.y': 8, 'anchor.x': 15, 'anchor.y': 15, rotation: 45, opacity: b.ch(100, [[0, 100], [2, 35]]) } }));
  // Images: rotated (bilinear edges), minified without mipmaps, magnified, every fit.
  b.add(b.layer('image', { name: 'Rotated image', d: { asset: ref('still-red.png'), fit: 'stretch', w: 160, h: 90 }, p: { 'position.x': 120, 'position.y': 230, rotation: 33, 'scale.x': 80, 'scale.y': 80 } }));
  b.add(b.layer('image', { name: 'Minified alpha image', d: { asset: ref('still-alpha.png'), fit: 'stretch', w: 160, h: 90 }, p: { 'position.x': 260, 'position.y': 200, 'scale.x': 37, 'scale.y': 37 } }));
  b.add(b.layer('image', { name: 'Magnified alpha image', d: { asset: ref('still-alpha.png'), fit: 'stretch', w: 160, h: 90 }, p: { 'position.x': 330, 'position.y': 290, 'scale.x': 230, 'scale.y': 230, rotation: -7 } }));
  b.add(b.layer('image', { name: 'Cover', d: { asset: ref('still.webp'), fit: 'cover', w: 80, h: 80 }, p: { 'position.x': 520, 'position.y': 230 } }));
  b.add(b.layer('image', { name: 'Contain', d: { asset: ref('still.webp'), fit: 'contain', w: 80, h: 80 }, p: { 'position.x': 600, 'position.y': 230 } }));
  b.add(b.layer('image', { name: 'Stretch', d: { asset: ref('still.webp'), fit: 'stretch', w: 80, h: 40 }, p: { 'position.x': 560, 'position.y': 320, rotation: b.ch(0, [[0, 0], [2, 180]]) } }));
  return project(b, { group: 'Render', title: 'Transforms and sampling', covers: ['abutting quads', 'fractional positions', 'negative scale', 'skew', 'parenting chain', 'opacity animation', 'image rotation and bilinear edges', 'minification without mipmaps', 'fit cover, contain, stretch'], samples: [0, 1, 2] });
}

function precomps(): CorpusProject {
  const b = newProject('render/precomps', { name: 'Precomps', backgroundFill: DARK, dur: 3 });
  const inner = b.composition({ name: 'Inner', w: 200, h: 120, dur: 3, bg: '#000000' });
  b.within(inner, () => {
    b.add(b.layer('solid', { name: 'Inner back', d: { color: '#2A2F45', w: 200, h: 120 } }));
    b.add(b.layer('solid', { name: 'Inner mover', d: { color: '#FF6B1A', w: 40, h: 40 }, p: { 'position.x': b.ch(0, [[0, 0], [3, 160]]), 'position.y': 40, rotation: b.ch(0, [[0, 0], [3, 90]]), 'anchor.x': 20, 'anchor.y': 20 } }));
    b.add(b.layer('image', { name: 'Inner image', d: { asset: ref('still-alpha.png'), fit: 'contain', w: 100, h: 60 }, p: { 'position.x': 140, 'position.y': 80 } }));
  });
  const middle = b.composition({ name: 'Middle', w: 300, h: 200, dur: 3, bg: '#000000' });
  b.within(middle, () => {
    b.add(b.layer('precomp', { name: 'Inner in middle', d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 10, 'position.y': 10, opacity: 80 } }));
    b.add(b.layer('precomp', { name: 'Inner offset', from: 1, d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 120, 'position.y': 90, 'scale.x': 60, 'scale.y': 60, rotation: 12 } }));
  });
  b.add(b.layer('precomp', { name: 'Middle', d: { comp: middle.id, w: 300, h: 200 }, p: { 'position.x': 20, 'position.y': 20 } }));
  b.add(b.layer('precomp', { name: 'Middle scaled up', d: { comp: middle.id, w: 300, h: 200 }, p: { 'position.x': 340, 'position.y': 60, 'scale.x': 95, 'scale.y': 140, rotation: -6, opacity: 90 } }));
  // A box larger than the composition: the target is sized from the box.
  b.add(b.layer('precomp', { name: 'Inner box stretched', d: { comp: inner.id, w: 333, h: 77 }, p: { 'position.x': 150, 'position.y': 270 } }));
  // Retimed: speed and trim.
  b.add(b.layer('precomp', { name: 'Inner fast', d: { comp: inner.id, w: 200, h: 120, speed: 2, trim: .25 }, p: { 'position.x': 470, 'position.y': 250, 'scale.x': 50, 'scale.y': 50 } }));
  return project(b, { group: 'Render', title: 'Precomps', covers: ['precomp nesting', 'precomp box size differs from its composition', 'precomp opacity and transform', 'precomp from offset', 'precomp speed and trim', 'image inside a precomp'], samples: [0, 1.2, 2.5], alpha: true });
}

function groups(): CorpusProject {
  const b = newProject('render/groups', { name: 'Groups', backgroundFill: DARK });
  // Overlapping members show whether the group composites as one image.
  const boundary = b.layer('group', { name: 'Group at 50% (compositing boundary)', p: { opacity: 50 } });
  b.add(b.layer('solid', { name: 'Member A', group: boundary.id, d: { color: '#FF6B1A', w: 120, h: 120 }, p: { 'position.x': 40, 'position.y': 40 } }));
  b.add(b.layer('solid', { name: 'Member B', group: boundary.id, d: { color: '#4C8DFF', w: 120, h: 120 }, p: { 'position.x': 100, 'position.y': 100 } }));
  b.add(boundary);
  const plain = b.layer('group', { name: 'Transform-only group', p: { 'position.x': b.ch(0, [[0, 0], [2.5, 40]]), rotation: 8 } });
  b.add(b.layer('solid', { name: 'Plain member A', group: plain.id, d: { color: '#FF6B1A', w: 120, h: 120 }, p: { 'position.x': 280, 'position.y': 40, opacity: 50 } }));
  b.add(b.layer('solid', { name: 'Plain member B', group: plain.id, d: { color: '#4C8DFF', w: 120, h: 120 }, p: { 'position.x': 340, 'position.y': 100, opacity: 50 } }));
  b.add(plain);
  // A boundary inside a boundary, with an animated opacity crossing 100%.
  const outer = b.layer('group', { name: 'Outer', p: { opacity: b.ch(100, [[0, 100], [2.5, 40]]) } });
  const nested = b.layer('group', { name: 'Nested at 70%', group: outer.id, p: { opacity: 70 } });
  b.add(b.layer('solid', { name: 'Nested member', group: nested.id, d: { color: '#3FCF8E', w: 100, h: 80 }, p: { 'position.x': 470, 'position.y': 50 } }));
  b.add(b.layer('solid', { name: 'Nested member 2', group: nested.id, d: { color: '#F2C14E', w: 100, h: 80 }, p: { 'position.x': 520, 'position.y': 90 } }));
  b.add(nested);
  b.add(b.layer('image', { name: 'Outer image', group: outer.id, d: { asset: ref('still-alpha.png'), fit: 'contain', w: 160, h: 90 }, p: { 'position.x': 540, 'position.y': 230 } }));
  b.add(outer);
  // Hidden layers and a hidden group.
  b.add(b.layer('solid', { name: 'Hidden', on: false, d: { color: '#FFFFFF', w: 640, h: 360 } }));
  const hidden = b.layer('group', { name: 'Hidden group', on: false });
  b.add(b.layer('solid', { name: 'In hidden group', group: hidden.id, d: { color: '#FFFFFF', w: 50, h: 50 }, p: { 'position.x': 10, 'position.y': 300 } }));
  b.add(hidden);
  // Layer timing: in and out points.
  b.add(b.layer('solid', { name: 'Late', from: 1.5, dur: 1, d: { color: '#E8E2CF', w: 60, h: 60 }, p: { 'position.x': 60, 'position.y': 260 } }));
  return project(b, { group: 'Render', title: 'Groups', covers: ['group compositing boundary (opacity)', 'transform-only group', 'nested boundaries', 'animated group opacity', 'hidden layers and groups', 'in and out points'], samples: [0, 1.25, 2.5], alpha: true });
}

function solo(): CorpusProject {
  const b = newProject('render/solo', { name: 'Solo', backgroundFill: DARK });
  b.add(b.layer('solid', { name: 'Not soloed', d: { color: '#FF6B1A', w: 200, h: 200 }, p: { 'position.x': 20, 'position.y': 20 } }));
  const g = b.layer('group', { name: 'Group with a soloed member' });
  b.add(b.layer('solid', { name: 'Soloed member', solo: true, group: g.id, d: { color: '#4C8DFF', w: 200, h: 200 }, p: { 'position.x': 220, 'position.y': 40 } }));
  b.add(b.layer('solid', { name: 'Unsoloed sibling', group: g.id, d: { color: '#3FCF8E', w: 100, h: 100 }, p: { 'position.x': 300, 'position.y': 200 } }));
  b.add(g);
  b.add(b.layer('solid', { name: 'Soloed', solo: true, d: { color: '#F2C14E', w: 120, h: 120 }, p: { 'position.x': 470, 'position.y': 120 } }));
  return project(b, { group: 'Render', title: 'Solo', covers: ['solo', 'solo inside a group'], samples: [0] });
}

function backgrounds(): CorpusProject[] {
  const fills: Array<[string, Doc]> = [
    ['linear-0', { type: 'linear', angle: 0, stops: [{ id: 's1', color: '#000000', position: 0 }, { id: 's2', color: '#FFFFFF', position: 100 }] }],
    ['linear-135', { type: 'linear', angle: 135, stops: [{ id: 's1', color: '#FF6B1A', position: 10 }, { id: 's2', color: '#4C8DFF', position: 50 }, { id: 's3', color: '#3FCF8E', position: 90 }] }],
    ['linear-equal-stops', { type: 'linear', angle: -90, stops: [{ id: 's1', color: '#202020', position: 40 }, { id: 's2', color: '#E0E0E0', position: 40 }] }],
    ['radial', { type: 'radial', angle: 0, stops: [{ id: 's1', color: '#FFFFFF', position: 0 }, { id: 's2', color: '#4C8DFF', position: 60 }, { id: 's3', color: '#000000', position: 100 }] }],
    ['solid-alpha', { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#FF6B1A80', position: 0 }] }],
    ['none', { type: 'none', angle: 0, stops: [{ id: 's1', color: '#FFFFFF', position: 0 }] }],
  ];
  return fills.map(([name, fill]) => {
    const b = newProject(`render/background-${name}`, { name: `Background ${name}`, backgroundFill: fill, w: 320, h: 180 });
    b.add(b.layer('solid', { name: 'Marker', d: { color: '#E8E2CF', w: 40, h: 40 }, p: { 'position.x': 10, 'position.y': 10, opacity: 60 } }));
    return project(b, { group: 'Render', title: `Background ${name}`, covers: ['composition background fill'], samples: [0], alpha: true });
  });
}

function scaled(): CorpusProject {
  const b = newProject('render/scaled-capture', { name: 'Scaled capture', backgroundFill: GRADIENT, w: 640, h: 360 });
  b.add(b.layer('solid', { name: 'Edge solid', d: { color: '#FF6B1A', w: 101, h: 57 }, p: { 'position.x': 33, 'position.y': 21, rotation: 9 } }));
  b.add(b.layer('image', { name: 'Image', d: { asset: ref('still-alpha.png'), fit: 'contain', w: 200, h: 112.5 }, p: { 'position.x': 400, 'position.y': 200, rotation: -14 } }));
  const inner = b.composition({ name: 'Scaled inner', w: 160, h: 90, dur: 3, bg: '#000000' });
  b.within(inner, () => { b.add(b.layer('solid', { name: 'Inner', d: { color: '#4C8DFF', w: 80, h: 45 }, p: { 'position.x': 40, 'position.y': 22.5 } })); });
  b.add(b.layer('precomp', { name: 'Precomp', d: { comp: inner.id, w: 160, h: 90 }, p: { 'position.x': 80, 'position.y': 200 } }));
  return project(b, { group: 'Render', title: 'Capture at a different size', covers: ['output scale', 'precomp target rounding at scale'], samples: [0], size: [533, 300] });
}

function video(): CorpusProject {
  const b = newProject('render/video', { name: 'Video transformed', backgroundFill: DARK, w: 480, h: 270, dur: 2 });
  b.add(b.layer('video', { name: 'Rotated H.264', d: { asset: ref('h264-aac.mp4'), fit: 'cover', w: 200, h: 112.5 }, p: { 'position.x': 130, 'position.y': 90, rotation: 21, opacity: 80 } }));
  const inner = b.composition({ name: 'Video inner', w: 320, h: 180, dur: 2, bg: '#000000' });
  b.within(inner, () => { b.add(b.layer('video', { name: 'VP9 in precomp', d: { asset: ref('vp9-opus.webm'), fit: 'contain', w: 320, h: 180 } })); });
  b.add(b.layer('precomp', { name: 'Precomp of video', d: { comp: inner.id, w: 320, h: 180 }, p: { 'position.x': 250, 'position.y': 120, 'scale.x': 60, 'scale.y': 60 } }));
  return project(b, { group: 'Render', title: 'Video transformed and nested', covers: ['video layer transform', 'video in a precomp'], samples: [.5, 1.5] });
}

const material = (color: string, extra: Doc = {}) => ({ p: { color, roughness: .45, metalness: .1, ...extra } });

function primitive3d(b: Builder, primitive: string, parameters: Doc, color: string, extra: Doc, environment: Doc, cast = true): void {
  b.add(b.object3d({ source: { primitive, parameters }, material: material(color, extra) }, { name: `Primitive ${primitive}`, position: [0, 0, 0], p: { 'rotation.y': b.ch(20, [[0, 20], [2, 110]]), 'rotation.x': 15 } }));
  b.add(b.light3d({ type: 'sun', castShadow: cast, p: { intensity: 2.5, color: '#FFF4E0' } }, { position: [-4, 6, 4] }));
  b.add(b.camera3d({ camera: { p: { targetX: 0, targetY: 0, targetZ: 0 } }, environment }, { position: [0, 1.2, 4] }));
}

function scenes3d(): CorpusProject[] {
  const cases: Array<[string, string, Doc, string, Doc, Doc, boolean]> = [
    ['sphere', 'sphere', { radius: 1, segments: 48 }, '#4C8DFF', { roughness: .35, metalness: .2 }, { shadows: false }, true],
    ['box', 'box', { width: 1.4, height: 1.4, depth: 1.4 }, '#FF6B1A', { roughness: .6, metalness: .05 }, { shadows: false, p: { ambient: .8, ambientColor: '#8899FF', exposure: 1.4 } }, true],
    ['plane-background', 'plane', { width: 2.4, height: 1.6 }, '#3FCF8E', { roughness: .8, emissive: '#102030', emissiveIntensity: 1.5 }, { shadows: true, background: '#1B2A4A' }, false],
  ];
  return cases.map(([name, primitive, parameters, color, extra, environment, cast]) => {
    const b = newProject(`render/3d-${name}`, { name: `3D ${name}`, backgroundFill: GRADIENT, w: 640, h: 360, dur: 2 });
    b.add(b.layer('solid', { name: 'Under', d: { color: '#202436', w: 200, h: 360 } }));
    primitive3d(b, primitive, parameters, color, extra, environment, cast);
    b.add(b.layer('solid', { name: 'Over', d: { color: '#E8E2CF', w: 60, h: 60 }, p: { 'position.x': 20, 'position.y': 280, opacity: 70 } }));
    return project(b, { group: 'Render', title: `3D primitive ${name} lit by a sun without shadows`, covers: ['layer3d primitive', 'sun light', 'hemisphere ambient', 'ACES filmic', 'no shadows'], samples: [0, 1], alpha: true });
  });
}

const noShadows = (extra: Doc = {}) => ({ camera: { p: { targetX: 0, targetY: 0, targetZ: 0 } }, environment: { shadows: false, ...extra } });

/** A 3D primitive inside a precomp drawn scaled and rotated, a single-sided
 *  cylinder lit by two suns. */
function precomp3d(): CorpusProject {
  const b = newProject('render/3d-precomp', { name: '3D in a precomp', backgroundFill: GRADIENT, w: 640, h: 360, dur: 2 });
  const inner = b.composition({ name: '3D inner', w: 320, h: 240, dur: 2, bg: '#000000' });
  b.within(inner, () => {
    b.add(b.object3d({ source: { primitive: 'cylinder', parameters: { radiusTop: .3, radiusBottom: .8, height: 1.6, segments: 24 } }, material: { ...material('#E8E2CF', { roughness: .5, metalness: 0 }), doubleSided: false } },
      { name: 'Cylinder', p: { 'rotation.y': b.ch(0, [[0, 0], [2, 90]]), 'rotation.x': -20 } }));
    b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 2, color: '#FFFFFF' } }, { position: [-4, 6, 4] }));
    b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 1.5, color: '#FF8844' } }, { position: [5, -1, 2] }));
    b.add(b.camera3d(noShadows({ p: { ambient: .3 } }), { position: [2.5, 1.5, 3.5] }));
  });
  b.add(b.layer('solid', { name: 'Back', d: { color: '#202436', w: 300, h: 360 } }));
  b.add(b.layer('precomp', { name: '3D precomp', d: { comp: inner.id, w: 320, h: 240 }, p: { 'position.x': 160, 'position.y': 60, 'scale.x': 110, 'scale.y': 110, rotation: 8, opacity: 90 } }));
  return project(b, { group: 'Render', title: '3D primitive in a precomp', covers: ['layer3d in a precomp', 'single-sided material', 'two suns', 'precomp transform over 3D'], samples: [0, 1.5], alpha: true });
}

/** Parented 3D layers: an object under a rotating 3D null, a light aimed
 *  through its parent's space, an orthographic camera rolled by its layer
 *  rotation; animated material and exposure. */
function rig3d(): CorpusProject {
  const b = newProject('render/3d-rig', { name: '3D rig', backgroundFill: DARK, w: 640, h: 360, dur: 2 });
  const rig = b.add(b.layer('null', { name: 'Rig', threeD: true, p: { 'position.x': 0, 'position.y': 0, 'anchor.x': 0, 'anchor.y': 0, 'rotation.y': b.ch(0, [[0, 0], [2, 60]]) } }));
  b.add(b.object3d({ source: { primitive: 'torus', parameters: { radius: 1, tube: .35, segments: 48 } }, material: { p: { color: b.ch('#A73366', [[0, '#A73366'], [2, '#4C8DFF']]), roughness: b.ch(.3, [[0, .3], [2, .8]]), metalness: .4 } } },
    { name: 'Torus', parent: rig.id, position: [0.5, 0, 0], rotation: [60, 20, 0] }));
  const lamp = b.add(b.layer('null', { name: 'Lamp rig', threeD: true, p: { 'position.x': 0, 'position.y': 0, 'anchor.x': 0, 'anchor.y': 0, rotation: 30 } }));
  b.add(b.light3d({ type: 'sun', castShadow: true, p: { intensity: 3, color: '#FFFFFF', targetX: 1, targetY: 0, targetZ: 0 } }, { position: [-3, 5, 4], parent: lamp.id }));
  b.add(b.camera3d({ camera: { projection: 'orthographic', p: { targetX: 0, targetY: 0, targetZ: 0, zoom: 1.3 } }, environment: { shadows: false, p: { exposure: b.ch(1, [[0, .8], [2, 1.6]]) } } },
    { position: [3, 2, 6], p: { rotation: 12 } }));
  return project(b, { group: 'Render', title: 'Parented 3D layers', covers: ['3D parenting through a null', 'light aim through its parent space', 'orthographic camera', 'camera roll', 'animated material', 'animated exposure'], samples: [0, 1, 2] });
}

/** The studio defaults: no light layer (the studio sun), environment shadows
 *  off, a background on the bottom layer, a half-transparent layer (no depth
 *  writes), and a hidden light. */
function studio3d(): CorpusProject {
  const b = newProject('render/3d-studio', { name: '3D studio', backgroundFill: GRADIENT, w: 640, h: 360, dur: 2 });
  b.add(b.object3d({ source: { primitive: 'sphere', parameters: { radius: 1.2, segments: 24 } }, material: material('#7DD3FC') },
    { name: 'Faded sphere', position: [0, 0, 0], p: { opacity: 55, 'rotation.y': b.ch(0, [[0, 0], [2, 40]]) } }));
  b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 9, color: '#FF0000' } }, { position: [3, 1, 2], on: false }));
  b.add(b.camera3d(noShadows({ background: '#1B2A4A', p: { ambient: .6, ambientColor: '#FFE0C0' } }), { position: [0, 1.5, 5] }));
  b.add(b.layer('solid', { name: 'Over', d: { color: '#E8E2CF', w: 60, h: 60 }, p: { 'position.x': 20, 'position.y': 280, opacity: 70 } }));
  return project(b, { group: 'Render', title: 'Studio sun, background, faded layer', covers: ['studio sun', 'hidden light layer', '3D background', 'layer opacity without depth writes'], samples: [0, 2], alpha: true });
}

/** A 3D capture at a size other than the composition's. */
function scaled3d(): CorpusProject {
  const b = newProject('render/3d-scaled', { name: '3D scaled capture', backgroundFill: GRADIENT, w: 640, h: 360, dur: 2 });
  b.add(b.object3d({ source: { primitive: 'cone', parameters: { radius: .8, height: 1.5, segments: 40 } }, material: material('#D4AF37', { roughness: .15, metalness: 1 }) },
    { name: 'Cone', rotation: [10, 0, 8] }));
  b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 3 } }, { position: [-3, 5, 4] }));
  b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 2, color: '#88AAFF' } }, { position: [4, 2, 1] }));
  b.add(b.camera3d(noShadows({ p: { ambient: .2, exposure: 1.2 } }), { position: [0, .5, 3.2] }));
  return project(b, { group: 'Render', title: '3D at another output size', covers: ['3D output scale', 'metallic material'], samples: [0], size: [533, 300] });
}

/** The tested still classes (docs/engine/pm-render.md, "Image classes"):
 *  each generated still drawn 1:1 on texel centers (decoding and color
 *  conversion compared directly) and again minified and rotated (sampling). */
const STILLS: Array<[string, number, number, boolean]> = [
  ['corpus-photo-420.jpg', 251, 147, false], ['corpus-photo-422.jpg', 250, 150, false], ['corpus-photo-444.jpg', 250, 150, false],
  ['corpus-photo-444-tall-mcu.jpg', 251, 147, false], ['corpus-photo-422-tall-mcu.jpg', 251, 147, false],
  ['corpus-photo-progressive.jpg', 251, 147, false], ['corpus-photo-gray.jpg', 250, 150, false], ['corpus-photo-orientation-6.jpg', 150, 250, false],
  ['corpus-p3.jpg', 240, 135, false], ['corpus-p3.png', 240, 135, false], ['corpus-p3.heic', 240, 135, false],
  ['corpus-16bit.png', 200, 120, true], ['corpus-gray-alpha.png', 181, 103, true], ['corpus-indexed.png', 160, 100, true],
  ['corpus-photo-lossy.webp', 250, 150, false], ['corpus-photo-lossless.webp', 250, 150, true], ['corpus-photo-alpha.webp', 250, 150, true],
  ['corpus-photo.avif', 250, 150, false], ['corpus-photo.heic', 250, 150, false],
];

function stills(): CorpusProject[] {
  return STILLS.map(([file, w, h, alpha]) => {
    const stem = file.replace(/^corpus-/, '').replace('.', '-');
    const b = newProject(`render/still-${stem}`, { name: `Still ${file}`, backgroundFill: DARK, w: 420, h: 280 });
    // Position is the box center: the 1:1 copy's corner lands on (8, 12), texel centers on pixel centers.
    b.add(b.layer('image', { name: '1:1', d: { asset: ref(file), fit: 'stretch', w, h }, p: { 'position.x': 8 + w / 2, 'position.y': 12 + h / 2 } }));
    b.add(b.layer('image', { name: 'Minified and rotated', d: { asset: ref(file), fit: 'stretch', w, h }, p: { 'position.x': 345, 'position.y': 215, 'scale.x': 45, 'scale.y': 45, rotation: 13 } }));
    return project(b, { group: 'Render', title: `Still ${file}`, covers: ['image decoding', 'image color interpretation', 'image sampling'], samples: [0], alpha });
  });
}

export function renderProjects(): CorpusProject[] {
  return [transforms(), precomps(), groups(), solo(), ...backgrounds(), scaled(), video(), ...scenes3d(), precomp3d(), rig3d(), studio3d(), scaled3d(), ...stills()];
}
