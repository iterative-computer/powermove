import * as THREE from 'three';
import { engineGeometry } from './engine-access';
import { layer3DRole } from './layers';
import { toScreen, type Rect } from './modal';
import { world3D } from '../../legacy/core/space-3d';

/** Hit geometry is synchronous even while the native chrome planes are in
 * flight. Flag-on answers come from the same strict wasm evaluator as the
 * other viewport hit tests, without constructing a reference scene. */
export function viewportGlyphHits(
  PM: any,
  camera: THREE.Camera,
  rect: Rect,
  hidden: string | null,
  local: ReadonlySet<string> | null
): Array<{ id: string; x: number; y: number }> {
  const engine = engineGeometry(PM, ['world3D', 'active']);
  const hits: Array<{ id: string; x: number; y: number }> = [];
  for (const layer of (PM.curComp?.() || PM.proj).layers) {
    const role = layer3DRole(layer);
    if ((role !== 'light' && role !== 'camera') || layer.id === hidden || (local && !local.has(layer.id))) continue;
    if (!(engine ? engine.active(layer, PM.time) : PM.active(layer, PM.time))) continue;
    const values = engine ? engine.world3D(layer, PM.time) : world3D(PM, layer, PM.time);
    const position = new THREE.Vector3().setFromMatrixPosition(new THREE.Matrix4().fromArray(values));
    const screen = toScreen(camera, rect, position);
    if (screen.z < 1) hits.push({ id: layer.id, x: screen.x, y: screen.y });
  }
  return hits;
}
