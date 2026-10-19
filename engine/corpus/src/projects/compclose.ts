/**
 * Cases that close the compositing and motion lane's routing
 * (docs/engine/composite.md, docs/engine/motion.md): motion blur and 2.5D on
 * every layer type (shapes, editable paths, text, 3D objects, adjustment
 * layers), motion blur together with masks, blend modes, mattes, and
 * effects (today's slow path), and bezier path masks (today's Canvas2D
 * coverage route) with feather, subtraction, animation, transforms, 3D
 * layers, groups, and precomps.
 */
import { newProject } from '../doc';
import { ref } from '../media';
import { GRADIENT, project, type CorpusProject } from './common';

const DARK = { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#14161C', position: 0 }] };

function blurVector(): CorpusProject {
  const b = newProject('motion/blur-vector', { name: 'Motion blur: shapes and paths', w: 480, h: 270, backgroundFill: GRADIENT });
  b.add(b.layer('shape', { name: 'Rounded rect slide (blur)', mblur: true, d: { shape: 'rect', w: 70, h: 44, radius: 12, color: '#FF6B1A' }, p: { 'position.x': b.ch(40, [[0, 40], [2, 440]]), 'position.y': 36 } }));
  b.add(b.layer('shape', { name: 'Stroked ellipse spin (blur, 80%)', mblur: true, d: { shape: 'ellipse', w: 110, h: 46, color: '#4C8DFF', stroke: 5, strokeColor: '#FFFFFF' }, p: { 'position.x': 110, 'position.y': 130, rotation: b.ch(0, [[0, 0], [2, 900]]), opacity: 80 } }));
  // The raster density follows the scale at every sample time.
  b.add(b.layer('shape', { name: 'Star zoom (blur)', mblur: true, d: { shape: 'star', w: 60, h: 60, points: 6, color: '#3FCF8E', stroke: 2, strokeColor: '#0C0D12' }, p: { 'position.x': 250, 'position.y': 130, 'scale.x': b.ch(30, [[0, 30], [2, 330]]), 'scale.y': b.ch(30, [[0, 30], [2, 330]]) } }));
  // Shape content animates inside the shutter.
  b.add(b.layer('shape', { name: 'Growing bar (blur)', mblur: true, d: { shape: 'rect', w: b.ch(10, [[0, 10], [2, 220]]), h: 18, radius: 4, color: '#F2C14E' }, p: { 'position.x': 360, 'position.y': 210 } }));
  const paths = b.add(b.layer('shape', { name: 'Editable path (blur)', mblur: true, d: { shape: 'rect', w: 200, h: 200, color: '#E8E2CF', radius: 0, stroke: 0, strokeColor: '#ffffff', points: 5 }, p: { 'position.x': b.ch(380, [[0, 380], [2, 120]]), 'position.y': 90, rotation: b.ch(-10, [[0, -10], [2, 40]]) } }));
  paths.d.paths = [b.path('Badge', [[-50, -30, 0, 0, 25, 0], [50, -30, -25, 0, 0, 0], [50, 30], [-50, 30]], { closed: true, fill: '#A73366', stroke: '#FFFFFF', strokeWidth: 3 })];
  const group = b.layer('group', { name: 'Group of shapes (blur)', mblur: true, p: { 'position.x': b.ch(0, [[0, 0], [2, 200]]) } });
  b.add(b.layer('shape', { name: 'Grouped polygon', group: group.id, d: { shape: 'polygon', w: 50, h: 50, points: 5, color: '#7DD3FC' }, p: { 'position.x': 40, 'position.y': 230, rotation: b.ch(0, [[0, 0], [2, 180]]) } }));
  b.add(b.layer('shape', { name: 'Grouped line', group: group.id, d: { shape: 'line', w: 80, h: 0, stroke: 6, strokeColor: '#FB7185', color: '#FB7185' }, p: { 'position.x': 110, 'position.y': 250 } }));
  b.add(group);
  const inner = b.composition({ name: 'Inner shapes', w: 160, h: 90, dur: 3, bg: '#000000' });
  b.within(inner, () => {
    b.add(b.layer('shape', { name: 'Inner spinner', d: { shape: 'rect', w: 70, h: 12, radius: 6, color: '#FF6B1A' }, p: { 'position.x': 80, 'position.y': 45, rotation: b.ch(0, [[0, 0], [3, 720]]) } }));
  });
  b.add(b.layer('precomp', { name: 'Precomp of shapes (blur)', mblur: true, d: { comp: inner.id, w: 160, h: 90 }, p: { 'position.x': b.ch(300, [[0, 300], [2, 260]]), 'position.y': 150, 'scale.x': 70, 'scale.y': 70 } }));
  return project(b, { group: 'Motion', title: 'Motion blur on shapes and paths', covers: ['shape motion blur', 'raster density per sample', 'animated shape content inside the shutter', 'editable path motion blur', 'blurred group of shapes', 'blurred precomp of shapes'], samples: [0.3, 1, 1.7], alpha: true, blurSamples: [3, 6] });
}

function blurSlow(): CorpusProject {
  const b = newProject('motion/blur-slow', { name: 'Motion blur with masks, blends, mattes, effects', w: 480, h: 270, backgroundFill: GRADIENT });
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 480, h: 270 }, p: { 'position.x': 240, 'position.y': 135, opacity: 60 } }));
  const masked = b.add(b.layer('solid', { name: 'Masked slide (blur)', mblur: true, d: { color: '#FF6B1A', w: 110, h: 70 }, p: { 'position.x': b.ch(20, [[0, 20], [2, 340]]), 'position.y': 20 } }));
  masked.masks.push(b.mask('ellipse', { x: 0, y: 0, w: 90, h: 60, feather: 14 }));
  b.add(b.layer('solid', { name: 'Multiply spin (blur)', mblur: true, blend: 'multiply', d: { color: '#4C8DFF', w: 130, h: 26 }, p: { 'position.x': 120, 'position.y': 140, rotation: b.ch(0, [[0, 0], [2, 720]]) } }));
  const source = b.add(b.layer('solid', { name: 'Matte (hidden)', on: false, d: { color: '#FFFFFF', w: 120, h: 80 }, p: { 'position.x': 300, 'position.y': 110, rotation: 12 } }));
  const target = b.add(b.layer('solid', { name: 'Matted slide (blur)', mblur: true, d: { color: '#3FCF8E', w: 70, h: 70 }, p: { 'position.x': b.ch(260, [[0, 260], [2, 380]]), 'position.y': 110 } }));
  target.matteSource = source.id;
  const blurredSource = b.add(b.layer('shape', { name: 'Blurred matte source', on: false, mblur: true, d: { shape: 'ellipse', w: 80, h: 80, color: '#FFFFFF' }, p: { 'position.x': b.ch(40, [[0, 40], [2, 200]]), 'position.y': 220 } }));
  const t2 = b.add(b.layer('solid', { name: 'Matted by a blurred source', d: { color: '#F2C14E', w: 260, h: 60 }, p: { 'position.x': 20, 'position.y': 190 } }));
  t2.matteSource = blurredSource.id;
  t2.matteMode = b.ch('luma');
  b.add(b.layer('solid', { name: 'Effect slide (blur)', mblur: true, fx: [b.effect('color', { saturation: -60, contrast: 20 })], d: { color: '#E8E2CF', w: 60, h: 60 }, p: { 'position.x': b.ch(400, [[0, 400], [2, 300]]), 'position.y': 200, rotation: b.ch(0, [[0, 0], [2, 120]]) } }));
  const group = b.layer('group', { name: 'Screen group with mask (blur, 80%)', mblur: true, blend: 'screen', p: { opacity: 80, 'position.y': b.ch(0, [[0, 0], [2, 40]]) } });
  b.add(b.layer('solid', { name: 'Group member', group: group.id, d: { color: '#A73366', w: 90, h: 60 }, p: { 'position.x': 380, 'position.y': 20, rotation: b.ch(0, [[0, 0], [2, 90]]) } }));
  b.add(group);
  group.masks.push(b.mask('rect', { x: 410, y: 40, w: 80, h: 70, feather: 6 }));
  return project(b, { group: 'Motion', title: 'Motion blur on the slow path', covers: ['blurred layer with a mask', 'blurred layer with a blend mode', 'blurred matted layer', 'blurred track matte source', 'blurred layer with an effect', 'blurred styled group (blend, mask, opacity)'], samples: [0.3, 1, 1.7], alpha: true, blurSamples: [3] });
}

function blurText(): CorpusProject {
  const b = newProject('motion/blur-text', { name: 'Motion blur on text', w: 480, h: 270, backgroundFill: DARK });
  b.add(b.layer('text', { name: 'Sliding title (blur)', mblur: true, d: { text: 'Powermove', size: 44, weight: 700, color: '#FFFFFF', align: 'left' }, p: { 'position.x': b.ch(20, [[0, 20], [2, 260]]), 'position.y': 50 } }));
  b.add(b.layer('text', { name: 'Spinning label (blur)', mblur: true, d: { text: 'Spin', size: 36, weight: 600, color: '#FF6B1A', align: 'center' }, p: { 'position.x': 120, 'position.y': 170, rotation: b.ch(0, [[0, 0], [2, 540]]) } }));
  b.add(b.layer('text', { name: 'Zooming label (blur)', mblur: true, d: { text: 'Zoom', size: 24, weight: 500, color: '#4C8DFF', align: 'center' }, p: { 'position.x': 340, 'position.y': 180, 'scale.x': b.ch(40, [[0, 40], [2, 300]]), 'scale.y': b.ch(40, [[0, 40], [2, 300]]) } }));
  const group = b.layer('group', { name: 'Text group (blur)', mblur: true, p: { 'position.x': b.ch(0, [[0, 0], [2, -120]]) } });
  b.add(b.layer('text', { name: 'Grouped caption', group: group.id, d: { text: 'grouped', size: 22, color: '#3FCF8E', align: 'left' }, p: { 'position.x': 300, 'position.y': 240 } }));
  b.add(group);
  return project(b, { group: 'Motion', title: 'Motion blur on text', covers: ['text motion blur', 'text raster density per sample', 'blurred group of text'], samples: [0.4, 1.2], alpha: true });
}

function blur3d(): CorpusProject {
  const b = newProject('motion/blur-3d', { name: 'Motion blur on 3D layers', w: 480, h: 270, shutter: 0.75, backgroundFill: GRADIENT });
  b.add(b.object3d({ source: { primitive: 'box', parameters: { width: 0.8, height: 0.8, depth: 0.8 } }, material: { p: { color: '#FF6B1A', roughness: .45, metalness: .1 } } },
    { name: 'Sliding box (blur)', mblur: true, position: [-1.6, 0.6, 0], p: { 'position.x': b.ch(-1.6, [[0, -1.6], [2, 1.6]]), 'rotation.y': b.ch(0, [[0, 0], [2, 200]]) } }));
  const masked = b.add(b.object3d({ source: { primitive: 'sphere', parameters: { radius: 0.5, segments: 32 } }, material: { p: { color: '#4C8DFF', roughness: .5, metalness: .1 } } },
    { name: 'Masked rising sphere (blur)', mblur: true, position: [0.9, -0.6, 0], p: { 'position.y': b.ch(-0.9, [[0, -0.9], [2, 0.6]]) } }));
  masked.masks.push(b.mask('rect', { x: 330, y: 135, w: 120, h: 300, feather: 10 }));
  b.add(b.object3d({ source: { primitive: 'torus', parameters: { radius: 0.4, tube: 0.14, segments: 48 } }, material: { p: { color: '#3FCF8E', roughness: .4, metalness: .2 } } },
    { name: 'Torus (no blur)', position: [0, -0.2, -0.5], p: { 'rotation.x': b.ch(0, [[0, 0], [2, 360]]) } }));
  b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 2.5, color: '#FFFFFF' } }, { position: [-4, 6, 4] }));
  b.add(b.camera3d({ camera: { p: { targetX: 0, targetY: 0, targetZ: 0 } }, environment: { shadows: false } }, { position: [0, 0.6, 4.5] }));
  return project(b, { group: 'Motion', title: 'Motion blur on 3D object layers', covers: ['3D object motion blur (the scene at each sample time)', 'masked blurred 3D object', 'unblurred object sharing the scene'], samples: [0.5, 1.3] });
}

function blurGroup3d(): CorpusProject {
  const b = newProject('motion/blur-3d-group', { name: '3D layers in a blurred group', w: 480, h: 270, backgroundFill: DARK });
  const group = b.layer('group', { name: 'Blurred group', mblur: true, p: { opacity: 85 } });
  b.add(b.object3d({ source: { primitive: 'box', parameters: { width: 0.9, height: 0.9, depth: 0.9 } }, material: { p: { color: '#F2C14E', roughness: .5, metalness: 0 } } },
    { name: 'Spinning box in the group', group: group.id, position: [0, 0, 0], p: { 'rotation.y': b.ch(0, [[0, 0], [2, 400]]), 'rotation.x': 20 } }));
  b.add(b.layer('solid', { name: 'Solid in the group', group: group.id, d: { color: '#A73366', w: 60, h: 60 }, p: { 'position.x': b.ch(20, [[0, 20], [2, 380]]), 'position.y': 190 } }));
  b.add(group);
  b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 2.2, color: '#FFFFFF' } }, { position: [3, 5, 4] }));
  b.add(b.camera3d({ camera: { p: { targetX: 0, targetY: 0, targetZ: 0 } }, environment: { shadows: false } }, { position: [0, 0.5, 4] }));
  return project(b, { group: 'Motion', title: '3D object in a blurred group', covers: ['3D object sampled with its blurred group'], samples: [0.6, 1.4] });
}

function spaceVector(): CorpusProject {
  const b = newProject('space25d/vector', { name: '2.5D shapes and paths', w: 640, h: 360, backgroundFill: GRADIENT });
  b.add(b.layer('shape', { name: 'Card (rotation Y)', threeD: true, d: { shape: 'rect', w: 160, h: 100, radius: 16, color: '#FF6B1A', stroke: 4, strokeColor: '#FFFFFF' }, p: { 'position.x': 140, 'position.y': 110, 'rotation.y': b.ch(-55, [[0, -55], [2.5, 55]]), perspective: 40 } }));
  b.add(b.layer('shape', { name: 'Ellipse (rotation X, Z)', threeD: true, d: { shape: 'ellipse', w: 140, h: 90, color: '#4C8DFF' }, p: { 'position.x': 330, 'position.y': 110, 'rotation.x': b.ch(60, [[0, 60], [2.5, -40]]), 'position.z': b.ch(-150, [[0, -150], [2.5, 120]]) } }));
  b.add(b.layer('shape', { name: 'Star (orientation, scale)', threeD: true, d: { shape: 'star', w: 110, h: 110, points: 5, color: '#3FCF8E' }, p: { 'position.x': 500, 'position.y': 120, 'orientation.y': 35, 'orientation.x': -15, 'scale.x': b.ch(60, [[0, 60], [2.5, 180]]), 'scale.y': b.ch(60, [[0, 60], [2.5, 180]]) } }));
  const paths = b.add(b.layer('shape', { name: 'Editable path (3D)', threeD: true, d: { shape: 'rect', w: 200, h: 200, color: '#E8E2CF', radius: 0, stroke: 0, strokeColor: '#ffffff', points: 5 }, p: { 'position.x': 160, 'position.y': 270, 'rotation.y': 40, 'rotation.x': b.ch(-20, [[0, -20], [2.5, 30]]) } }));
  paths.d.paths = [b.path('Wave', [[-80, 0, 0, 0, 30, -50], [0, 0, -30, 50, 30, -50], [80, 0, -30, 50, 0, 0]], { fillEnabled: false, stroke: '#F2C14E', strokeWidth: 6 })];
  const masked = b.add(b.layer('shape', { name: 'Masked shape (3D)', threeD: true, d: { shape: 'rect', w: 150, h: 110, color: '#A73366' }, p: { 'position.x': 330, 'position.y': 270, 'rotation.y': b.ch(30, [[0, 30], [2.5, -30]]) } }));
  masked.masks.push(b.mask('ellipse', { x: 0, y: 0, w: 120, h: 80, feather: 10 }));
  const rig = b.layer('group', { name: '3D group of shapes', threeD: true, p: { 'position.x': 520, 'position.y': 270, 'anchor.x': 520, 'anchor.y': 270, 'rotation.y': b.ch(-40, [[0, -40], [2.5, 40]]), perspective: 55 } });
  b.add(b.layer('shape', { name: 'Grouped polygon', group: rig.id, d: { shape: 'polygon', w: 70, h: 70, points: 6, color: '#7DD3FC' }, p: { 'position.x': 480, 'position.y': 260 } }));
  b.add(b.layer('shape', { name: 'Grouped line (Z)', threeD: true, group: rig.id, d: { shape: 'line', w: 90, h: 0, stroke: 6, strokeColor: '#FB7185', color: '#FB7185' }, p: { 'position.x': 560, 'position.y': 290, 'position.z': -50 } }));
  b.add(rig);
  b.add(b.layer('shape', { name: 'Blurred 3D shape', threeD: true, mblur: true, d: { shape: 'rect', w: 60, h: 40, radius: 8, color: '#FFFFFF' }, p: { 'position.x': 330, 'position.y': 30, 'rotation.y': b.ch(0, [[0, 0], [2.5, 720]]) } }));
  return project(b, { group: 'Space', title: '2.5D shapes and paths', covers: ['3D shape layers', 'editable paths in 3D', 'masks on 3D shapes', 'shapes in a 3D group', 'motion blur on a 3D shape'], samples: [0, 1.25, 2.5], alpha: true });
}

function spaceText(): CorpusProject {
  const b = newProject('space25d/text', { name: '2.5D text', w: 640, h: 360, backgroundFill: DARK });
  b.add(b.layer('text', { name: 'Swinging title', threeD: true, d: { text: 'Powermove', size: 56, weight: 700, color: '#FFFFFF', align: 'center' }, p: { 'position.x': 320, 'position.y': 110, 'rotation.y': b.ch(-50, [[0, -50], [2.5, 50]]), perspective: 45 } }));
  b.add(b.layer('text', { name: 'Floor label', threeD: true, d: { text: 'on the floor', size: 40, weight: 500, color: '#FF6B1A', align: 'center' }, p: { 'position.x': 320, 'position.y': 280, 'rotation.x': 65, 'position.z': b.ch(150, [[0, 150], [2.5, -100]]) } }));
  const rig = b.layer('group', { name: '3D group of text', threeD: true, p: { 'position.x': 520, 'position.y': 200, 'anchor.x': 520, 'anchor.y': 200, 'rotation.x': b.ch(-30, [[0, -30], [2.5, 30]]) } });
  b.add(b.layer('text', { name: 'Grouped text', group: rig.id, d: { text: 'grouped', size: 30, color: '#3FCF8E', align: 'left' }, p: { 'position.x': 460, 'position.y': 200 } }));
  b.add(rig);
  return project(b, { group: 'Space', title: '2.5D text', covers: ['3D text layers', 'text in a 3D group'], samples: [0, 1.25, 2.5], alpha: true });
}

function spaceOthers(): CorpusProject {
  const b = newProject('space25d/others', { name: '3D objects in 3D groups', w: 480, h: 270, backgroundFill: GRADIENT });
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 480, h: 270 }, p: { 'position.x': 240, 'position.y': 135 } }));
  const rig = b.layer('group', { name: '3D group with an object', threeD: true, p: { 'position.x': 240, 'position.y': 135, 'anchor.x': 240, 'anchor.y': 135, 'rotation.y': b.ch(-20, [[0, -20], [2, 20]]) } });
  b.add(b.object3d({ source: { primitive: 'sphere', parameters: { radius: 0.6, segments: 32 } }, material: { p: { color: '#4C8DFF', roughness: .5, metalness: .1 } } }, { name: 'Sphere in a 3D group', group: rig.id, position: [0.8, 0, 0] }));
  b.add(b.layer('solid', { name: 'Card in the group', group: rig.id, d: { color: '#FF6B1A', w: 90, h: 60 }, p: { 'position.x': 120, 'position.y': 100 } }));
  b.add(rig);
  // Adjustment layers have no 3D switch (normalization clears it); one with
  // a mask sits over the 3D group.
  const adjustment = b.add(b.layer('adjustment', { name: 'Adjustment with mask', fx: [b.effect('color', { saturation: -80, contrast: 10 })], p: { 'anchor.x': 240, 'anchor.y': 135, 'position.x': 240, 'position.y': 135, rotation: b.ch(-6, [[0, -6], [2, 6]]) } }));
  adjustment.masks.push(b.mask('rect', { x: 120, y: 135, w: 200, h: 200, feather: 8 }));
  b.add(b.light3d({ type: 'sun', castShadow: false, p: { intensity: 2.5, color: '#FFFFFF' } }, { position: [-4, 6, 4] }));
  b.add(b.camera3d({ camera: { p: { targetX: 0, targetY: 0, targetZ: 0 } }, environment: { shadows: false } }, { position: [0, 0.5, 4] }));
  return project(b, { group: 'Space', title: '3D objects in 3D groups', covers: ['3D object inside a 3D group', 'flat layer beside it in the group', 'masked adjustment over a 3D group'], samples: [0, 1.4] });
}

function pathMasks(): CorpusProject {
  const b = newProject('composite/path-masks', { name: 'Bezier path masks', w: 480, h: 300, dur: 3, backgroundFill: GRADIENT });
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 480, h: 300 }, p: { 'position.x': 240, 'position.y': 150 } }));
  const blob = (extra: Record<string, unknown> = {}) => b.path('Blob', [[-60, -20, 0, 0, 30, -40], [30, -50, -20, 0, 40, 15], [70, 15, 0, -30, -10, 45], [-15, 55, 40, 0, -50, 0]], { closed: true, ...extra });
  // No feather: the antialiased fill alone.
  const sharp = b.add(b.layer('solid', { name: 'Sharp path mask', d: { color: '#FF6B1A', w: 150, h: 120 }, p: { 'position.x': 20, 'position.y': 20 } }));
  sharp.masks.push(b.mask('rect', { x: 75, y: 60, feather: 0 }, { path: blob() }));
  // Feather (CSS blur) with a parametric subtract mask on the same layer.
  const feathered = b.add(b.layer('image', { name: 'Feathered path and a subtract ellipse', d: { asset: ref('still-alpha.png'), fit: 'stretch', w: 150, h: 120 }, p: { 'position.x': 180, 'position.y': 20 } }));
  feathered.masks.push(b.mask('rect', { x: 75, y: 60, feather: 16 }, { path: blob({ rotation: b.ch(0, [[0, 0], [3, 40]]) }) }));
  feathered.masks.push(b.mask('ellipse', { x: 80, y: 60, w: 40, h: 40, feather: 4 }, { mode: 'subtract' }));
  // Animated vertices, a subtracted path, an animated mode, and a mask that switches off.
  const animated = blob({ scaleX: b.ch(80, [[0, 80], [3, 130]]) });
  animated.vertices[1]!.p.y = b.ch(-50, [[0, -50], [3, -10]]);
  const moving = b.add(b.layer('solid', { name: 'Animated paths', d: { color: '#4C8DFF', w: 140, h: 120 }, p: { 'position.x': 340, 'position.y': 20 } }));
  moving.masks.push(b.mask('rect', { x: 70, y: 60, feather: 6 }, { path: animated }));
  moving.masks.push(b.mask('rect', { x: 70, y: 60, feather: 2 }, { path: b.path('Hole', [[-15, -15], [15, -15], [15, 15], [-15, 15]], { closed: true }), mode: b.ch('subtract', [[0, 'subtract', { hold: true }], [1.5, 'add', { hold: true }]]) }));
  moving.masks.push(b.mask('rect', { x: 20, y: 20, w: 30, h: 30, feather: 0 }, { on: b.ch(true, [[0, true, { hold: true }], [1, false, { hold: true }], [2, true, { hold: true }]]) }));
  // Subtract-only paths over a rotated, scaled layer (feather scales with the world matrix).
  const rotated = b.add(b.layer('solid', { name: 'Subtract-only, rotated and scaled', d: { color: '#3FCF8E', w: 120, h: 90 }, p: { 'position.x': 70, 'position.y': 230, rotation: b.ch(-15, [[0, -15], [3, 25]]), 'scale.x': 130, 'scale.y': 90, 'anchor.x': 60, 'anchor.y': 45 } }));
  rotated.masks.push(b.mask('rect', { x: 60, y: 45, feather: 10 }, { path: blob({ scaleX: 60, scaleY: 60 }), mode: 'subtract' }));
  // A shape layer and a 3D layer.
  const shape = b.add(b.layer('shape', { name: 'Shape with a path mask', d: { shape: 'star', w: 120, h: 120, points: 7, color: '#F2C14E' }, p: { 'position.x': 230, 'position.y': 220 } }));
  shape.masks.push(b.mask('rect', { x: 0, y: 0, feather: 8 }, { path: blob({ scaleX: 90, scaleY: 90 }) }));
  const plane = b.add(b.layer('solid', { name: '3D layer with a path mask', threeD: true, d: { color: '#A73366', w: 140, h: 100 }, p: { 'position.x': 330, 'position.y': 170, 'rotation.y': b.ch(-35, [[0, -35], [3, 35]]), 'anchor.x': 70, 'anchor.y': 50 } }));
  plane.masks.push(b.mask('rect', { x: 70, y: 50, feather: 12 }, { path: blob() }));
  return project(b, { group: 'Compositing', title: 'Bezier path masks', covers: ['path mask without feather', 'feathered path mask', 'path and parametric masks on one layer', 'animated path vertices and transform', 'animated mask mode', 'mask switching off', 'subtract-only path masks', 'feather under rotation and scale', 'path mask on a shape layer', 'path mask on a 3D layer'], samples: [0, 1.2, 2.4], alpha: true });
}

function pathMaskGroups(): CorpusProject {
  const b = newProject('composite/path-masks-nested', { name: 'Path masks on groups and precomps', w: 400, h: 240, dur: 3, backgroundFill: GRADIENT });
  const blob = () => b.path('Blob', [[-60, -20, 0, 0, 30, -40], [30, -50, -20, 0, 40, 15], [70, 15, 0, -30, -10, 45], [-15, 55, 40, 0, -50, 0]], { closed: true });
  const group = b.layer('group', { name: 'Masked group', p: { opacity: 90 } });
  b.add(b.layer('solid', { name: 'Member A', group: group.id, d: { color: '#FF6B1A', w: 120, h: 80 }, p: { 'position.x': 30, 'position.y': 40 } }));
  b.add(b.layer('solid', { name: 'Member B', group: group.id, d: { color: '#4C8DFF', w: 80, h: 120 }, p: { 'position.x': 90, 'position.y': 70 } }));
  b.add(group);
  group.masks.push(b.mask('rect', { x: 110, y: 110, feather: 10 }, { path: blob() }));
  const inner = b.composition({ name: 'Inner', w: 160, h: 120, dur: 3, bg: '#202638' });
  b.within(inner, () => {
    b.add(b.layer('solid', { name: 'Inner bar', d: { color: '#3FCF8E', w: 160, h: 30 }, p: { 'position.y': b.ch(0, [[0, 0], [3, 90]]) } }));
  });
  const pre = b.add(b.layer('precomp', { name: 'Masked precomp', d: { comp: inner.id, w: 160, h: 120 }, p: { 'position.x': 220, 'position.y': 60, rotation: 8 } }));
  pre.masks.push(b.mask('rect', { x: 80, y: 60, feather: 4 }, { path: blob() }));
  return project(b, { group: 'Compositing', title: 'Path masks on groups and precomps', covers: ['path mask on a styled group', 'path mask on a precomp'], samples: [0, 1.5], alpha: true });
}

function unknownModes(): CorpusProject {
  const b = newProject('composite/unknown-modes', { name: 'Unknown and animated blend and matte modes', w: 400, h: 240, dur: 3, backgroundFill: GRADIENT });
  b.add(b.layer('image', { name: 'Photo', d: { asset: ref('corpus-photo.tiff'), fit: 'cover', w: 400, h: 240 }, p: { 'position.x': 200, 'position.y': 120, opacity: 80 } }));
  // Modes today's BLEND_ID does not know draw normally. Normalization turns
  // a stored unknown mode into normal; a keyed one reaches the compositor.
  b.add(b.layer('solid', { name: 'Keyed unknown mode', blend: b.ch('hardlight'), d: { color: '#FF6B1A', w: 90, h: 60 }, p: { 'position.x': 20, 'position.y': 20, opacity: 85 } }));
  b.add(b.layer('solid', { name: 'Keyed empty mode', blend: b.ch(''), d: { color: '#4C8DFF', w: 90, h: 60 }, p: { 'position.x': 120, 'position.y': 20 } }));
  b.add(b.layer('solid', { name: 'Keyed into an unknown mode', blend: b.ch('multiply', [[0, 'multiply', { hold: true }], [1, 'colorDodge', { hold: true }], [2, 'screen', { hold: true }]]), d: { color: '#3FCF8E', w: 90, h: 60 }, p: { 'position.x': 220, 'position.y': 20, opacity: 70 } }));
  // Unknown and keyed matte modes read with startsWith('luma') and
  // endsWith('inverted'). (A missing matte source does not pass validation;
  // tests/composite.rs draws it.)
  const source = b.add(b.layer('solid', { name: 'Matte (hidden)', on: false, d: { color: '#808080', w: 80, h: 80 }, p: { 'position.x': 130, 'position.y': 110, rotation: 15 } }));
  const unknown = b.add(b.layer('solid', { name: 'Unknown matte mode', d: { color: '#A73366', w: 100, h: 70 }, p: { 'position.x': 120, 'position.y': 120 } }));
  unknown.matteSource = source.id;
  unknown.matteMode = b.ch('luma-ish');
  const source2 = b.add(b.layer('solid', { name: 'Matte 2 (hidden)', on: false, d: { color: '#C0C0C0', w: 90, h: 70 }, p: { 'position.x': 250, 'position.y': 110, rotation: -10 } }));
  const keyed = b.add(b.layer('solid', { name: 'Keyed matte modes', d: { color: '#7DD3FC', w: 100, h: 70 }, p: { 'position.x': 240, 'position.y': 120 } }));
  keyed.matteSource = source2.id;
  keyed.matteMode = b.ch('alpha', [[0, 'alpha', { hold: true }], [0.8, 'luma', { hold: true }], [1.6, 'alpha-inverted', { hold: true }], [2.4, 'mode-inverted', { hold: true }]]);
  return project(b, { group: 'Compositing', title: 'Unknown and animated blend and matte modes', covers: ['keyed unknown blend mode', 'keyed empty blend mode', 'keyed blend mode into an unknown mode', 'unknown matte mode', 'keyed matte modes'], samples: [0.4, 1.2, 2, 2.8], alpha: true });
}

export function compcloseProjects(): CorpusProject[] {
  return [blurVector(), blurSlow(), blurText(), blur3d(), blurGroup3d(), spaceVector(), spaceText(), spaceOthers(), pathMasks(), pathMaskGroups(), unknownModes()];
}
