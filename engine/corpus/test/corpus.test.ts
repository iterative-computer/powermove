import { describe, expect, test } from 'bun:test';
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import vm from 'node:vm';
import path from 'node:path';
import { animatorPresets, blendModes, captionModel, effects, layerTypes, makeAnimator, matteModes, scene3d, transitions } from '../src/catalog';
import { generateCorpus, validateAll } from '../src/generate';
import { corpusGlb, corpusMaterialsGlb } from '../src/glb';
import { LEGACY_GLOBAL, RETIMING_MODULE, TEXT_ANIMATION_MODULE, legacyScript } from '../src/inject';
import { FIXTURE_FILES, GENERATED_MEDIA, mediaRefs, MISSING_MEDIA, resolveMedia } from '../src/media';
import { readFloat, straightTopDown, writeFile, writeFloat, writePng } from '../src/output';
import { decodePng, encodePng } from '../src/png';
import { allProjects } from '../src/projects';
import { validateDocument } from '../src/validate';
import { verifyCapture } from '../src/verify';

const projects = allProjects();
/** Run the legacy helper bundle in a fresh context, as the renderer does. */
const bundle = async () => {
  const context: Record<string, any> = {};
  context.globalThis = context;
  expect(vm.runInNewContext(await legacyScript(), context)).toBe(true);
  return context[LEGACY_GLOBAL];
};
const temp = (label: string) => mkdtempSync(path.join(os.tmpdir(), `pm-corpus-${label}-`));

/** Every value of `key` anywhere in the documents, across compositions and nested data. */
function collect(predicate: (node: any) => unknown[] | undefined): Set<unknown> {
  const out = new Set<unknown>();
  const visit = (node: any) => {
    if (Array.isArray(node)) { node.forEach(visit); return; }
    if (!node || typeof node !== 'object') return;
    for (const value of predicate(node) ?? []) out.add(value);
    Object.values(node).forEach(visit);
  };
  projects.forEach(item => visit(item.doc));
  return out;
}

const channelValues = (value: any): unknown[] => value && typeof value === 'object' && Array.isArray(value.kf) ? [value.v, ...value.kf.map((key: any) => key.v)] : [value];
const isLayer = (node: any) => typeof node.type === 'string' && typeof node.id === 'string' && 'p' in node && 'fx' in node;

describe('synthetic documents', () => {
  test('every document is a fixed point of sanitizeProject', () => {
    expect(validateAll(projects)).toEqual([]);
  });

  test('the validator catches drops, rewrites, and duplicate keyframe ids', () => {
    const doc = structuredClone(projects[0]!.doc);
    doc.layers[0].p.opacity.v = 'not a number';
    doc.layers[0].bogus = { nested: true };
    expect(validateDocument(doc).map(issue => issue.message).join('\n')).toMatch(/became|dropped/);
    const dup = structuredClone(projects.find(item => item.id === 'animation/interpolation')!.doc);
    const keyed = dup.layers.filter((layer: any) => layer.p['position.x'].kf.length > 1);
    keyed[1].p['position.x'].kf[0].i = keyed[0].p['position.x'].kf[0].i;
    expect(validateDocument(dup).some(issue => /duplicate keyframe id/.test(issue.message))).toBe(true);
  });

  test('project ids are unique, sampled inside their duration, and name their appendix B group', () => {
    const ids = projects.map(item => item.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const item of projects) {
      expect(item.samples.length).toBeGreaterThan(0);
      for (const t of item.samples) { expect(t).toBeGreaterThanOrEqual(0); expect(t).toBeLessThanOrEqual(item.doc.dur); }
      expect(item.group.length).toBeGreaterThan(0);
      expect(item.covers.length).toBeGreaterThan(0);
    }
  });

  test('every layer type is used', () => {
    const used = collect(node => isLayer(node) ? [node.type] : undefined);
    expect(layerTypes.filter(type => !used.has(type))).toEqual([]);
  });

  test('all 16 effects and 4 transitions are used', () => {
    expect(effects).toHaveLength(16);
    expect(transitions).toHaveLength(4);
    const fx = collect(node => Array.isArray(node.fx) ? node.fx.map((item: any) => item.type) : undefined);
    const tr = collect(node => isLayer(node) ? [node.transitionIn?.type, node.transitionOut?.type].filter(Boolean) : undefined);
    expect(effects.map(e => e.id).filter(id => !fx.has(id))).toEqual([]);
    expect(transitions.map(t => t.id).filter(id => !tr.has(id))).toEqual([]);
  });

  test('all 9 blend modes and 4 matte modes are used', () => {
    expect(blendModes).toHaveLength(9);
    expect(matteModes).toHaveLength(4);
    const blends = collect(node => isLayer(node) ? channelValues(node.blend) : undefined);
    const mattes = collect(node => isLayer(node) && node.matteSource ? channelValues(node.matteMode ?? 'alpha') : undefined);
    expect(blendModes.filter(mode => !blends.has(mode))).toEqual([]);
    expect(matteModes.filter(mode => !mattes.has(mode))).toEqual([]);
  });

  test('3D roles, light types, primitives, and projections are covered', () => {
    const definitions = collect(node => isLayer(node) && node.type === 'extension' ? [node.d.definition] : undefined);
    for (const id of [...Object.values(scene3d.definitions), scene3d.sceneDefinition]) expect(definitions.has(id)).toBe(true);
    const lights = collect(node => isLayer(node) && node.d?.data?.light ? [node.d.data.light.type] : undefined);
    expect([...lights].sort()).toEqual(['area', 'point', 'spot', 'sun']);
    const primitives = collect(node => node.source?.primitive ? [node.source.primitive] : undefined);
    for (const kind of ['box', 'sphere', 'plane', 'cylinder', 'cone', 'torus', 'capsule', 'icosahedron']) expect(primitives.has(kind)).toBe(true);
    const sources = collect(node => node.source && typeof node.source === 'object' ? Object.keys(node.source) : undefined);
    for (const kind of ['lathe', 'extrude', 'mesh', 'assetId']) expect(sources.has(kind)).toBe(true);
    const projections = collect(node => node.camera?.projection ? [node.camera.projection] : undefined);
    expect(projections.has('orthographic')).toBe(true);
  });

  test('masks, track mattes, text animators, captions, paths, precomps, adjustment, 2.5D, retiming, motion blur, backgrounds', () => {
    const has = (fn: (node: any) => boolean) => collect(node => fn(node) ? [true] : undefined).has(true);
    expect(has(node => isLayer(node) && node.masks.some((mask: any) => mask.path))).toBe(true);
    expect(has(node => isLayer(node) && node.masks.some((mask: any) => mask.mode === 'subtract'))).toBe(true);
    expect(has(node => isLayer(node) && node.type === 'text' && node.d.animators?.length)).toBe(true);
    expect(has(node => isLayer(node) && node.type === 'text' && node.d.styles?.length)).toBe(true);
    expect(has(node => isLayer(node) && node.type === 'captions' && node.d.cues.some((cue: any) => cue.words?.length))).toBe(true);
    expect(has(node => isLayer(node) && node.type === 'shape' && node.d.paths?.length)).toBe(true);
    expect(has(node => isLayer(node) && node.type === 'precomp')).toBe(true);
    expect(has(node => isLayer(node) && node.type === 'adjustment' && node.fx.length)).toBe(true);
    expect(has(node => isLayer(node) && node.threeD && node.type !== 'extension')).toBe(true);
    expect(has(node => isLayer(node) && node.d?.timeRemap)).toBe(true);
    expect(has(node => isLayer(node) && node.mblur === true)).toBe(true);
    const backgrounds = collect(node => node.backgroundFill ? [node.backgroundFill.type] : undefined);
    expect([...backgrounds].sort()).toEqual(['linear', 'none', 'radial', 'solid']);
  });

  test('every caption preset is authored by id', () => {
    expect(captionModel.presets.length).toBeGreaterThan(1);
    const used = collect(node => isLayer(node) && node.type === 'captions' ? [node.d.style.preset] : undefined);
    expect(captionModel.presets.filter(id => !used.has(id))).toEqual([]);
    expect(projects.filter(item => item.id.startsWith('captions/preset-')).map(item => item.id.slice('captions/preset-'.length)).sort()).toEqual([...captionModel.presets].sort());
  });

  test('every fixture video and image is imported somewhere, and every media reference resolves', () => {
    const refs = new Set(projects.flatMap(item => [...mediaRefs(item.doc)]));
    expect(FIXTURE_FILES.filter(file => !refs.has(file))).toEqual([]);
    const dir = temp('media');
    try {
      generateCorpus(dir, []);
      for (const name of refs) {
        if (`media:${name}` === MISSING_MEDIA) continue;
        const source = resolveMedia(name, dir);
        expect(source.bytes).toBeGreaterThan(0);
      }
    } finally { rmSync(dir, { recursive: true, force: true }); }
  });
});

describe('generation', () => {
  test('is deterministic byte for byte', () => {
    const a = temp('a'), b = temp('b');
    try {
      generateCorpus(a, projects);
      generateCorpus(b, allProjects());
      const files = (root: string): string[] => readdirSync(root, { recursive: true }).map(String).filter(file => statSync(path.join(root, file)).isFile()).sort();
      expect(files(a)).toEqual(files(b));
      for (const file of files(a)) expect(readFileSync(path.join(b, file)).equals(readFileSync(path.join(a, file)))).toBe(true);
      expect(files(a)).toContain('index.json');
      expect(files(a).filter(file => file.endsWith('document.json'))).toHaveLength(projects.length);
    } finally { rmSync(a, { recursive: true, force: true }); rmSync(b, { recursive: true, force: true }); }
  });

  /** Container, accessor bounds, attribute counts, and index ranges of a generated GLB. */
  const checkGlb = (bytes: Uint8Array) => {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    expect(view.getUint32(0, true)).toBe(0x46546c67); // 'glTF'
    expect(view.getUint32(4, true)).toBe(2);
    expect(view.getUint32(8, true)).toBe(bytes.length);
    const jsonLength = view.getUint32(12, true);
    expect(view.getUint32(16, true)).toBe(0x4e4f534a); // 'JSON'
    const gltf = JSON.parse(new TextDecoder().decode(bytes.subarray(20, 20 + jsonLength)));
    const binOffset = 20 + jsonLength;
    const binLength = view.getUint32(binOffset, true);
    expect(view.getUint32(binOffset + 4, true)).toBe(0x004e4942); // 'BIN\0'
    expect(gltf.asset.version).toBe('2.0');
    expect(gltf.buffers[0].byteLength).toBeLessThanOrEqual(binLength);
    const size: Record<string, number> = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4, MAT4: 16 };
    const component: Record<number, number> = { 5120: 1, 5121: 1, 5122: 2, 5123: 2, 5126: 4 };
    for (const accessor of gltf.accessors) {
      const bufferView = gltf.bufferViews[accessor.bufferView];
      const element = size[accessor.type]! * component[accessor.componentType]!;
      const stride = bufferView.byteStride ?? element;
      expect(bufferView.byteOffset % 4).toBe(0);
      expect(stride % 4 === 0 || !bufferView.target || bufferView.target === 34963).toBe(true);
      expect((accessor.count - 1) * stride + element).toBeLessThanOrEqual(bufferView.byteLength);
      expect(bufferView.byteOffset + bufferView.byteLength).toBeLessThanOrEqual(gltf.buffers[0].byteLength);
    }
    const bin = (index: number) => { const bufferView = gltf.bufferViews[index]; return bytes.subarray(binOffset + 8 + bufferView.byteOffset, binOffset + 8 + bufferView.byteOffset + bufferView.byteLength); };
    for (const mesh of gltf.meshes) for (const primitive of mesh.primitives) {
      const count = gltf.accessors[primitive.attributes.POSITION].count;
      for (const accessor of Object.values<number>(primitive.attributes)) expect(gltf.accessors[accessor].count).toBe(count);
      for (const target of primitive.targets ?? []) for (const accessor of Object.values<number>(target)) expect(gltf.accessors[accessor].count).toBe(count);
      if ((primitive.mode ?? 4) === 4) expect((primitive.indices !== undefined ? gltf.accessors[primitive.indices].count : count) % 3).toBe(0);
      if (primitive.indices !== undefined) {
        const indices = gltf.accessors[primitive.indices];
        const values = new Uint16Array(bin(indices.bufferView).slice(0, indices.count * 2).buffer);
        expect(Math.max(...values)).toBeLessThan(count);
      }
      expect(gltf.materials[primitive.material]).toBeDefined();
    }
    for (const image of gltf.images) expect(bin(image.bufferView).length).toBeGreaterThan(0);
    for (const image of gltf.images.filter((item: any) => item.mimeType === 'image/png')) expect(decodePng(bin(image.bufferView)).width).toBeGreaterThan(0);
    return gltf;
  };

  test('the generated GLBs are structurally valid glTF 2.0', () => {
    const model = checkGlb(corpusGlb());
    expect(model.animations).toHaveLength(2);
    expect(model.skins).toHaveLength(1);
    const materials = checkGlb(corpusMaterialsGlb());
    expect(materials.extensionsUsed).toEqual([
      'EXT_materials_bump', 'EXT_mesh_gpu_instancing', 'EXT_texture_avif', 'EXT_texture_webp', 'KHR_lights_punctual', 'KHR_materials_anisotropy',
      'KHR_materials_clearcoat', 'KHR_materials_dispersion', 'KHR_materials_ior', 'KHR_materials_iridescence', 'KHR_materials_sheen', 'KHR_materials_specular',
      'KHR_materials_transmission', 'KHR_materials_unlit', 'KHR_materials_volume', 'KHR_mesh_quantization', 'KHR_texture_transform',
    ]);
    const modes = new Set(materials.meshes.flatMap((mesh: any) => mesh.primitives.map((primitive: any) => primitive.mode ?? 4)));
    expect([...modes].sort()).toEqual([0, 2, 3, 4]);
    expect(materials.meshes.some((mesh: any) => mesh.primitives.some((primitive: any) => primitive.targets?.length))).toBe(true);
    expect(materials.meshes.some((mesh: any) => mesh.primitives.some((primitive: any) => primitive.attributes.NORMAL === undefined && (primitive.mode ?? 4) === 4))).toBe(true);
    expect(materials.animations.map((animation: any) => animation.channels[0].target.path)).toEqual(['weights']);
  });

  test('generated media are listed', () => {
    expect(Object.keys(GENERATED_MEDIA).sort()).toEqual(['corpus-art.svg', 'corpus-gem.obj', 'corpus-materials.glb', 'corpus-model.glb']);
  });
});

describe('capture formats', () => {
  test('PNG encodes and decodes losslessly with text chunks', () => {
    const width = 7, height = 5, data = new Uint8Array(width * height * 4);
    for (let i = 0; i < data.length; i++) data[i] = (i * 37 + (i >> 2) * 11) & 0xff;
    const bytes = encodePng({ width, height, channels: 4, data }, { Source: 'test' });
    const decoded = decodePng(bytes);
    expect([decoded.width, decoded.height, decoded.channels]).toEqual([width, height, 4]);
    expect(Buffer.from(decoded.data).equals(Buffer.from(data))).toBe(true);
    expect(Buffer.from(bytes).includes(Buffer.from('Source\0test'))).toBe(true);
  });

  test('float RGBA round-trips through its header, and corruption is detected', () => {
    const dir = temp('float');
    try {
      const pixels = new Float32Array([0, .25, .5, 1, 1.5, -0.125, 1e-7, .75]);
      const record = writeFloat(dir, 'f/frame.rgba32f', { width: 2, height: 1, rowOrder: 'top-down', alpha: 'premultiplied', time: 1, sourcePrecision: 'binary32', encoding: 'test', source: 'test' }, new Uint8Array(pixels.buffer));
      expect(record.data.bytes).toBe(32);
      const read = readFloat(path.join(dir, 'f/frame.rgba32f.json'));
      expect(Array.from(read.pixels)).toEqual(Array.from(pixels));
      expect(read.header).toMatchObject({ format: 'powermove-corpus-rgba32f', version: 1, width: 2, height: 1, channels: 4, sampleType: 'f32', endianness: 'little' });
      writeFileSync(path.join(dir, 'f/frame.rgba32f'), new Uint8Array(32));
      expect(() => readFloat(path.join(dir, 'f/frame.rgba32f.json'))).toThrow(/does not match/);
      expect(() => writeFloat(dir, 'g.rgba32f', { width: 3, height: 1, rowOrder: 'top-down', alpha: 'premultiplied', time: 0, sourcePrecision: 'binary32', encoding: '', source: '' }, new Uint8Array(32))).toThrow();
    } finally { rmSync(dir, { recursive: true, force: true }); }
  });

  test('straight-alpha conversion matches the exporter (flip rows, unpremultiply, truncate)', () => {
    // Bottom-up premultiplied: row 0 is the bottom row.
    const px = new Uint8Array([
      10, 20, 30, 255, 0, 0, 0, 0,
      64, 32, 16, 128, 255, 255, 255, 255,
    ]);
    const out = straightTopDown(px, 2, 2);
    expect(Array.from(out)).toEqual([
      127, 63, 31, 128, 255, 255, 255, 255,
      10, 20, 30, 255, 0, 0, 0, 0,
    ]);
  });
});

describe('renderer injection', () => {
  test('the bundled text-animation module poses glyphs exactly as the source module does', async () => {
    const bundled = await bundle();
    const source = await import(TEXT_ANIMATION_MODULE);
    // A stub PM: channels evaluate to their static value, as the evaluator does for a channel without keys.
    const PM = { evP: (_layer: unknown, p: any) => (p && typeof p === 'object' && 'v' in p ? p.v : p) };
    let n = 0;
    const P = (v: unknown) => ({ v, kf: [], expr: null, id: `p${n++}` });
    const chars = 'Ab cd'.split('');
    const layout = {
      characters: chars.map((text, index) => ({ text, x: index * 20, y: 0, w: 18, line: 0, lineUnit: 0, word: index < 2 ? 0 : 1, sourceStart: index, index })),
      words: [{ x: 0, w: 38 }, { x: 60, w: 38 }], lines: [{ x: 0, w: 98 }],
    };
    for (const preset of ['custom-range', 'wave', 'pop']) {
      const animator = makeAnimator(P, 'ta1', preset, animatorPresets.find(item => item.id === preset), 40);
      const layer = { type: 'text', from: 0, d: { size: 40, color: '#ffffff', align: 'left', animators: [animator], styles: [{ id: 's1', start: 1, end: 3, p: { color: P('#ff0000'), size: P(48) } }] } };
      for (const time of [0, .3, 1.1]) {
        const expected = source.animatedGlyphs(PM, layer, time, layer.d, layout);
        const actual = bundled.animatedGlyphs(PM, layer, time, layer.d, layout);
        expect(JSON.parse(JSON.stringify(actual))).toEqual(JSON.parse(JSON.stringify(expected)));
        expect(expected).toHaveLength(chars.length);
      }
    }
  });
});

describe('bundled retiming and Blender requests', () => {
  /** A stub PM: channels evaluate linearly between their keys, as the evaluator does for linear keys. */
  const evaluate = (p: any, t: number) => {
    if (!p || typeof p !== 'object' || !('v' in p)) return p;
    const kf = p.kf ?? [];
    if (!kf.length) return p.v;
    if (t <= kf[0].t) return kf[0].v;
    for (let i = 1; i < kf.length; i++) if (t <= kf[i].t) return kf[i - 1].v + (kf[i].v - kf[i - 1].v) * (t - kf[i - 1].t) / (kf[i].t - kf[i - 1].t);
    return kf.at(-1).v;
  };
  const makePM = (proj: any) => ({
    proj, scope: [] as any[], animVersion: () => 1,
    evP: (_layer: unknown, p: any, t: number) => evaluate(p, t),
    active: (layer: any, t: number) => t >= layer.from && t < layer.from + layer.dur,
  });

  test('sourceTime matches the source module for trims, speed ramps, and remaps', async () => {
    const bundled = await bundle();
    const source = await import(RETIMING_MODULE);
    const proj = { fps: 30 };
    const layers = [
      { type: 'precomp', from: .5, dur: 4, d: { trim: .25, speed: 2 } },
      { type: 'video', from: 0, dur: 4, d: { trim: .1, speed: { v: 1, kf: [{ t: 0, v: .5 }, { t: 2, v: 3 }], expr: null } } },
      { type: 'precomp', from: 0, dur: 4, d: { timeRemap: true, sourceTime: { v: 0, kf: [{ t: 0, v: 3 }, { t: 3, v: 0 }], expr: null } } },
    ];
    for (const layer of layers) for (const t of [0, .4, 1.33, 2.9, 3.7]) {
      expect(bundled.sourceTime(makePM(proj), layer, t)).toBe(source.sourceTime(makePM(proj), layer, t));
    }
  });

  test('blenderFrameRequests lists nested compositions and shutter samples in order', async () => {
    const bundled = await bundle();
    const object = (id: string, extra: any = {}) => ({ id, type: 'extension', from: 0, dur: 3, mblur: false, d: { definition: scene3d.definitions.object, data: { object: {} } }, ...extra });
    const inner = { compId: 'inner', w: 320, h: 180, fps: 24, render3d: { enabled: true }, layers: [object('a')] };
    const proj = {
      compId: 'root', w: 640, h: 360, fps: 30, shutter: .5, render3d: { enabled: true }, comps: { inner },
      layers: [object('b', { mblur: true }), { id: 'p', type: 'precomp', from: 0, dur: 3, d: { comp: 'inner', w: 320, h: 180 } }],
    };
    const requests = bundled.blenderFrameRequests(makePM(proj), 1, proj, { shutter: .5 });
    const roots = requests.filter((request: any) => request.comp === proj);
    // The frame time, then ten shutter samples spread over 0.5 / 30 s.
    expect(roots.map((request: any) => request.time)).toEqual([1, ...Array.from({ length: 10 }, (_, i) => 1 + ((i + .5) / 10 - .5) * (.5 / 30))]);
    expect(roots.every((request: any) => request.width === 640 && request.height === 360)).toBe(true);
    const nested = requests.filter((request: any) => request.comp === inner);
    expect(nested).toHaveLength(11);
    expect(nested.every((request: any) => request.width === 320 && request.height === 180)).toBe(true);
    proj.render3d.enabled = false;
    expect(bundled.blenderFrameRequests(makePM(proj), 1, proj, {}).every((request: any) => request.comp === inner)).toBe(true);
  });

  test('the bundle carries the legacy sources and none of the renderer or media modules', async () => {
    const script = await legacyScript();
    const modules = [...script.matchAll(/^ {2}\/\/ (.+)$/gm)].map(match => match[1]!).filter(file => !file.endsWith('legacy-entry.ts'));
    expect(modules.map(file => file.replace(/^.*\/src\/renderer\/src\//, ''))).toEqual([
      'legacy/core/text-animation.ts', 'legacy/core/content-properties.ts', 'legacy/core/retiming.ts', 'legacy/core/frame-preparation.ts',
    ]);
  });
});

describe('capture verification', () => {
  test('accepts a consistent capture and reports corruption, missing, and unlisted files', () => {
    const dir = temp('verify');
    try {
      const png = writePng(dir, 'projects/a/frames/t0.png', 2, 1, new Uint8Array([1, 2, 3, 255, 4, 5, 6, 255]), {});
      const float = writeFloat(dir, 'projects/a/frames/t0.rgba32f', { width: 2, height: 1, rowOrder: 'top-down', alpha: 'premultiplied', time: 0, sourcePrecision: 'binary16', encoding: '', source: '' }, new Uint8Array(32));
      const evalRecord = writeFile(dir, 'projects/a/eval/t0.json', JSON.stringify({ format: 'powermove-corpus-eval', version: 1, time: 0 }));
      writeFile(dir, 'projects/a/project.json', '{}');
      const asset = writeFile(dir, 'projects/a/blender/assets/x.png', new Uint8Array([9, 9]));
      const blender = writeFile(dir, 'projects/a/blender/t0.json', JSON.stringify({
        format: 'powermove-corpus-blender', version: 1, time: 0,
        final: { snapshots: [{ request: { comp: 'root', time: 0, preview: false }, snapshot: {}, assets: [{ id: 't', name: 'x.png', ...asset }] }] }, preview: { snapshots: [] },
      }));
      const sample = { time: 0, width: 2, height: 1, png, float, eval: evalRecord, blender };
      const bytes = png.bytes + float.data.bytes + float.header.bytes + evalRecord.bytes + blender.bytes + asset.bytes;
      writeFile(dir, 'manifest.json', JSON.stringify({
        format: 'powermove-engine-corpus', version: 1, projects: [{ id: 'a', dir: 'projects/a', blender: { assets: [asset] }, samples: [sample] }],
        totals: { projects: 1, frames: 1, floatFrames: 1, evalDumps: 1, blenderDumps: 1, failedSamples: 0, bytes },
      }));
      expect(verifyCapture(dir)).toEqual({ problems: [], checked: 6, unlisted: [] });
      writeFileSync(path.join(dir, 'projects/a/frames/t0.png'), new Uint8Array(png.bytes));
      writeFileSync(path.join(dir, 'stray.bin'), 'x');
      rmSync(path.join(dir, 'projects/a/eval/t0.json'));
      const result = verifyCapture(dir);
      expect(result.problems.some(problem => /t0\.png does not match its SHA-256/.test(problem))).toBe(true);
      expect(result.problems.some(problem => /t0\.png does not decode/.test(problem))).toBe(true);
      expect(result.problems.some(problem => /t0\.json is missing/.test(problem))).toBe(true);
      expect(result.unlisted).toEqual(['stray.bin']);
    } finally { rmSync(dir, { recursive: true, force: true }); }
  });
});
