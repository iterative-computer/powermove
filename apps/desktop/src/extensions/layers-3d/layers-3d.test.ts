import { describe, expect, it, vi } from 'vitest';

import activate from './index';
import { OBJ_MODEL_ID } from './obj-model';
import { STUDIO_CUBE_ID } from './studio-cube';

describe('3D layer extension', () => {
  it('registers renderer capability while adding only structured instance data to the project', () => {
    const apply = vi.fn((..._args: any[]) => ({ ok: true }));
    const layerDefinitions: any[] = [];
    const commands: any[] = [];
    activate({
      layers: { register: (definition: any) => { layerDefinitions.push(definition); return { dispose() {} }; } },
      commands: { register: (command: any) => { commands.push(command); return { dispose() {} }; } },
      palette: { registerProvider: () => ({ dispose() {} }) },
      project: { apply },
    } as any);

    expect(layerDefinitions[0]).toMatchObject({ id: STUDIO_CUBE_ID, version: 1, renderer: { kind: 'fragment' } });
    expect(layerDefinitions[0].defaults.objects[0]).toMatchObject({ id: 'cube', geometry: 'rounded-box' });
    commands.find((command) => command.id === '3d.add-studio-cube').run();
    expect(apply).toHaveBeenCalledWith({
      type: 'add_layer', layerType: 'extension', name: '3D Studio Cube',
      content: { definition: STUDIO_CUBE_ID }
    }, { label: 'Add 3D Studio Cube' });
    expect(JSON.stringify(apply.mock.calls[0]?.[0])).not.toContain('void main');
  });

  it('imports OBJ bytes as a durable model asset before creating the structured layer', async () => {
    const apply = vi.fn(() => ({ ok: true, message: 'ok' }));
    const importAsset = vi.fn(async () => ({ id: 'asset-obj', name: 'chair.obj', kind: 'model' }));
    const commands: any[] = [];
    const definitions: any[] = [];
    activate({
      layers: { register: (definition: any) => { definitions.push(definition); return { dispose() {} }; } },
      commands: { register: (command: any) => { commands.push(command); return { dispose() {} }; } },
      palette: { registerProvider: () => ({ dispose() {} }) },
      project: { apply },
      assets: { pick: async () => [], import: importAsset },
      ui: { toast: vi.fn() },
    } as any);

    const file = new File(['v 0 0 0'], 'chair.obj', { type: 'model/obj' });
    const result = await commands.find((command) => command.id === '3d.import-obj').run(file);
    expect(definitions).toContainEqual(expect.objectContaining({ id: OBJ_MODEL_ID, renderer: { kind: 'mesh', assetField: 'assetId' } }));
    expect(importAsset).toHaveBeenCalledWith(file, { layerDefinition: OBJ_MODEL_ID });
    expect(apply).toHaveBeenCalledWith(expect.objectContaining({
      type: 'add_layer', layerType: 'extension', name: 'chair',
      content: { definition: OBJ_MODEL_ID, data: { assetId: 'asset-obj', objects: [{ id: 'model', assetId: 'asset-obj' }] } }
    }), { label: 'Import chair.obj' });
    expect(result.ok).toBe(true);
  });
});
