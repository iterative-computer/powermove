import { objModel, shaderPresets, studioCube } from '../catalog';
import { newProject, type Builder, type Channel, type Value } from '../doc';
import { FIXTURE_FILES, fixtureManifest, ref } from '../media';
import { GRADIENT, project, testCard, type CorpusProject } from './common';

/** Shader-layer uniforms as `sanitizeShaderUniforms` derives them from `@param` annotations. */
export function shaderUniforms(b: Builder, code: string, overrides: Record<string, Value | Channel> = {}): Record<string, Channel> {
  const out: Record<string, Channel> = {};
  for (const match of code.matchAll(/uniform\s+(float|vec2|vec3|vec4|int|bool)\s+(\w+)\s*;\s*(?:\/\/\s*@param\s*([^\n]*))?/g)) {
    const [, type, name, annotation] = match as unknown as [string, string, string, string | undefined];
    if (/^i(Resolution|Time|GlobalTime|Progress|Frame|Mouse)$/.test(name)) continue;
    const values = (annotation || '').trim().split(/\s+/).filter(Boolean);
    let fallback: Value;
    if (type === 'vec3' && values[0]?.startsWith('#')) fallback = values[0];
    else if (type === 'bool') fallback = values[0] === 'true';
    else { const parsed = values[0] === undefined ? 0 : Number.parseFloat(values[0]); fallback = Number.isNaN(parsed) ? 0 : parsed; }
    out[name] = b.ch(fallback);
  }
  for (const [name, value] of Object.entries(overrides)) {
    if (!out[name]) throw new Error(`Shader has no @param uniform ${name}`);
    out[name] = typeof value === 'object' ? value : b.ch(value);
  }
  return out;
}

function solids(): CorpusProject {
  const b = newProject('layers/solid', { name: 'Solids', backgroundFill: GRADIENT });
  b.add(b.layer('solid', { name: 'Full frame', d: { color: '#1F2433' } }));
  b.add(b.layer('solid', { name: 'Rounded (radius ignored at level 1)', d: { color: '#FF6B1A', w: 200, h: 120, radius: 40 }, p: { 'position.x': 60, 'position.y': 40 } }));
  b.add(b.layer('solid', {
    name: 'Rotating translucent', d: { color: '#4C8DFF', w: 160, h: 160 },
    p: { 'position.x': 420, 'position.y': 180, 'anchor.x': 80, 'anchor.y': 80, opacity: 70, rotation: b.ch(0, [[0, 0], [3, 90]]) },
  }));
  b.add(b.layer('solid', { name: 'Skewed', d: { color: '#3FCF8E', w: 120, h: 80 }, p: { 'position.x': 300, 'position.y': 250, skew: 30, 'scale.x': 120, 'scale.y': 80 } }));
  return project(b, { group: 'Layers', title: 'Solid layers', covers: ['solid', 'solid radius stored but ignored', 'skew', 'opacity', 'rotation keyframes'], samples: [0, 1.5, 2.9] });
}

function mediaProjects(): CorpusProject[] {
  const manifest = new Map(fixtureManifest().map(item => [item.file, item]));
  const out: CorpusProject[] = [];
  for (const file of FIXTURE_FILES) {
    const info = manifest.get(file);
    const kind = info?.kind ?? (file.startsWith('still') ? 'image' : file.startsWith('tone') ? 'audio' : 'video');
    if (kind === 'audio') continue;
    const animated = file.startsWith('animated');
    const slug = 'media/' + file.replace(/\./g, '-');
    const b = newProject(slug, { name: file, w: 320, h: 180, dur: animated ? .3 : 2, bg: '#202020' });
    const isVideo = kind === 'video' || animated;
    const layerType = isVideo ? 'video' : 'image';
    // `contain` keeps the full source frame, including alpha halves, inside the composition.
    b.add(b.layer(layerType, { name: file, d: { asset: ref(file), fit: 'contain', w: 320, h: 180 } }));
    const samples = animated ? [.05, .15, .25] : isVideo
      ? [info?.expectedColorAt2?.t ?? .5, info?.expectedColorAt?.t ?? 1.5].sort((a, c) => a - c)
      : [0];
    out.push(project(b, {
      group: 'Media', title: `Fixture ${file}`, covers: [`import ${file}`, isVideo ? 'video layer' : 'image layer'],
      samples, alpha: !!info?.hasAlpha || file.includes('alpha') || animated,
      notes: info?.skipped ? `fixtures.json marks this fixture skipped: ${info.skipped}` : undefined,
    }));
  }
  const b = newProject('media/audio', { name: 'Audio layers', w: 320, h: 180, dur: 2, bg: '#202020' });
  for (const [index, file] of ['tone.wav', 'tone.mp3', 'tone.m4a', 'tone.flac', 'tone.opus'].entries()) {
    const layer = b.layer('audio', { name: file, d: { asset: ref(file), gain: 1 - index * .1, trim: index * .05, fadeIn: .2, fadeOut: .3 } });
    b.add(layer);
  }
  b.add(b.layer('video', { name: 'Embedded audio video', d: { asset: ref('h264-aac.mp4'), fit: 'contain', w: 320, h: 180, embeddedAudio: true, audioGain: b.ch(1, [[0, 0], [1, 1]]) } }));
  out.push(project(b, {
    group: 'Audio', title: 'Audio layers', covers: ['audio layer', 'gain', 'trim', 'fades', 'embedded audio', 'keyframed gain'],
    samples: [.5, 1.5], notes: 'Audio is not rendered by these captures; the evaluated dumps record the audio channels.',
  }));
  return out;
}

function svgScales(): CorpusProject {
  const b = newProject('layers/svg-scales', { name: 'SVG scale buckets', bg: '#F4F1EA' });
  const sizes: Array<[number, number, number]> = [[90, 120, 25], [250, 110, 60], [470, 200, 140]];
  for (const [x, y, scale] of sizes) b.add(b.layer('image', { name: `SVG at ${scale}%`, d: { asset: ref('corpus-art.svg'), fit: 'contain', w: 320, h: 180 }, p: { 'position.x': x, 'position.y': y, 'scale.x': scale, 'scale.y': scale } }));
  b.add(b.layer('image', { name: 'SVG zooming', d: { asset: ref('corpus-art.svg'), fit: 'contain', w: 320, h: 180 }, p: { 'position.x': 520, 'position.y': 90, 'scale.x': b.ch(10, [[0, 10], [2.5, 90]]), 'scale.y': b.ch(10, [[0, 10], [2.5, 90]]) } }));
  return project(b, { group: 'Layers', title: 'SVG images across size buckets', covers: ['image (SVG)', 'SVG raster per size bucket'], samples: [0, 1.2, 2.5] });
}

function shaderLayers(): CorpusProject[] {
  const presets = shaderPresets();
  const out: CorpusProject[] = [];
  for (const [name, code] of Object.entries(presets)) {
    const b = newProject('layers/shader-' + name.toLowerCase().replace(/\s+/g, '-'), { name: `Shader ${name}`, bg: '#000000' });
    b.add(b.layer('shader', { name, d: { code, uniforms: shaderUniforms(b, code) } }));
    out.push(project(b, { group: 'Shader and extension layers', title: `Shader preset ${name}`, covers: ['shader layer', `preset ${name}`, 'iTime', 'iProgress', '@param uniforms'], samples: [0, 1, 2.5], alpha: name === 'Scanline Bars' }));
  }
  const b = newProject('layers/shader-binding-quirks', { name: 'Shader uniform binding quirks', bg: '#000000' });
  const code = `uniform float uAmount; // @param 0.6 0 1
uniform vec2 uOffset;  // @param 0.25 -1 1
uniform int uSteps;    // @param 6 1 12
uniform bool uInvert;  // @param true
uniform vec3 uColor;   // @param #3FCF8E
uniform vec4 uExtra;   // @param 0.5
void main(){
  vec2 p = uv + uOffset;
  float bands = floor(p.x * float(max(uSteps, 1)) * 4.0) / 4.0;
  vec3 c = mix(vec3(0.05), uColor, fract(bands + iTime * 0.2) * uAmount);
  if (uInvert) c = 1.0 - c;
  fragColor = vec4(c + uExtra.rgb * 0.0, 1.0);
}`;
  b.add(b.layer('shader', {
    name: 'Binding quirks', d: { code, uniforms: shaderUniforms(b, code, { uAmount: b.ch(.2, [[0, .2], [2, 1]]), uSteps: 9 }) },
    p: { 'position.x': 40, 'position.y': 20, 'scale.x': 85, 'scale.y': 85 },
  }));
  out.push(project(b, {
    group: 'Shader and extension layers', title: 'Shader uniform binding quirks',
    covers: ['@param float, vec2, int, bool, vec3, vec4', 'vec2/vec4/int stay 0 (compositor binding quirk)', 'animated uniform'],
    samples: [0, 1, 2],
  }));
  return out;
}

function extensionLayers(): CorpusProject[] {
  const params = (b: Builder, definition: typeof studioCube, overrides: Record<string, Value | Channel> = {}) => {
    const out: Record<string, Channel> = {};
    for (const param of definition.params) out[param.k] = b.ch(param.def as Value);
    for (const [key, value] of Object.entries(overrides)) out[key] = typeof value === 'object' ? value : b.ch(value);
    return out;
  };
  const cube = newProject('layers/studio-cube', { name: 'Studio Cube', bg: '#000000' });
  cube.add(cube.layer('extension', {
    name: '3D Studio Cube',
    d: { definition: studioCube.id, version: studioCube.version, w: 640, h: 360, params: params(cube, studioCube, { rotation: cube.ch(0, [[0, 0], [3, 120]]), objectColor: '#FFB36B' }), data: structuredClone(studioCube.defaults) },
  }));
  const cubeTwo = cube.layer('extension', {
    name: 'Studio Cube (small, auto rotate off)',
    d: { definition: studioCube.id, version: studioCube.version, w: 240, h: 180, params: params(cube, studioCube, { autoRotate: false, roundness: .3, metalness: .2, background: '#1B2A4A' }), data: structuredClone(studioCube.defaults) },
    p: { 'position.x': 390, 'position.y': 170 },
  });
  cube.add(cubeTwo);
  const obj = newProject('layers/obj-model', { name: 'Legacy OBJ model', bg: '#000000' });
  obj.add(obj.layer('extension', {
    name: 'OBJ Model',
    d: { definition: objModel.id, version: objModel.version, w: 640, h: 360, params: params(obj, objModel, { rotationY: obj.ch(0, [[0, 0], [3, 180]]) }), data: { ...structuredClone(objModel.defaults as object), assetId: ref('corpus-gem.obj') } },
  }));
  const missing = newProject('layers/extension-missing', { name: 'Missing extension definition', backgroundFill: GRADIENT });
  testCard(missing);
  missing.add(missing.layer('extension', {
    name: 'Unregistered definition',
    d: { definition: 'com.example.absent-layer', version: 3, w: 300, h: 200, params: { amount: missing.ch(4) }, data: { note: 'kept verbatim' } },
    p: { 'position.x': 170, 'position.y': 80 },
  }));
  return [
    project(cube, { group: 'Shader and extension layers', title: 'Studio Cube fragment layer', covers: ['extension fragment layer', 'Studio Cube', 'extension params', 'toggle params as uniform bool'], samples: [0, 1.5, 3] }),
    project(obj, { group: 'Shader and extension layers', title: 'Legacy OBJ mesh layer', covers: ['extension mesh layer', 'legacy OBJ', 'assetField'], samples: [0, 1.5, 3] }),
    project(missing, { group: 'Shader and extension layers', title: 'Missing extension definition', covers: ['missing extension layer checkerboard placeholder'], samples: [0] }),
  ];
}

function nullsAndParenting(): CorpusProject {
  const b = newProject('layers/null-parenting', { name: 'Nulls and parenting', backgroundFill: GRADIENT });
  const rig = b.add(b.layer('null', { name: 'Rig', p: { 'position.x': 320, 'position.y': 180, rotation: b.ch(0, [[0, 0], [3, 60]]), 'scale.x': b.ch(100, [[0, 100], [3, 70]]), 'scale.y': 100 } }));
  const arm = b.add(b.layer('shape', { name: 'Arm', parent: rig.id, d: { w: 160, h: 24, color: '#FF6B1A', radius: 4 }, p: { 'position.x': 80, 'position.y': 0, 'anchor.x': -80, rotation: b.ch(0, [[0, 0], [3, -120]]) } }));
  b.add(b.layer('shape', { name: 'Hand', parent: arm.id, d: { shape: 'ellipse', w: 40, h: 40, color: '#4C8DFF' }, p: { 'position.x': 160, 'position.y': 0, skew: 15 } }));
  b.add(b.layer('text', { name: 'Label', parent: rig.id, d: { text: 'Parented', size: 28, color: '#E8E2CF' }, p: { 'position.x': 0, 'position.y': 60 } }));
  return project(b, { group: 'Layers', title: 'Nulls and parenting chains', covers: ['null', 'parent', 'parent chains with rotation and scale', 'skew under parents'], samples: [0, 1.5, 3] });
}

export function layerProjects(): CorpusProject[] {
  return [solids(), svgScales(), nullsAndParenting(), ...shaderLayers(), ...extensionLayers(), ...mediaProjects()];
}
