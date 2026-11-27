import type { PowermoveAPI } from 'powermove';
import { OBJ_MODEL, OBJ_MODEL_ID } from './obj-model';
import { STUDIO_CUBE, STUDIO_CUBE_ID } from './studio-cube';

export default function activate(api: PowermoveAPI): void {
  api.layers.register(STUDIO_CUBE);
  api.layers.register(OBJ_MODEL);

  const addStudioCube = () => api.project.apply({
    type: 'add_layer',
    layerType: 'extension',
    name: '3D Studio Cube',
    content: { definition: STUDIO_CUBE_ID }
  }, { label: 'Add 3D Studio Cube' });

  api.commands.register({
    id: '3d.add-studio-cube',
    label: 'Add 3D Studio Cube',
    category: 'Layer',
    run: addStudioCube
  });

  const importObj = async (supplied?: unknown) => {
    try {
      const file = supplied instanceof File
        ? supplied
        : (await api.assets.pick({ accept: '.obj,model/obj,text/plain' }))[0];
      if (!file) return { ok: false, message: 'Import cancelled' };
      if (!/\.obj$/i.test(file.name)) throw new Error('Choose a Wavefront .obj file');
      api.ui.toast(`Importing ${file.name}…`);
      const asset = await api.assets.import(file, { layerDefinition: OBJ_MODEL_ID });
      const result = api.project.apply({
        type: 'add_layer',
        layerType: 'extension',
        name: file.name.replace(/\.obj$/i, '') || 'OBJ Model',
        content: {
          definition: OBJ_MODEL_ID,
          data: { assetId: asset.id, objects: [{ id: 'model', assetId: asset.id }] }
        }
      }, { label: `Import ${file.name}` });
      if (!result.ok) throw new Error(result.message || 'Could not create the 3D layer');
      api.ui.toast(`Imported ${file.name}`);
      return result;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      api.ui.toast(`Could not import OBJ · ${message}`, { sticky: true });
      return { ok: false, message };
    }
  };

  api.commands.register({
    id: '3d.import-obj',
    label: 'Import OBJ Model…',
    category: 'Layer',
    run: importObj
  });

  api.palette.registerProvider((query) => {
    const value = query.trim().toLowerCase();
    const entries = [
      { id: '3d.add-studio-cube', label: 'Add 3D Studio Cube', category: 'Layer', run: addStudioCube },
      { id: '3d.import-obj', label: 'Import OBJ Model…', category: 'Layer', run: importObj }
    ];
    if (!value) return entries;
    return entries.filter((entry) => entry.label.toLowerCase().includes(value) || (value.includes('3d') && entry.id.startsWith('3d.')));
  });
}
