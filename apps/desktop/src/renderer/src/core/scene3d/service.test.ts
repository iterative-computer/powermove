import { afterEach, describe, expect, it, vi } from 'vitest';
import * as THREE from 'three';
import { makePM } from '../../legacy/__tests__/make-pm';
import { editScene } from './operations';
import { LAYER3D_DEFINITIONS } from './layers';
import { compositionRuntime, disposeSceneRuntimes } from './service';
import { install as installCompositor } from '../../legacy/gl/compositor';

const editors: any[] = [], sourceMeshes: THREE.Mesh[] = [];
afterEach(() => {
  vi.unstubAllGlobals(); vi.restoreAllMocks();
  for (const PM of editors.splice(0)) disposeSceneRuntimes(PM);
  for (const source of sourceMeshes.splice(0)) { source.geometry.dispose(); (source.material as THREE.Material).dispose(); }
});

function editor(count = 1, source: any = { primitive: 'box' }, maps: any = {}) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: 'Runtime cache', w: 320, h: 180, dur: 3 }); PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
  PM.assets.get = (id: string) => PM.assets.map.get(id);
  for (let i = 0; i < count; i++) expect(editScene(PM, { operation: 'add_object', object: {
    id: `model${i}`, source, material: { maps },
  } }).ok).toBe(true);
  editors.push(PM); return PM;
}

function model(width: number) {
  const object3d = new THREE.Mesh(new THREE.BoxGeometry(width, 1, 1), new THREE.MeshStandardMaterial());
  sourceMeshes.push(object3d); return { kind: 'model', object3d, vertices: 24 };
}

describe('composition runtime reference observation', () => {
  it('reuses warm scan storage without rebuilding maps or per-layer sets', () => {
    const PM = editor(20), runtime = compositionRuntime(PM), revision = runtime.revision;
    const original = runtime.objects.get('model0'), NativeMap = Map, NativeSet = Set;
    let maps = 0, sets = 0;
    vi.stubGlobal('Map', class extends NativeMap<any, any> { constructor(entries?: any) { super(entries); maps++; } });
    vi.stubGlobal('Set', class extends NativeSet<any> { constructor(entries?: any) { super(entries); sets++; } });
    for (let i = 0; i < 15; i++) expect(compositionRuntime(PM)).toBe(runtime);
    expect({ maps, sets }).toEqual({ maps: 0, sets: 0 });
    expect(runtime.revision).toBe(revision); expect(runtime.objects.get('model0')).toBe(original);
  });

  it('observes hydration, replacement, deletion and clearing of an ordinary asset map without touch', () => {
    const PM = editor(1, { assetId: 'model-asset' }), runtime = compositionRuntime(PM), editRevision = PM._scene3dRevision;
    expect(runtime.objects.get('model0')!.userData.missing).toBe(true);
    PM.assets.map.set('model-asset', model(1)); compositionRuntime(PM);
    const first = runtime.objects.get('model0') as THREE.Mesh, dispose = vi.spyOn(first.geometry, 'dispose');
    expect(first.userData.missing).not.toBe(true);
    PM.assets.map.set('model-asset', model(2)); compositionRuntime(PM);
    expect(runtime.objects.get('model0')).not.toBe(first); expect(dispose).toHaveBeenCalledOnce();
    PM.assets.map.delete('model-asset'); compositionRuntime(PM);
    expect(runtime.objects.get('model0')!.userData.missing).toBe(true);
    PM.assets.map = new Map([['model-asset', model(3)]]); compositionRuntime(PM);
    expect(runtime.objects.get('model0')!.userData.missing).not.toBe(true);
    PM.assets.map.clear(); compositionRuntime(PM);
    expect(runtime.objects.get('model0')!.userData.missing).toBe(true);
    expect(PM._scene3dRevision).toBe(editRevision);
  });

  it('observes an in-place source ID change without touch', () => {
    const PM = editor(1, { assetId: 'one' });
    PM.assets.map.set('one', model(1)); PM.assets.map.set('two', model(2));
    const runtime = compositionRuntime(PM), first = runtime.objects.get('model0'), revision = runtime.revision;
    PM.L('model0').d.data.object.source.assetId = 'two';
    compositionRuntime(PM);
    expect(runtime.revision).toBe(revision + 1); expect(runtime.objects.get('model0')).not.toBe(first);
  });

  it('observes replaced layers, source objects, material maps and asset getters without touch', () => {
    const PM = editor(1, { assetId: 'one' }, { color: 'red' });
    const red = { kind: 'image', el: { name: 'red' } }, blue = { kind: 'image', el: { name: 'blue' } };
    PM.assets.map = new Map<string, any>([['one', model(1)], ['two', model(2)], ['red', red], ['blue', blue]]);
    const runtime = compositionRuntime(PM), revision = PM._scene3dRevision;
    const first = runtime.objects.get('model0');
    const layer = PM.L('model0');
    PM.proj.layers = [{ ...layer, d: { ...layer.d, data: { object: {
      ...layer.d.data.object, source: { assetId: 'two' }, material: { ...layer.d.data.object.material, maps: { color: 'blue' } },
    } } } }];
    compositionRuntime(PM);
    expect(runtime.objects.get('model0')).not.toBe(first);
    const material = (runtime.objects.get('model0') as THREE.Mesh).material as THREE.MeshStandardMaterial;
    expect(material.map!.image).toBe(blue.el);
    const next = model(3), previous = runtime.objects.get('model0');
    PM.assets.get = (id: string) => id === 'two' ? next : PM.assets.map.get(id);
    compositionRuntime(PM); expect(runtime.objects.get('model0')).not.toBe(previous);
    expect(PM._scene3dRevision).toBe(revision);
  });

  it('keeps per-layer dedup, occurrence order and the last duplicate getter value', () => {
    const PM = editor(2, { primitive: 'box' }, { color: 'shared', normal: 'shared' });
    const old = { kind: 'image', el: {} }, changed = { kind: 'image', el: {} };
    PM.assets.map.set('shared', old);
    const runtime = compositionRuntime(PM), revision = runtime.revision;
    const get = vi.fn().mockReturnValueOnce(changed).mockReturnValue(old); PM.assets.get = get;
    compositionRuntime(PM);
    expect(get.mock.calls).toEqual([['shared'], ['shared']]); expect(runtime.revision).toBe(revision);
    PM.assets.get = vi.fn().mockReturnValueOnce(old).mockReturnValue(changed);
    compositionRuntime(PM); expect(runtime.revision).toBe(revision + 1);
    expect(((runtime.objects.get('model0') as THREE.Mesh).material as THREE.MeshStandardMaterial).map!.image).toBe(changed.el);
  });

  it('collects all source IDs before an asset getter mutates a later layer', () => {
    const PM = editor(2, { primitive: 'box' }, { color: 'one' });
    PM.proj.layers.reverse();
    PM.L('model1').d.data.object.material.maps.color = 'two';
    for (const id of ['one', 'two', 'three']) PM.assets.map.set(id, { kind: 'image', el: { name: id } });
    const runtime = compositionRuntime(PM), revision = runtime.revision;
    const get = vi.fn((id: string) => {
      if (id === 'one') PM.L('model1').d.data.object.material.maps.color = 'three';
      return PM.assets.map.get(id);
    }); PM.assets.get = get;
    compositionRuntime(PM);
    expect(get.mock.calls).toEqual([['one'], ['two']]); expect(runtime.revision).toBe(revision);
    get.mockClear(); compositionRuntime(PM);
    expect(get.mock.calls.slice(0, 2)).toEqual([['one'], ['three']]); expect(runtime.revision).toBe(revision + 1);
  });

  it('preserves source/map property getter ordering and ignores inherited/nonenumerable map entries', () => {
    const PM = editor(1, { assetId: 'one' }, { color: 'red', normal: 'red' });
    PM.assets.map.set('one', model(1)); PM.assets.map.set('red', { kind: 'image', el: {} });
    const runtime = compositionRuntime(PM), revision = runtime.revision, calls: string[] = [];
    const node = PM.L('model0').d.data.object;
    Object.defineProperty(node.source, 'assetId', { enumerable: true, get: () => { calls.push('source'); return 'one'; } });
    const maps = Object.create({ emissive: 'ignored' });
    Object.defineProperties(maps, {
      color: { enumerable: true, get: () => { calls.push('color'); return 'red'; } },
      normal: { enumerable: true, get: () => { calls.push('normal'); return 'red'; } },
      ao: { enumerable: false, get: () => { throw new Error('Must not read hidden map'); } },
    }); node.material.maps = maps;
    PM.assets.get = (id: string) => { calls.push(`get:${id}`); return PM.assets.map.get(id); };
    compositionRuntime(PM);
    expect(calls).toEqual(['source', 'color', 'normal', 'get:one', 'get:red']); expect(runtime.revision).toBe(revision);
  });

  it('protects the outer snapshot from reentrant asset and source getters', () => {
    const PM = editor(2, { primitive: 'box' }, { color: 'shared' });
    PM.assets.map.set('shared', { kind: 'image', el: {} });
    const runtime = compositionRuntime(PM), revision = runtime.revision;
    let inside = false;
    const enter = () => { if (!inside) { inside = true; try { expect(compositionRuntime(PM)).toBe(runtime); } finally { inside = false; } } };
    PM.assets.get = (id: string) => { enter(); return PM.assets.map.get(id); };
    const node = PM.L('model0').d.data.object;
    Object.defineProperty(node.source, 'assetId', { enumerable: true, get: () => { enter(); return undefined; } });
    compositionRuntime(PM);
    expect(runtime.revision).toBe(revision); expect(runtime.objects.has('model0') && runtime.objects.has('model1')).toBe(true);
  });

  it('cleans a failed source/getter scan and retries against the committed references', () => {
    const PM = editor(1, { assetId: 'one' }); PM.assets.map.set('one', model(1));
    const runtime = compositionRuntime(PM), revision = runtime.revision, first = runtime.objects.get('model0');
    const layer = PM.L('model0'), data = layer.d.data;
    Object.defineProperty(layer.d, 'data', { configurable: true, get: () => { throw new Error('Source unavailable'); } });
    expect(() => compositionRuntime(PM)).toThrow('Source unavailable');
    Object.defineProperty(layer.d, 'data', { configurable: true, writable: true, value: data });
    PM.assets.get = () => { throw new Error('Asset unavailable'); };
    expect(() => compositionRuntime(PM)).toThrow('Asset unavailable');
    PM.assets.get = (id: string) => PM.assets.map.get(id); compositionRuntime(PM);
    expect(runtime.revision).toBe(revision); expect(runtime.objects.get('model0')).toBe(first);
    PM.assets.map.set('one', model(2)); compositionRuntime(PM);
    expect(runtime.objects.get('model0')).not.toBe(first);
  });

  it('keeps the last successful reference snapshot when synchronization rejects a replacement', () => {
    const PM = editor(1, { assetId: 'one' }); PM.assets.map.set('one', model(1));
    const runtime = compositionRuntime(PM), revision = runtime.revision, first = runtime.objects.get('model0');
    const oversized = { ...model(2), vertices: 30_000_001 }; PM.assets.map.set('one', oversized);
    expect(() => compositionRuntime(PM)).toThrow('30 million vertices');
    expect(runtime.revision).toBe(revision); expect(runtime.objects.get('model0')).toBe(first);
    oversized.vertices = 24; compositionRuntime(PM);
    expect(runtime.revision).toBe(revision + 1); expect(runtime.objects.get('model0')).not.toBe(first);
  });

  it('resynchronizes root and one-layer matte compositions that share the same runtime key', () => {
    const PM = editor(2), root = PM.proj, runtime = compositionRuntime(PM), revision = runtime.revision;
    const matte = { ...root, layers: [{ ...root.layers.find((layer: any) => layer.id === 'model0'), on: true }] };
    expect(compositionRuntime(PM, 0, matte)).toBe(runtime);
    expect(runtime.objects.has('model0')).toBe(true); expect(runtime.objects.has('model1')).toBe(false);
    expect(compositionRuntime(PM, 0, root)).toBe(runtime); expect(runtime.objects.has('model1')).toBe(true);
    expect(runtime.revision).toBe(revision + 2);
    PM._renderComposition3D = matte; expect(compositionRuntime(PM)).toBe(runtime); delete PM._renderComposition3D;
    compositionRuntime(PM); expect(runtime.objects.has('model1')).toBe(true);
  });

  it('keeps frame/edit/metadata invalidation, live camera dimensions and fresh runtimes after disposal', () => {
    const PM = editor(), runtime = compositionRuntime(PM), revision = runtime.revision;
    compositionRuntime(PM, 1); expect(runtime.revision).toBe(revision + 1);
    PM.L('model0').p['position.x'].v = 2; PM.touch(); compositionRuntime(PM, 1);
    expect(runtime.objects.get('model0')!.position.x).toBe(2); expect(runtime.revision).toBe(revision + 2);
    PM.proj.assets = { ...PM.proj.assets }; compositionRuntime(PM, 1); expect(runtime.revision).toBe(revision + 3);
    PM.proj.w = 640; compositionRuntime(PM, 1);
    expect((runtime.camera as THREE.PerspectiveCamera).aspect).toBe(640 / 180); expect(runtime.revision).toBe(revision + 3);
    disposeSceneRuntimes(PM); const next = compositionRuntime(PM, 1);
    expect(next).not.toBe(runtime); expect(next.objects.get('model0')!.position.x).toBe(2);
  });

  it('resolves a single runtime snapshot for native object bounds and retains camera selection', () => {
    const PM = editor(1, { assetId: 'one' }); PM.assets.map.set('one', model(1));
    const runtime = compositionRuntime(PM); installCompositor(PM);
    const get = vi.fn((id: string) => PM.assets.map.get(id)); PM.assets.get = get;
    expect(PM.GL.bounds(PM.L('model0'), 0)).not.toBeNull();
    expect(get.mock.calls).toEqual([['one']]);
    expect(runtime.objects.has('model0')).toBe(true);
  });

  it('preserves map numeric-key ordering and getter changes to later descriptors and keys', () => {
    for (const change of ['hide', 'show', 'delete', 'add'] as const) {
      const PM = editor(1, { primitive: 'box' }, { color: 'one', normal: 'two' });
      PM.assets.map.set('one', { kind: 'image', el: {} }); PM.assets.map.set('two', { kind: 'image', el: {} });
      const runtime = compositionRuntime(PM), maps: any = {};
      Object.defineProperty(maps, 'color', { enumerable: true, get: () => {
        if (change === 'hide') Object.defineProperty(maps, 'normal', { enumerable: false });
        if (change === 'show') Object.defineProperty(maps, 'normal', { enumerable: true });
        if (change === 'delete') delete maps.normal;
        if (change === 'add') maps.roughness = 'two';
        return 'one';
      } });
      Object.defineProperty(maps, 'normal', { configurable: true, enumerable: change !== 'show', value: 'two' });
      if (change === 'add') delete maps.normal;
      PM.L('model0').d.data.object.material.maps = maps;
      const get = vi.fn((id: string) => PM.assets.map.get(id)); PM.assets.get = get;
      const sync = runtime.sync.bind(runtime); let scanned: any[] | undefined;
      vi.spyOn(runtime, 'sync').mockImplementation((...args) => { scanned = get.mock.calls.map(call => [...call]); sync(...args); });
      compositionRuntime(PM);
      expect(scanned || get.mock.calls).toEqual(change === 'show' ? [['one'], ['two']] : [['one']]);
    }
    const PM = editor(1, { primitive: 'box' }, { color: 'one', normal: 'two' });
    for (const id of ['one', 'two']) PM.assets.map.set(id, { kind: 'image', el: {} });
    compositionRuntime(PM); const order: string[] = [], maps: any = {};
    Object.defineProperties(maps, {
      2: { enumerable: true, get: () => { order.push('2'); return 'two'; } },
      1: { enumerable: true, get: () => { order.push('1'); return 'one'; } },
      color: { enumerable: true, get: () => { order.push('color'); return 'one'; } },
    }); PM.L('model0').d.data.object.material.maps = maps;
    PM.assets.get = (id: string) => { order.push(`get:${id}`); return PM.assets.map.get(id); };
    compositionRuntime(PM); expect(order).toEqual(['1', '2', 'color', 'get:one', 'get:two']);
  });

  it('snapshots sparse layer length before an index getter appends another model', () => {
    const PM = editor(2, { primitive: 'box' }, { color: 'one' });
    PM.L('model1').d.data.object.material.maps.color = 'two';
    for (const id of ['one', 'two', 'three']) PM.assets.map.set(id, { kind: 'image', el: {} });
    const runtime = compositionRuntime(PM), revision = runtime.revision;
    const first = PM.L('model0'), second = PM.L('model1');
    const extra = JSON.parse(JSON.stringify(first)); extra.id = 'extra'; extra.d.data.object.material.maps.color = 'three';
    const layers = [first, , second]; let appended = false;
    Object.defineProperty(layers, 0, { enumerable: true, get: () => {
      if (!appended) { appended = true; layers.push(extra); } return first;
    } }); PM.proj.layers = layers;
    const get = vi.fn((id: string) => PM.assets.map.get(id)); PM.assets.get = get;
    compositionRuntime(PM); expect(get.mock.calls).toEqual([['one'], ['two']]); expect(runtime.revision).toBe(revision);
    PM.proj.layers = layers.filter(Boolean);
    get.mockClear(); compositionRuntime(PM);
    expect(get.mock.calls.slice(0, 3)).toEqual([['one'], ['two'], ['three']]); expect(runtime.objects.has('extra')).toBe(true);
  });
});
