/*
 * The engine's 3D geometry questions (picks, bounds, model-group bounds)
 * answered by pm-wasm (engine/crates/pm-eval/src/query3d.rs) instead of the
 * composition's three.js runtime:
 *
 * - `geometryRequest(PM)`: what a question carries besides the document,
 *   the editor camera exactly as the renderer holds it (`viewportCamera` in
 *   the editor view) and the Local View set.
 * - `resolveModels(PM, wasm, run, reply)`: a reply of `{models: [ids]}`
 *   means the mirror met model assets it does not have. They are added from
 *   the renderer's asset store as `SceneRuntime.makeObject` reads it (a glTF
 *   model's source bytes, an OBJ model's text, else the missing-model box)
 *   and the question is asked again.
 */
import { editorCamera, localViewIds } from './viewport';
import { modelSourceBytes } from './import-model';
import { layer3DRole } from './layers';
import type { PmWasm } from '../../engine-doc/wasm';

const encoder = new TextEncoder();

/** Model assets added to each module instance, with the record they came from. */
const fed = new WeakMap<object, Map<string, unknown>>();

const asArray = (m: { elements: ArrayLike<number> }) => Array.from(m.elements);

/** The editor camera and Local View of the open composition, when it has 3D layers. */
export function geometryRequest(PM: any): { camera?: object; localView?: string[] | null } {
  const layers: any[] = PM?.proj?.layers || [];
  if (!layers.some(layer => layer3DRole(layer))) return {};
  const local = localViewIds(PM);
  const camera: any = editorCamera(PM);
  if (!camera) return { localView: local ? [...local] : null };
  return {
    localView: local ? [...local] : null,
    camera: {
      world: asArray(camera.matrixWorld),
      view: asArray(camera.matrixWorldInverse),
      projection: asArray(camera.projectionMatrix),
      projectionInverse: asArray(camera.projectionMatrixInverse),
      orthographic: !!camera.isOrthographicCamera,
      fov: camera.fov,
      zoom: camera.zoom,
      near: camera.near,
      far: camera.far
    }
  };
}

/** Adds model assets `ids` to the module from the renderer's asset store. */
export function feedModels(PM: any, wasm: PmWasm, ids: readonly string[]): void {
  let map = fed.get(wasm);
  if (!map) { map = new Map(); fed.set(wasm, map); }
  for (const id of ids) {
    const asset = PM?.assets?.get?.(id);
    let kind: 'missing' | 'obj' | 'gltf' = 'missing';
    let data: Uint8Array | undefined;
    if (asset?.object3d) {
      const bytes = modelSourceBytes(asset.object3d);
      if (bytes) { kind = 'gltf'; data = new Uint8Array(bytes); }
      else console.warn(`[engine-doc] 3D model ${id} has no source bytes; the engine draws it as missing`);
    } else if (asset?.mesh && typeof asset.sourceText === 'string') {
      kind = 'obj';
      data = encoder.encode(asset.sourceText);
    }
    const error = wasm.setModel(id, kind, data);
    if (error) console.warn('[engine-doc]', error);
    map.set(id, asset ?? null);
  }
}

/** Forgets models whose asset record changed (relinked, reloaded, removed). */
export function refreshModels(PM: any, wasm: PmWasm): void {
  const map = fed.get(wasm);
  if (!map) return;
  for (const [id, record] of map) {
    if ((PM?.assets?.get?.(id) ?? null) === record) continue;
    wasm.forgetModel(id);
    map.delete(id);
  }
}

/** Answers `{models}` replies by adding the models and asking again. */
export function resolveModels<R>(PM: any, wasm: PmWasm, run: () => R, reply: R): R {
  let r: any = reply;
  // Each round adds every model asked for; nested questions may ask once more.
  for (let round = 0; round < 4 && Array.isArray(r?.models); round++) {
    feedModels(PM, wasm, r.models);
    r = run();
  }
  return r;
}
