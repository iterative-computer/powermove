import { object3dFromEngine } from './compat-object3d';
import { registerModelSource } from './import-model';

/** Import for the new-engine lane. The compatibility Object3D is assembled
 * from pm-geom3d's scene graph; GLTFLoader never parses these bytes. */
export async function loadGltfFromEngine(blob: Blob, engine: { importModel(bytes: Uint8Array): any }) {
  if (blob.size > 128 * 1024 * 1024) throw new Error('GLB models must be smaller than 128 MB');
  const source = await blob.arrayBuffer();
  const data = engine.importModel(new Uint8Array(source));
  const result = await object3dFromEngine(data, new Uint8Array(source));
  registerModelSource(result.object3d, source);
  return result;
}
