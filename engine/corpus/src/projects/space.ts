import { scene3d } from '../catalog';
import { newProject, type Builder, type Doc } from '../doc';
import { MISSING_MEDIA, ref } from '../media';
import { GRADIENT, project, type CorpusProject } from './common';

const BLACK = { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#0C0D12', position: 0 }] };

function layers25d(): CorpusProject[] {
  const b = newProject('space/layers-25d', { name: '2.5D layers', backgroundFill: GRADIENT });
  b.add(b.layer('image', { name: 'Floor art', threeD: true, d: { asset: ref('corpus-art.svg'), fit: 'cover', w: 420, h: 236 }, p: { 'position.y': 260, 'rotation.x': 70, 'position.z': 50 } }));
  b.add(b.layer('shape', { name: 'Card swinging', threeD: true, d: { w: 160, h: 110, color: '#FF6B1A', radius: 12 }, p: { 'position.x': 220, 'rotation.y': b.ch(-60, [[0, -60], [2.5, 60]]), perspective: 35 } }));
  b.add(b.layer('text', { name: 'Text in depth', threeD: true, d: { text: 'DEPTH', size: 64, weight: 800, color: '#E8E2CF' }, p: { 'position.x': 440, 'position.z': b.ch(-200, [[0, -200], [2.5, 200]]), 'orientation.y': 25 } }));
  b.add(b.layer('shape', { name: 'Coplanar A', threeD: true, d: { w: 100, h: 100, color: '#4C8DFF' }, p: { 'position.x': 500, 'position.y': 90 } }));
  b.add(b.layer('shape', { name: 'Coplanar B', threeD: true, d: { shape: 'ellipse', w: 100, h: 100, color: '#3FCF8E' }, p: { 'position.x': 530, 'position.y': 110 } }));
  b.add(b.layer('shape', { name: '2D overlay', d: { w: 600, h: 6, color: '#F2C14E' }, p: { 'position.y': 20 } }));
  const g = newProject('space/groups-25d', { name: '3D groups with flat children', backgroundFill: GRADIENT });
  const rig = g.layer('group', { name: '3D rig', threeD: true, p: { 'position.x': 320, 'position.y': 180, 'anchor.x': 320, 'anchor.y': 180, 'rotation.y': g.ch(-40, [[0, -40], [2.5, 40]]), 'rotation.x': 15, 'position.z': 80, perspective: 60 } });
  const inner = g.layer('group', { name: 'Inner cards', group: rig.id, p: { 'scale.x': 90, 'scale.y': 90, 'position.x': 320, 'position.y': 180, 'anchor.x': 320, 'anchor.y': 180 } });
  g.add(g.layer('shape', { name: 'Left card', group: inner.id, d: { w: 130, h: 140, color: '#FF6040' }, p: { 'position.x': 240, 'position.y': 180 } }));
  g.add(g.layer('shape', { name: 'Right card', group: inner.id, d: { w: 130, h: 140, color: '#4060FF' }, p: { 'position.x': 400, 'position.y': 180 }, masks: [] }));
  g.add(inner);
  g.add(g.layer('text', { name: 'Caption', group: rig.id, d: { text: 'flat in a 3D group', size: 26 }, p: { 'position.x': 320, 'position.y': 290 } }));
  g.add(rig);
  return [
    project(b, { group: 'Transforms', title: '2.5D layers', covers: ['3D switch', 'Z position', 'X and Y rotation', 'orientation', 'perspective', 'homography planes', 'coplanar ordering', 'painter sort', '2D layer over 3D'], samples: [0, 1.25, 2.5], alpha: true }),
    project(g, { group: 'Transforms', title: '3D groups with flat children', covers: ['perspective per 3D group', 'nested groups in a 3D group'], samples: [0, 1.25, 2.5] }),
  ];
}

const material = (color: string, extra: Doc = {}) => ({ p: { color, roughness: .45, metalness: .1, ...extra } });

function camera(b: Builder, data: Doc = {}, position: [number, number, number] = [0, 1.6, 7]): Doc {
  return b.camera3d({ camera: { p: { targetX: 0, targetY: 0, targetZ: 0, ...(data.camera?.p ?? {}) }, ...(data.camera?.projection ? { projection: data.camera.projection } : {}) }, environment: data.environment ?? {} }, { position });
}

function primitives(): CorpusProject {
  const b = newProject('3d/primitives', { name: '3D primitives', backgroundFill: BLACK });
  b.add(b.object3d({ source: { primitive: 'plane', parameters: { width: 12, height: 8 } }, material: material('#3A3D4A', { roughness: .9 }), castShadow: false }, { name: 'Ground', position: [0, -1.2, 0], rotation: [-90, 0, 0] }));
  const kinds: Array<[string, Doc]> = [
    ['box', { width: 1, height: 1, depth: 1 }], ['sphere', { radius: .6, segments: 32 }], ['cylinder', { radiusTop: .4, radiusBottom: .5, height: 1.2, segments: 24 }],
    ['cone', { radius: .55, height: 1.2, segments: 24 }], ['torus', { radius: .45, tube: .16, segments: 32 }], ['capsule', { radius: .35, height: .8, segments: 16 }],
    ['icosahedron', { radius: .6, detail: 1 }], ['plane', { width: 1, height: 1 }],
  ];
  const colors = ['#FF6B1A', '#4C8DFF', '#3FCF8E', '#F2C14E', '#A73366', '#E8E2CF', '#7DD3FC', '#FB7185'];
  kinds.forEach(([primitive, parameters], index) => b.add(b.object3d(
    { source: { primitive, parameters }, material: material(colors[index]!, { metalness: index % 3 === 0 ? .8 : .1, roughness: index % 2 ? .3 : .7, ...(index === 5 ? { emissive: '#FF6B1A', emissiveIntensity: 1.5 } : {}) }) },
    { name: `Primitive ${primitive}`, position: [-3.3 + (index % 4) * 2.2, index < 4 ? .9 : -.6, index < 4 ? -.6 : .9], rotation: [0, 0, 0], p: { 'rotation.y': b.ch(0, [[0, 0], [3, 120]]), 'rotation.x': 15 } },
  )));
  b.add(b.light3d({ type: 'sun', p: { intensity: 2.5, color: '#FFF4E0' } }, { position: [-4, 6, 4] }));
  b.add(camera(b));
  return project(b, { group: '3D', title: 'Primitives with shadows', covers: ['primitives: box, sphere, plane, cylinder, cone, torus, capsule, icosahedron', 'standard material color, roughness, metalness, emissive', 'sun shadow', 'receive shadow', 'animated 3D rotation'], samples: [0, 1.5, 3], alpha: true });
}

function latheExtrude(): CorpusProject {
  const b = newProject('3d/lathe-extrude', { name: 'Lathe and extrude', backgroundFill: BLACK });
  b.add(b.object3d({ source: { lathe: [[0, -1], [.5, -1], [.65, -.6], [.35, .1], [.42, .7], [.25, 1]], segments: 40 }, material: material('#7DD3FC', { metalness: .6, roughness: .25 }) }, { name: 'Lathe vase', position: [-1.4, 0, 0] }));
  const star = Array.from({ length: 10 }, (_, i) => { const r = i % 2 ? .35 : .8, a = i * Math.PI / 5 + Math.PI / 2; return [Math.round(Math.cos(a) * r * 1e4) / 1e4 || 0, Math.round(Math.sin(a) * r * 1e4) / 1e4 || 0]; });
  b.add(b.object3d({ source: { extrude: star, depth: .4, bevel: .06 }, material: material('#F2C14E', { metalness: .9, roughness: .3 }) }, { name: 'Extruded star (bevel)', position: [1.4, 0, 0], p: { 'rotation.y': b.ch(-30, [[0, -30], [3, 60]]) } }));
  b.add(b.object3d({ source: { extrude: [[-.5, -.3], [.5, -.3], [.5, .3], [-.5, .3]], depth: .2 }, material: material('#FB7185') }, { name: 'Extruded slab', position: [0, -1.3, 0] }));
  b.add(b.light3d({ type: 'point', p: { intensity: 30, color: '#FFFFFF', distance: 0, decay: 2 } }, { position: [0, 2.5, 3] }));
  b.add(camera(b, {}, [0, .5, 5]));
  return project(b, { group: '3D', title: 'Lathe and extrude', covers: ['lathe', 'extrude', 'extrude bevel', 'point light with shadows'], samples: [0, 1.5, 3] });
}

function lights(): CorpusProject {
  const b = newProject('3d/lights', { name: 'Every light type with shadows', backgroundFill: BLACK });
  b.add(b.object3d({ source: { primitive: 'plane', parameters: { width: 14, height: 10 } }, material: material('#DADADA', { roughness: .8 }), castShadow: false }, { name: 'Floor', position: [0, -1, 0], rotation: [-90, 0, 0] }));
  for (const [index, primitive] of ['box', 'sphere', 'torus', 'cone', 'cylinder'].entries()) {
    b.add(b.object3d({ source: { primitive }, material: material('#FFFFFF', { roughness: .5 }) }, { name: `Caster ${primitive}`, position: [-2.4 + index * 1.2, -.3, 0], scale: [70, 70, 70] }));
  }
  b.add(b.light3d({ type: 'sun', p: { intensity: 1.2, color: '#FFE2B8' } }, { name: 'Sun', position: [-3, 5, 3] }));
  b.add(b.light3d({ type: 'point', p: { intensity: 15, color: '#4C8DFF', distance: 12, decay: 2 } }, { name: 'Point', position: [2.5, 2, 1.5] }));
  b.add(b.light3d({ type: 'spot', p: { intensity: 40, color: '#FF6B1A', angle: 30, penumbra: .5, targetX: -1, targetY: -1, targetZ: 0 } }, { name: 'Spot', position: [-2, 4, 2] }));
  b.add(b.light3d({ type: 'area', p: { intensity: 4, color: '#3FCF8E', width: 2, height: 1, targetX: 0, targetY: 0, targetZ: 0 } }, { name: 'Area (no native shadow)', position: [0, 2, 3] }));
  b.add(b.light3d({ type: 'point', p: { intensity: 10, color: '#F2C14E' } }, { name: 'Fifth shadow light (over the limit of 4)', position: [3, 1, -2] }));
  b.add(camera(b, { environment: { p: { ambient: .3, ambientColor: '#8899FF', exposure: 1.1 } } }, [0, 2.5, 7]));
  return project(b, { group: '3D', title: 'Light types and shadow limits', covers: ['sun, point, spot, area lights', 'hemisphere ambient', 'exposure', 'shadow map limit (first 4 shadow lights)', 'area light LTC without shadows'], samples: [0], alpha: true });
}

function cameras(): CorpusProject[] {
  const scene = (b: Builder) => {
    b.add(b.object3d({ source: { primitive: 'box' }, material: material('#FF6B1A') }, { name: 'Box', position: [-1, 0, 0] }));
    b.add(b.object3d({ source: { primitive: 'sphere' }, material: material('#4C8DFF') }, { name: 'Sphere', position: [1, 0, -1] }));
    b.add(b.light3d({ type: 'sun' }, { position: [-3, 5, 4] }));
  };
  const ortho = newProject('3d/camera-orthographic', { name: 'Orthographic camera', backgroundFill: BLACK });
  scene(ortho);
  ortho.add(camera(ortho, { camera: { projection: 'orthographic', p: { zoom: ortho.ch(1, [[0, 1], [2.5, 1.8]]) } } }, [4, 3, 6]));
  const persp = newProject('3d/camera-animated', { name: 'Animated perspective camera', backgroundFill: BLACK });
  scene(persp);
  const cam = camera(persp, { camera: { p: { fov: persp.ch(30, [[0, 30], [2.5, 75]]), near: .1, far: 50 } }, environment: { background: '#1B2A4A' } }, [0, 1, 6]);
  cam.p['position.x'] = persp.ch(-3, [[0, -3], [2.5, 3]]);
  cam.p.rotation = persp.ch(0, [[0, 0], [2.5, 15]]);
  persp.add(cam);
  return [
    project(ortho, { group: '3D', title: 'Orthographic camera', covers: ['orthographic camera (half-height 3 / zoom)', 'animated zoom'], samples: [0, 1.25, 2.5] }),
    project(persp, { group: '3D', title: 'Animated perspective camera', covers: ['perspective camera', 'animated field of view', 'camera roll from layer rotation', 'environment background', 'near and far'], samples: [0, 1.25, 2.5] }),
  ];
}

function transparency(): CorpusProject {
  const b = newProject('3d/transparency', { name: 'Transparent and double-sided models', backgroundFill: BLACK });
  b.add(b.object3d({ source: { primitive: 'box', parameters: { width: 3, height: 2, depth: .2 } }, material: material('#3FCF8E') }, { name: 'Opaque backdrop', position: [0, 0, -1.5] }));
  b.add(b.object3d({ source: { primitive: 'sphere' }, material: { ...material('#FF6B1A', { opacity: .5 }), doubleSided: true } }, { name: 'Transparent double-sided', position: [-1.1, 0, 0] }));
  b.add(b.object3d({ source: { primitive: 'torus' }, material: { ...material('#4C8DFF', { opacity: .6 }), doubleSided: false } }, { name: 'Transparent single-sided', position: [1.1, 0, 0], p: { 'rotation.x': b.ch(0, [[0, 0], [2.5, 180]]) } }));
  b.add(b.object3d({ source: { primitive: 'plane', parameters: { width: 1.2, height: 1.2 } }, material: { ...material('#F2C14E'), doubleSided: false } }, { name: 'Single-sided plane seen from behind', position: [0, 1.2, 0], rotation: [0, 180, 0] }));
  b.add(b.object3d({ source: { primitive: 'icosahedron' }, material: material('#E8E2CF') }, { name: 'Layer opacity 50%', position: [0, -1.1, .5], p: { opacity: 50 } }));
  const masked = b.object3d({ source: { primitive: 'box' }, material: material('#A73366') }, { name: 'Masked model', position: [2.2, -1, .5] });
  masked.masks = [b.mask('ellipse', { x: 480, y: 270, w: 140, h: 140, feather: 10 })];
  b.add(masked);
  b.add(b.light3d({ type: 'sun' }, { position: [-3, 5, 4] }));
  b.add(camera(b, {}, [0, .5, 6]));
  return project(b, { group: '3D', title: 'Transparency, double-sided, opacity, masks', covers: ['transparent materials', 'double-sided transparency (back then front)', 'single-sided culling', 'layer opacity disables depth write', 'mask on a 3D layer'], samples: [0, 1.25, 2.5], alpha: true });
}

function depthSets(): CorpusProject {
  const b = newProject('3d/depth-sets', { name: 'Depth-set barriers', backgroundFill: BLACK });
  // Bottom set.
  b.add(b.object3d({ source: { primitive: 'box', parameters: { width: 2, height: 2, depth: 2 } }, material: material('#4C8DFF') }, { name: 'Set 1: big box', position: [0, 0, -1] }));
  b.add(b.light3d({ type: 'sun' }, { position: [-3, 5, 4] }));
  b.add(b.layer('shape', { name: 'Hidden 2D (does not end the set)', on: false, d: { w: 640, h: 360, color: '#FFFFFF' } }));
  b.add(b.object3d({ source: { primitive: 'sphere' }, material: material('#FF6B1A') }, { name: 'Set 1: sphere intersecting', position: [.6, .3, 0] }));
  // A visible 2D layer ends the depth set.
  b.add(b.layer('shape', { name: '2D barrier band', d: { w: 640, h: 60, color: '#F2C14E', radius: 0 }, p: { opacity: 80 } }));
  b.add(b.object3d({ source: { primitive: 'torus' }, material: material('#3FCF8E') }, { name: 'Set 2: torus behind but drawn above', position: [0, 0, -3] }));
  const styled = b.layer('group', { name: 'Styled group barrier', p: { opacity: 70 } });
  b.add(b.layer('shape', { name: 'In styled group', group: styled.id, d: { shape: 'ellipse', w: 120, h: 120, color: '#A73366' }, p: { 'position.x': 520, 'position.y': 90 } }));
  b.add(styled);
  b.add(b.object3d({ source: { primitive: 'cone' }, material: material('#E8E2CF') }, { name: 'Set 3: cone', position: [-1.5, -.5, 1] }));
  b.add(camera(b, {}, [0, 1, 6]));
  return project(b, { group: '3D', title: 'Depth sets split by 2D layers and styled groups', covers: ['depth sets', '2D barrier between depth sets', 'styled group barrier', 'hidden layers do not end a set', 'lights illuminate every set', 'ordering back to front by origin depth'], samples: [0], alpha: true });
}

function studioDefaults(): CorpusProject {
  const b = newProject('3d/studio-defaults', { name: 'Studio defaults', backgroundFill: BLACK });
  b.add(b.object3d({ source: { primitive: 'box' }, material: material('#C7C4FF') }, { name: 'Model only' }));
  b.add(b.object3d({ source: { primitive: 'sphere', parameters: { radius: .4 } }, material: material('#FF6B1A') }, { name: 'Second model', position: [1.2, 0, 0] }));
  return project(b, { group: '3D', title: 'Studio defaults (no light, no camera)', covers: ['default sun at (-3, 5, 4), intensity 3, shadows', 'default camera at (3, 2, 6), 45° fov'], samples: [0] });
}

function gltf(): CorpusProject {
  const b = newProject('3d/gltf', { name: 'Generated glTF', backgroundFill: BLACK });
  b.add(b.object3d({ source: { assetId: ref('corpus-model.glb') }, useSourceMaterials: true, material: material('#FFFFFF') }, { name: 'glTF source materials' }));
  b.add(b.object3d({ source: { assetId: ref('corpus-model.glb') }, useSourceMaterials: false, material: material('#7DD3FC', { metalness: .7, roughness: .2 }) }, { name: 'glTF override material', position: [0, -1.6, -1], scale: [60, 60, 60] }));
  b.add(b.light3d({ type: 'sun', p: { intensity: 2.5 } }, { position: [-3, 5, 4] }));
  b.add(camera(b, {}, [0, .6, 5]));
  return project(b, { group: '3D', title: 'Generated glTF: texture, vertex colors, skinning, two clips', covers: ['imported glTF', 'source materials', 'base color texture', 'vertex colors', 'KHR_materials_emissive_strength', 'skinning', 'multi-clip animation', 'material override'], samples: [0, .5, 1, 1.5, 2] });
}

function meshSlots(): CorpusProject {
  const b = newProject('3d/mesh-slots-maps', { name: 'Indexed mesh, slots, and maps', backgroundFill: BLACK });
  const positions = [-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0, -1, -1, -1, 1, -1, -1];
  const indices = [0, 1, 2, 0, 2, 3, 0, 4, 5, 0, 5, 1];
  const mesh = { positions, indices, uvs: [0, 0, 1, 0, 1, 1, 0, 1, 0, 1, 1, 1], groups: [{ start: 0, count: 6, materialIndex: 0 }, { start: 6, count: 6, materialIndex: 1 }] };
  b.add(b.object3d({
    source: { mesh },
    slots: [{ id: 'front', name: 'Front', material: { p: { color: '#FF6B1A', roughness: .4, metalness: 0 } } }, { id: 'floor', name: 'Floor', material: { p: { color: '#4C8DFF', roughness: .6, metalness: .3 } } }],
  }, { name: 'Indexed mesh with two slots', position: [-1.3, 0, 0], p: { 'rotation.y': 25, 'rotation.x': 20 } }));
  b.add(b.object3d({ source: { primitive: 'box' }, material: { p: { color: '#FFFFFF', roughness: .5, metalness: .1 }, maps: { color: ref('still.webp') }, placement: { scaleX: 2, scaleY: 2, offsetX: 0, offsetY: 0, rotation: 15 } } }, { name: 'Color map with placement', position: [1.4, 0, 0], p: { 'rotation.y': 30 } }));
  b.add(b.light3d({ type: 'sun' }, { position: [-3, 5, 4] }));
  b.add(camera(b, {}, [0, .8, 5]));
  return project(b, { group: '3D', title: 'Indexed mesh with material slots; color map with UV placement', covers: ['indexed meshes', 'material slots', 'color map', 'UV placement'], samples: [0] });
}

function special(): CorpusProject[] {
  const missing = newProject('3d/missing-model', { name: 'Missing model', backgroundFill: BLACK });
  missing.add(missing.object3d({ source: { assetId: MISSING_MEDIA } }, { name: 'Model with absent asset' }));
  missing.add(missing.object3d({ source: { primitive: 'sphere' }, material: material('#3FCF8E') }, { name: 'Present model', position: [1.5, 0, 0] }));
  const legacy = newProject('3d/legacy-scene', { name: 'Legacy scene3d layer', backgroundFill: GRADIENT });
  const scene = scene3d.parseScene({
    objects: [
      { id: 'cube', name: 'Cube', source: { primitive: 'box' }, p: { ry: { v: 0, kf: [{ t: 0, v: 0 }, { t: 3, v: 180 }] } }, material: { p: { color: '#FF6B1A' } } },
      { id: 'ring', name: 'Ring', parent: 'cube', source: { primitive: 'torus' }, p: { x: 1.2 }, material: { p: { color: '#4C8DFF' } } },
    ],
    lights: [{ id: 'key', name: 'Key', type: 'sun' }],
    environment: { background: '#101820' },
  });
  // Keyframes inside the legacy scene need ids like any other channel.
  scene.objects[0].p.ry.kf = scene.objects[0].p.ry.kf.map((key: Doc, index: number) => ({ ...key, i: `legacy-k${index}` }));
  legacy.add(legacy.layer('extension', { name: 'Legacy 3D Scene', d: { definition: scene3d.sceneDefinition, version: 1, w: 480, h: 270, params: {}, data: { scene } }, p: { 'position.x': 80, 'position.y': 45 } }));
  const nested = newProject('3d/nested-precomp', { name: '3D inside a precomp', backgroundFill: GRADIENT });
  const inner = nested.composition({ name: '3D precomp', w: 320, h: 180, dur: 3, bg: '#0C0D12' });
  nested.within(inner, () => {
    nested.add(nested.object3d({ source: { primitive: 'torus' }, material: material('#F2C14E', { metalness: .8, roughness: .2 }) }, { name: 'Nested torus', p: { 'rotation.x': nested.ch(0, [[0, 0], [3, 270]]) } }));
    nested.add(nested.light3d({ type: 'point', p: { intensity: 20 } }, { position: [2, 2, 3] }));
    nested.add(camera(nested, {}, [0, 0, 4]));
  });
  nested.add(nested.layer('precomp', { name: '3D precomp A', d: { comp: inner.id, w: 320, h: 180 }, p: { 'position.x': 0, 'position.y': 0 } }));
  nested.add(nested.layer('precomp', { name: '3D precomp B rotated', d: { comp: inner.id, w: 320, h: 180 }, p: { 'position.x': 320, 'position.y': 180, rotation: 10 } }));
  const solo = newProject('3d/solo-hidden', { name: 'Soloed and hidden 3D layers', backgroundFill: BLACK });
  solo.add(solo.object3d({ source: { primitive: 'box' }, material: material('#FF6B1A') }, { name: 'Soloed box', solo: true, position: [-1, 0, 0] }));
  solo.add(solo.object3d({ source: { primitive: 'sphere' }, material: material('#4C8DFF') }, { name: 'Not soloed sphere', position: [1, 0, 0] }));
  solo.add(solo.object3d({ source: { primitive: 'cone' }, material: material('#3FCF8E') }, { name: 'Hidden cone', on: false, position: [0, 1, 0] }));
  solo.add(solo.light3d({ type: 'sun' }, { position: [-3, 5, 4] }));
  solo.add(camera(solo, {}, [0, .5, 5]));
  return [
    project(missing, { group: '3D', title: 'Missing model', covers: ['missing-model orange wireframe box'], samples: [0] }),
    project(legacy, { group: '3D', title: 'Legacy scene3d layer', covers: ['legacy scene3d layer', 'legacy scene background', 'parented legacy objects', 'legacy o.<id> channels'], samples: [0, 1.5, 3] }),
    project(nested, { group: '3D', title: '3D layers inside precomps', covers: ['nested 3D precomps'], samples: [0, 1.5] }),
    project(solo, { group: '3D', title: 'Soloed and hidden 3D layers', covers: ['soloed 3D layers', 'hidden 3D layers'], samples: [0] }),
  ];
}

export function spaceProjects(): CorpusProject[] {
  return [...layers25d(), primitives(), latheExtrude(), lights(), ...cameras(), transparency(), depthSets(), studioDefaults(), gltf(), meshSlots(), ...special()];
}
