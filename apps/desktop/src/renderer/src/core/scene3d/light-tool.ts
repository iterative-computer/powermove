import * as THREE from 'three';
import { highlightPlacement, surfaceAt } from '../studio/operations';
import { compositionRuntime } from './service';
import { editScene } from './operations';

/*
 * The light tool: put light where you point, after Light Wrangler for Blender.
 *
 * - Reflect: the light moves so its reflection (the highlight) lands on the
 *   surface under the cursor, as seen from the camera.
 * - Direct: the light faces the surface under the cursor straight on, along
 *   its normal (where the light falls, not where it shines back).
 * - Orbit: dragging circles the light around its target at the same distance.
 *
 * Pointing aims the selected light; with none selected, or after Add, it adds
 * one there. While pointing, the light moves live; releasing makes the whole
 * move one undoable edit. Brightness, size, distance (brightness follows, so
 * the product stays as bright) and a spotlight's spread are set by value from
 * the viewer's light panel, live while dragging and one edit on release.
 */

export type LightAim = 'reflect' | 'direct' | 'orbit';
export type LightKind = 'area' | 'spot' | 'point' | 'sun';
export type LightAdjust = 'power' | 'size' | 'distance' | 'spread';

export type LightInfo = {
  id: string; name: string; type: LightKind;
  power: number; size: number | null; distance: number; spread: number | null;
};

const NAMES: Record<LightKind, string> = { area: 'Softbox', spot: 'Spotlight', point: 'Bulb', sun: 'Sun' };
const POWER: Record<LightKind, number> = { area: 6, spot: 40, point: 25, sun: 3 };

type Snapshot = { x: number; y: number; z: number; targetX: number; targetY: number; targetZ: number; intensity: number; width: number; height: number; angle: number };

const lightOf = (PM: any, id: string | null | undefined) => {
  const layer = id ? PM.L?.(id) : null;
  return layer?.d?.data?.light ? layer : null;
};
const num = (v: any, fallback = 0) => (Number.isFinite(Number(v)) ? Number(v) : fallback);

function snapshot(layer: any): Snapshot {
  const p = layer.d.data.light.p;
  return {
    x: num(layer.p['position.x']?.v), y: num(layer.p['position.y']?.v), z: num(layer.p['position.z']?.v),
    targetX: num(p.targetX?.v), targetY: num(p.targetY?.v), targetZ: num(p.targetZ?.v),
    intensity: num(p.intensity?.v, 3), width: num(p.width?.v, 2), height: num(p.height?.v, 1.2), angle: num(p.angle?.v, 45),
  };
}

/** Write a light's values in place (a live preview, not an edit). */
function apply(PM: any, layer: any, s: Snapshot): void {
  const p = layer.d.data.light.p;
  layer.p['position.x'].v = s.x; layer.p['position.y'].v = s.y; layer.p['position.z'].v = s.z;
  p.targetX.v = s.targetX; p.targetY.v = s.targetY; p.targetZ.v = s.targetZ;
  p.intensity.v = s.intensity; p.width.v = s.width; p.height.v = s.height; p.angle.v = s.angle;
  PM.touch?.(); PM.invalidate?.();
}

const position = (s: Snapshot) => new THREE.Vector3(s.x, s.y, s.z);
const target = (s: Snapshot) => new THREE.Vector3(s.targetX, s.targetY, s.targetZ);

export function lightInfo(PM: any, id: string | null | undefined): LightInfo | null {
  const layer = lightOf(PM, id);
  if (!layer) return null;
  const s = snapshot(layer), type = layer.d.data.light.type as LightKind;
  return {
    id: layer.id, name: layer.name || NAMES[type], type, power: s.intensity,
    size: type === 'area' ? Math.max(s.width, s.height) : null,
    distance: position(s).distanceTo(target(s)),
    spread: type === 'spot' ? s.angle : null,
  };
}

export function createLightTool(PM: any) {
  let gesture: { id: string; start: Snapshot; mode: LightAim; last: { u: number; v: number } } | null = null;
  // A value being dragged in the light panel, and the light as it was before.
  let tuning: { id: string; start: Snapshot } | null = null;

  const eye = () => {
    const runtime = compositionRuntime(PM, PM.time), camera = PM._renderCamera3D || runtime.camera;
    camera.updateMatrixWorld(true);
    return { runtime, camera, eye: new THREE.Vector3().setFromMatrixPosition(camera.matrixWorld) };
  };
  const hit = (u: number, v: number) => {
    const { runtime, camera, eye: e } = eye();
    const h = surfaceAt(runtime.scene, camera, u, v);
    return h ? { point: new THREE.Vector3(...h.point), normal: new THREE.Vector3(...h.normal), eye: e } : null;
  };

  /** Where a light at `distance` goes to aim at a surface point in this mode. */
  const placement = (mode: LightAim, point: THREE.Vector3, normal: THREE.Vector3, e: THREE.Vector3, distance: number) =>
    mode === 'direct' ? point.clone().add(normal.clone().normalize().multiplyScalar(distance)) : highlightPlacement(point, normal, e, distance);

  const commit = (id: string, start: Snapshot, label: string) => {
    const layer = lightOf(PM, id);
    if (!layer) return;
    const end = snapshot(layer);
    if (JSON.stringify(end) === JSON.stringify(start)) return;
    // Restore, then apply the whole change as one edit.
    apply(PM, layer, start);
    const result = editScene(PM, { operation: 'update_light', target: id, patch: { p: end } }, { origin: 'canvas', label });
    if (!result.ok) PM.toast?.(result.message, { error: true });
  };
  const commitTuning = () => {
    if (!tuning) return;
    const { id, start } = tuning;
    tuning = null;
    commit(id, start, 'Adjust light');
  };

  return {
    /** Is there a surface under this frame point to aim at? */
    surface: (u: number, v: number) => !!hit(u, v),

    /**
     * Start pointing. Aims `lightId` (or adds a light of `kind` when it is not a
     * light) at the surface under (u, v). Orbit needs a light and no surface.
     * Returns the light being aimed, or null when there is nothing to aim at.
     */
    begin(lightId: string | null, u: number, v: number, mode: LightAim, kind: LightKind = 'area', add = false): string | null {
      commitTuning();
      let layer = add ? null : lightOf(PM, lightId);
      if (mode === 'orbit') {
        if (!layer) return null;
        gesture = { id: layer.id, start: snapshot(layer), mode, last: { u, v } };
        return layer.id;
      }
      const h = hit(u, v);
      if (!h) return null;
      if (!layer) {
        // A new light, sized to the product and placed at the camera's distance.
        const distance = Math.max(1.5, h.eye.distanceTo(h.point) * 0.9);
        const at = placement(mode, h.point, h.normal, h.eye, distance);
        const size = distance * 0.6;
        const result: any = editScene(PM, {
          operation: 'add_light', name: NAMES[kind],
          light: { type: kind, name: NAMES[kind], p: {
            x: at.x, y: at.y, z: at.z, targetX: h.point.x, targetY: h.point.y, targetZ: h.point.z,
            intensity: POWER[kind], ...(kind === 'area' ? { width: size, height: size * 0.62 } : {}), ...(kind === 'spot' ? { angle: 30 } : {}),
          } },
        }, { origin: 'canvas', label: `Add ${NAMES[kind].toLowerCase()}` });
        if (!result.ok) { PM.toast?.(result.message, { error: true }); return null; }
        const id = result.data?.result?.id;
        layer = lightOf(PM, id);
        if (!layer) return null;
        PM.selectLayers?.([layer.id]);
      }
      gesture = { id: layer.id, start: snapshot(layer), mode, last: { u, v } };
      this.move(u, v);
      return layer.id;
    },

    /** Follow the pointer: the light moves live. */
    move(u: number, v: number): void {
      if (!gesture) return;
      const layer = lightOf(PM, gesture.id);
      if (!layer) { gesture = null; return; }
      const s = snapshot(layer);
      const distance = Math.max(0.05, position(s).distanceTo(target(s)));
      if (gesture.mode === 'orbit') {
        // Around the target: across turns about the world's up, up and down tilts.
        const du = u - gesture.last.u, dv = v - gesture.last.v;
        gesture.last = { u, v };
        const offset = position(s).sub(target(s));
        const sph = new THREE.Spherical().setFromVector3(offset);
        sph.theta -= du * Math.PI * 2;
        sph.phi = THREE.MathUtils.clamp(sph.phi + dv * Math.PI, 0.05, Math.PI - 0.05);
        const next = target(s).add(new THREE.Vector3().setFromSpherical(sph));
        apply(PM, layer, { ...s, x: next.x, y: next.y, z: next.z });
        return;
      }
      const h = hit(u, v);
      if (!h) return;
      const at = placement(gesture.mode, h.point, h.normal, h.eye, distance);
      apply(PM, layer, { ...s, x: at.x, y: at.y, z: at.z, targetX: h.point.x, targetY: h.point.y, targetZ: h.point.z });
    },

    /** Release: the move becomes one undoable edit. */
    end(): void {
      if (!gesture) return;
      const { id, start, mode } = gesture;
      gesture = null;
      commit(id, start, mode === 'orbit' ? 'Orbit light' : mode === 'direct' ? 'Aim light' : 'Move highlight');
    },

    cancel(): void {
      if (gesture) { const layer = lightOf(PM, gesture.id); if (layer) apply(PM, layer, gesture.start); gesture = null; }
      commitTuning();
    },

    active: () => !!gesture,

    /**
     * Set one value of a light live (the panel's slider is moving); `commit`
     * ends the drag and makes it one edit. Distance moves the light along its
     * line to the target and scales power by distance², so the light falling on
     * the product stays as bright.
     */
    set(lightId: string, what: LightAdjust, value: number): LightInfo | null {
      const layer = lightOf(PM, lightId);
      if (!layer) return null;
      if (tuning && tuning.id !== lightId) commitTuning();
      tuning ??= { id: lightId, start: snapshot(layer) };
      const s = snapshot(layer), type = layer.d.data.light.type as LightKind;
      if (what === 'power') s.intensity = THREE.MathUtils.clamp(value, 0, 10000);
      if (what === 'size' && type === 'area') {
        const k = Math.max(0.001, value) / Math.max(s.width, s.height, 1e-6);
        s.width *= k; s.height *= k;
      }
      if (what === 'spread' && type === 'spot') s.angle = THREE.MathUtils.clamp(value, 1, 89);
      if (what === 'distance' && type !== 'sun') {
        const t = target(s), d0 = Math.max(1e-6, position(s).distanceTo(t)), d1 = Math.max(0.05, value);
        const p = t.clone().add(position(s).sub(t).setLength(d1));
        s.x = p.x; s.y = p.y; s.z = p.z;
        s.intensity = THREE.MathUtils.clamp(s.intensity * (d1 / d0) ** 2, 0, 10000);
      }
      apply(PM, layer, s);
      return lightInfo(PM, lightId);
    },

    commit: commitTuning,
  };
}

export type LightTool = ReturnType<typeof createLightTool>;

/** The light tool's settings in the viewer: whether it is on, how it aims, and what pointing adds. */
export type LightToolState = { on: boolean; aim: LightAim; kind: LightKind; adding: boolean };
const toolStates = new WeakMap<object, { state: LightToolState; listeners: Set<(s: LightToolState) => void>; tool: LightTool }>();

function entry(PM: any) {
  let e = toolStates.get(PM);
  if (!e) { e = { state: { on: false, aim: 'reflect', kind: 'area', adding: false }, listeners: new Set(), tool: createLightTool(PM) }; toolStates.set(PM, e); }
  return e;
}

export const getLightTool = (PM: any): LightToolState => ({ ...entry(PM).state });
export function setLightTool(PM: any, patch: Partial<LightToolState>): void {
  const e = entry(PM), next = { ...e.state, ...patch };
  if (JSON.stringify(next) === JSON.stringify(e.state)) return;
  if (!next.on) { e.tool.cancel(); next.adding = false; }
  e.state = next;
  for (const listener of e.listeners) listener({ ...next });
  PM.invalidate?.();
}
export function onLightTool(PM: any, listener: (s: LightToolState) => void): () => void {
  const e = entry(PM);
  e.listeners.add(listener);
  return () => e.listeners.delete(listener);
}
export const lightToolOf = (PM: any): LightTool => entry(PM).tool;
