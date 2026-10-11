import { describe, expect, test } from 'bun:test';
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { blendModes, captionModel, effects, layerTypes, matteModes, scene3d, transitions } from '../src/catalog';
import { generateCorpus, validateAll } from '../src/generate';
import { corpusGlb } from '../src/glb';
import { FIXTURE_FILES, GENERATED_MEDIA, mediaRefs, MISSING_MEDIA, resolveMedia } from '../src/media';
import { readFloat, straightTopDown, writeFloat } from '../src/output';
import { decodePng, encodePng } from '../src/png';
import { allProjects } from '../src/projects';
import { validateDocument } from '../src/validate';

const projects = allProjects();
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

  test('the generated GLB is structurally valid glTF 2.0', () => {
    const bytes = corpusGlb();
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
    const component: Record<number, number> = { 5121: 1, 5123: 2, 5126: 4 };
    for (const accessor of gltf.accessors) {
      const bufferView = gltf.bufferViews[accessor.bufferView];
      expect(bufferView.byteOffset % 4).toBe(0);
      expect(accessor.count * size[accessor.type]! * component[accessor.componentType]!).toBeLessThanOrEqual(bufferView.byteLength);
      expect(bufferView.byteOffset + bufferView.byteLength).toBeLessThanOrEqual(gltf.buffers[0].byteLength);
    }
    for (const mesh of gltf.meshes) for (const primitive of mesh.primitives) {
      const count = gltf.accessors[primitive.attributes.POSITION].count;
      for (const accessor of Object.values<number>(primitive.attributes)) expect(gltf.accessors[accessor].count).toBe(count);
      if (primitive.indices !== undefined) {
        const indices = gltf.accessors[primitive.indices];
        const bufferView = gltf.bufferViews[indices.bufferView];
        const values = new Uint16Array(bytes.buffer.slice(bytes.byteOffset + binOffset + 8 + bufferView.byteOffset, bytes.byteOffset + binOffset + 8 + bufferView.byteOffset + indices.count * 2));
        expect(Math.max(...values)).toBeLessThan(count);
      }
    }
    expect(gltf.animations).toHaveLength(2);
    expect(gltf.skins).toHaveLength(1);
    const image = gltf.images[0];
    const png = bytes.subarray(binOffset + 8 + gltf.bufferViews[image.bufferView].byteOffset, binOffset + 8 + gltf.bufferViews[image.bufferView].byteOffset + gltf.bufferViews[image.bufferView].byteLength);
    expect(decodePng(png).width).toBeGreaterThan(0);
  });

  test('generated media are listed', () => {
    expect(Object.keys(GENERATED_MEDIA).sort()).toEqual(['corpus-art.svg', 'corpus-gem.obj', 'corpus-model.glb']);
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
      const record = writeFloat(dir, 'f/frame.rgba32f', { width: 2, height: 1, rowOrder: 'top-down', alpha: 'premultiplied', time: 1, encoding: 'test', source: 'test' }, new Uint8Array(pixels.buffer));
      expect(record.data.bytes).toBe(32);
      const read = readFloat(path.join(dir, 'f/frame.rgba32f.json'));
      expect(Array.from(read.pixels)).toEqual(Array.from(pixels));
      expect(read.header).toMatchObject({ format: 'powermove-corpus-rgba32f', version: 1, width: 2, height: 1, channels: 4, sampleType: 'f32', endianness: 'little' });
      Bun.write(path.join(dir, 'f/frame.rgba32f'), new Uint8Array(32));
      expect(() => readFloat(path.join(dir, 'f/frame.rgba32f.json'))).toThrow(/does not match/);
      expect(() => writeFloat(dir, 'g.rgba32f', { width: 3, height: 1, rowOrder: 'top-down', alpha: 'premultiplied', time: 0, encoding: '', source: '' }, new Uint8Array(32))).toThrow();
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
