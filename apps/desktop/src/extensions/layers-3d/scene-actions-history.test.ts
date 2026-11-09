import { expect, it, vi } from 'vitest';
import * as THREE from 'three';
import { makePM } from '../../renderer/src/legacy/__tests__/make-pm';
import { editScene } from '../../renderer/src/core/scene3d/operations';
import { LAYER3D_DEFINITIONS } from '../../renderer/src/core/scene3d/layers';
import { importModelIntoScene } from './scene-actions';

// Geometry decoding/rendering is covered by the desktop import test. Exercise
// the real editing, selection and history boundary without a GPU here.
vi.mock('../../renderer/src/core/scene3d/service', async importOriginal => ({
  ...await importOriginal<typeof import('../../renderer/src/core/scene3d/service')>(),
  compositionRuntime: () => ({ worldBounds: () => new THREE.Box3(new THREE.Vector3(-1, -1, -1), new THREE.Vector3(1, 1, 1)) })
}));

it('undoes the first imported model and its studio in one step after selecting the model', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: 'Import history', dur: 5 });
  PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
  const asset = { id: 'model-asset', kind: 'model', format: 'obj' };
  const api: any = {
    scene3d: { prepareImport: async (files: File[]) => files[0], edit: (args: any, meta: any) => editScene(PM, args, meta) },
    assets: { import: async () => { PM.proj.assets[asset.id] = asset; return asset; } },
    model: { curComp: () => PM.proj },
    selection: { select: (ids: string[]) => PM.selectLayers(ids) },
    edit: PM.Edit,
    transport: { invalidate: () => {} },
    ui: { toast: vi.fn() }
  };
  const result = await importModelIntoScene(api, new File(['v 0 0 0'], 'cube.obj'));
  expect(result.ok).toBe(true);
  if (!result.ok) throw new Error(result.message);
  const modelId = (result.data.result as any).id;
  expect(PM.sel.layers).toEqual([modelId]);
  expect(PM.proj.layers).toHaveLength(5);
  expect(PM.hist.label()).toBe('Import cube.obj');
  const imported = JSON.stringify(PM.proj.layers);
  expect(PM.hist.undo()).toBe(true);
  expect(PM.proj.layers).toEqual([]);
  expect(PM.proj.assets[asset.id]).toEqual(asset);
  expect(PM.hist.canUndo()).toBe(false);
  expect(PM.hist.redo()).toBe(true);
  expect(JSON.stringify(PM.proj.layers)).toBe(imported);
});
