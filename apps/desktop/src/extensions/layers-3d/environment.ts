import type { PowermoveAPI } from 'powermove';

export async function chooseEnvironment(api: PowermoveAPI, target?: string, supplied?: unknown): Promise<void> {
  const project = api.project.get();
  try {
    const [file] = supplied instanceof File ? [supplied] : await api.assets.pick({ accept: '.hdr,.exr' });
    if (!file) return;
    if (!/\.(hdr|exr)$/i.test(file.name)) throw new Error('Choose an HDR or EXR environment image');
    if (api.project.get() !== project) throw new Error('The project changed. Import the environment again.');
    const asset = await api.assets.import(file);
    if (api.project.get() !== project || target && !api.model.layer(target)) return;
    const result = api.scene3d.edit(target ? { operation: 'update_light', target, patch: { map: asset.id } } : {operation:'add_light',name:'Environment Light',light:{type:'environment',map:asset.id}}, { label: 'Set environment light', origin: 'inspector' });
    if (!result.ok) throw new Error(result.message);
    const id = (result.data.result as any)?.id;
    if (id || target) api.selection.select([id || target!]);
    api.transport.invalidate();
  } catch (error) { api.ui.toast(error instanceof Error ? error.message : String(error), { error: true }); }
}
