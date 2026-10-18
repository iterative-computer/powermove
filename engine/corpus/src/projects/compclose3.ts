/**
 * Motion blur and 2.5D on generated layers (docs/engine/motion.md,
 * docs/engine/composite.md): shader layers, extension fragment layers, and
 * captions, each blurred on its own, inside a blurred group, and inside a
 * blurred precomp; and each with the 3D switch and inside a 3D group.
 */
import { shaderPresets, studioCube } from '../catalog';
import { newProject, type Builder, type Channel, type Value } from '../doc';
import { GRADIENT, project, type CorpusProject } from './common';
import { shaderUniforms } from './layers';

const DARK = { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#14161C', position: 0 }] };

function cubeParams(b: Builder, overrides: Record<string, Value | Channel> = {}): Record<string, Channel> {
  const out: Record<string, Channel> = {};
  for (const param of studioCube.params) out[param.k] = b.ch(param.def as Value);
  for (const [key, value] of Object.entries(overrides)) out[key] = typeof value === 'object' ? value : b.ch(value);
  return out;
}

const CUES: Array<[number, number, string, boolean?]> = [
  [0, 1.5, 'Captions in motion', true],
  [1.5, 3, 'and in depth', true],
];

function blurGenerated(): CorpusProject {
  const b = newProject('motion/blur-generated', { name: 'Motion blur: shader, extension, captions', w: 480, h: 270, dur: 3, backgroundFill: GRADIENT });
  const presets = shaderPresets();
  const bars = presets['Scanline Bars']!;
  const aurora = presets['Aurora Field']!;
  // A translucent shader layer sliding across (its alpha is the preset's).
  b.add(b.layer('shader', { name: 'Scanline bars slide (blur)', mblur: true, d: { code: bars, uniforms: shaderUniforms(b, bars) }, p: { 'position.x': b.ch(-120, [[0, -120], [2, 80]]), 'position.y': 10, 'scale.x': 45, 'scale.y': 45 } }));
  // An extension fragment layer that moves and spins inside the shutter.
  b.add(b.layer('extension', {
    name: 'Studio Cube spin (blur)', mblur: true,
    d: { definition: studioCube.id, version: studioCube.version, w: 160, h: 120, params: cubeParams(b, { autoRotate: false, rotation: b.ch(0, [[0, 0], [2, 240]]), background: '#1B2A4A' }), data: structuredClone(studioCube.defaults) },
    p: { 'position.x': b.ch(300, [[0, 300], [2, 220]]), 'position.y': 20, rotation: b.ch(0, [[0, 0], [2, 60]]) },
  }));
  b.add(b.layer('captions', { name: 'Captions rise (blur)', mblur: true, d: b.captions(CUES, { placement: 'bottom' }), p: { 'position.y': b.ch(40, [[0, 40], [2, -60]]) } }));
  // A shader layer moving with its blurred group.
  const group = b.layer('group', { name: 'Blurred group with a shader', mblur: true, p: { 'position.x': b.ch(0, [[0, 0], [2, 160]]), opacity: 90 } });
  b.add(b.layer('shader', { name: 'Grouped aurora', group: group.id, d: { code: aurora, uniforms: shaderUniforms(b, aurora) }, p: { 'position.x': 10, 'position.y': 150, 'scale.x': 25, 'scale.y': 25, rotation: b.ch(0, [[0, 0], [2, 90]]) } }));
  b.add(group);
  // Generated layers moving inside a blurred precomp.
  const inner = b.composition({ name: 'Inner generated', w: 200, h: 120, dur: 3, bg: '#000000' });
  b.within(inner, () => {
    b.add(b.layer('shader', { name: 'Inner bars', d: { code: bars, uniforms: shaderUniforms(b, bars) }, p: { 'position.x': b.ch(-80, [[0, -80], [3, 40]]), 'scale.x': 40, 'scale.y': 40 } }));
    b.add(b.layer('extension', {
      name: 'Inner cube',
      d: { definition: studioCube.id, version: studioCube.version, w: 90, h: 70, params: cubeParams(b, { autoRotate: false, rotation: b.ch(0, [[0, 0], [3, 300]]) }), data: structuredClone(studioCube.defaults) },
      p: { 'position.x': 100, 'position.y': b.ch(10, [[0, 10], [3, 50]]) },
    }));
  });
  b.add(b.layer('precomp', { name: 'Precomp of generated layers (blur)', mblur: true, d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': b.ch(260, [[0, 260], [2, 200]]), 'position.y': 140, rotation: b.ch(0, [[0, 0], [2, 30]]) } }));
  return project(b, {
    group: 'Motion', title: 'Motion blur on shader, extension, and caption layers',
    covers: ['shader layer motion blur', 'extension fragment layer motion blur', 'captions motion blur', 'shader in a blurred group', 'shader and extension in a blurred precomp'],
    samples: [0.5, 1.3], alpha: true, blurSamples: [3, 6],
  });
}

function spaceGenerated(): CorpusProject {
  const b = newProject('space25d/generated', { name: '2.5D shader, extension, captions', w: 640, h: 360, dur: 3, backgroundFill: DARK });
  const presets = shaderPresets();
  const ember = presets['Ember Gradient']!;
  const bars = presets['Scanline Bars']!;
  b.add(b.layer('shader', { name: 'Shader card (rotation Y)', threeD: true, d: { code: ember, uniforms: shaderUniforms(b, ember) }, p: { 'position.x': 30, 'position.y': 20, 'scale.x': 35, 'scale.y': 35, 'rotation.y': b.ch(-50, [[0, -50], [2.5, 50]]), perspective: 45 } }));
  b.add(b.layer('extension', {
    name: 'Studio Cube (rotation X, Z)', threeD: true,
    d: { definition: studioCube.id, version: studioCube.version, w: 200, h: 140, params: cubeParams(b, { autoRotate: false, rotation: 30 }), data: structuredClone(studioCube.defaults) },
    p: { 'position.x': 330, 'position.y': 30, 'rotation.x': b.ch(50, [[0, 50], [2.5, -30]]), 'position.z': b.ch(-120, [[0, -120], [2.5, 100]]) },
  }));
  b.add(b.layer('captions', { name: 'Captions (orientation Y)', threeD: true, d: b.captions(CUES, { placement: 'bottom' }), p: { 'orientation.y': 25, 'rotation.x': b.ch(10, [[0, 10], [2.5, -20]]) } }));
  // Generated layers inside a 3D group, with a blurred member.
  const rig = b.layer('group', { name: '3D group of generated layers', threeD: true, p: { 'position.x': 320, 'position.y': 180, 'anchor.x': 320, 'anchor.y': 180, 'rotation.y': b.ch(35, [[0, 35], [2.5, -35]]), perspective: 55 } });
  b.add(b.layer('shader', { name: 'Grouped bars', group: rig.id, d: { code: bars, uniforms: shaderUniforms(b, bars) }, p: { 'position.x': 60, 'position.y': 200, 'scale.x': 30, 'scale.y': 30 } }));
  b.add(b.layer('extension', {
    name: 'Grouped cube (Z, blur)', threeD: true, mblur: true, group: rig.id,
    d: { definition: studioCube.id, version: studioCube.version, w: 140, h: 100, params: cubeParams(b, { autoRotate: false, rotation: b.ch(0, [[0, 0], [2.5, 200]]) }), data: structuredClone(studioCube.defaults) },
    p: { 'position.x': 300, 'position.y': 220, 'position.z': -60 },
  }));
  b.add(rig);
  return project(b, {
    group: 'Space', title: '2.5D shader, extension, and caption layers',
    covers: ['3D shader layer', '3D extension fragment layer', '3D captions', 'generated layers in a 3D group', 'blurred extension in a 3D group'],
    samples: [0, 1.25, 2.5], alpha: true,
  });
}

export function compclose3Projects(): CorpusProject[] {
  return [blurGenerated(), spaceGenerated()];
}
