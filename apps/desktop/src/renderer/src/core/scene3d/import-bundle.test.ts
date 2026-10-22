// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { prepareModelImport } from './import-bundle';

const converters = vi.hoisted(() => ({ obj: 0, material: 0, export: 0 }));
vi.mock('three/addons/loaders/OBJLoader.js', async () => {
  converters.obj++; return vi.importActual('three/addons/loaders/OBJLoader.js');
});
vi.mock('three/addons/loaders/MTLLoader.js', async () => {
  converters.material++; return vi.importActual('three/addons/loaders/MTLLoader.js');
});
vi.mock('three/addons/exporters/GLTFExporter.js', async () => {
  converters.export++; return vi.importActual('three/addons/exporters/GLTFExporter.js');
});
const source = 'v -1 -1 0\nv 1 -1 0\nv 0 1 0\nusemtl Paint\nf 1 2 3\n';

describe('on-demand OBJ material conversion', () => {
  it('passes single files through and rejects invalid bundles without loading converters', async () => {
    expect(converters).toEqual({ obj: 0, material: 0, export: 0 });
    for (const name of ['model.glb', 'model.gltf', 'model.obj']) {
      const model = new File([source], name);
      expect(await prepareModelImport([model])).toBe(model);
    }
    await expect(prepareModelImport([])).rejects.toThrow('Choose a model');
    await expect(prepareModelImport([new File([source], 'a.obj'), new File([source], 'b.obj')])).rejects.toThrow('one model');
    await expect(prepareModelImport([new File([source], 'a.obj'), new File([''], 'paint.png')])).rejects.toThrow('material (.mtl)');
    await expect(prepareModelImport([new File([source], 'a.obj'), new File([''], 'paint.mtl'), new File([''], 'Paint.MTL')])).rejects.toThrow('Duplicate file name');
    expect(converters).toEqual({ obj: 0, material: 0, export: 0 });
  });

  it('loads the converters once and packages selected material bundles as GLB', async () => {
    const files = [new File([source], 'model.obj'), new File(['newmtl Paint\nKd 1 .25 .1\n'], 'model.mtl')];
    const [first, second] = await Promise.all([prepareModelImport(files), prepareModelImport(files)]);
    expect(converters).toEqual({ obj: 1, material: 1, export: 1 });
    for (const file of [first, second]) {
      expect(file.name).toBe('model.glb'); expect(file.type).toBe('model/gltf-binary');
      const view = new DataView(await file.arrayBuffer());
      expect(view.getUint32(0, true)).toBe(0x46546c67); expect(view.getUint32(4, true)).toBe(2);
      expect(view.getUint32(8, true)).toBe(file.size);
      const json = JSON.parse(new TextDecoder().decode(new Uint8Array(view.buffer, 20, view.getUint32(12, true))));
      expect(json.meshes).toHaveLength(1); expect(json.materials).toHaveLength(1);
      expect(json.materials[0].pbrMetallicRoughness.baseColorFactor).toHaveLength(4);
    }
  });
});
