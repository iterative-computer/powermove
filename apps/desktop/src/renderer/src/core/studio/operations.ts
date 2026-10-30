import * as THREE from 'three';
import { layer3DRole } from '../scene3d/layers';
import { compositionRuntime } from '../scene3d/service';

/*
 * Studio's product-film vocabulary, built from ordinary guarded edits so every
 * result is undoable, keyframed and visible in the timeline:
 *
 * - apply_material: a named finish with a few plain sliders.
 * - light_highlight: light by pointing — place a softbox so its reflection
 *   lands on a chosen surface point as seen from the composition camera.
 * - shot: camera and product moves written as keyframes.
 * - set_studio: the seamless set (backdrop color plus a shadow-catching floor).
 */

export type MaterialPreset = { id: string; label: string; p: Record<string, number | string> };

/** Finishes a product film reaches for. Each is a starting point; sliders refine it. */
export const MATERIAL_PRESETS: MaterialPreset[] = [
  { id: 'anodized', label: 'Anodized aluminium', p: { color: '#5C5F66', metalness: 1, roughness: 0.34, clearcoat: 0, transmission: 0, specular: 1 } },
  { id: 'brushed', label: 'Brushed metal', p: { color: '#B9BCC2', metalness: 1, roughness: 0.42, clearcoat: 0, transmission: 0, specular: 1 } },
  { id: 'polished', label: 'Polished steel', p: { color: '#D7D9DD', metalness: 1, roughness: 0.06, clearcoat: 0, transmission: 0, specular: 1 } },
  { id: 'gold', label: 'Gold', p: { color: '#E8C26A', metalness: 1, roughness: 0.14, clearcoat: 0, transmission: 0, specular: 1 } },
  { id: 'glass', label: 'Clear glass', p: { color: '#FFFFFF', metalness: 0, roughness: 0.02, clearcoat: 0, transmission: 1, ior: 1.5, specular: 1 } },
  { id: 'frosted', label: 'Frosted glass', p: { color: '#F4F6F8', metalness: 0, roughness: 0.35, clearcoat: 0, transmission: 1, ior: 1.5, specular: 1 } },
  { id: 'lacquer', label: 'Gloss lacquer', p: { color: '#B3122E', metalness: 0, roughness: 0.45, clearcoat: 1, clearcoatRoughness: 0.02, transmission: 0, specular: 1 } },
  { id: 'soft-touch', label: 'Soft-touch plastic', p: { color: '#E8E4DC', metalness: 0, roughness: 0.62, clearcoat: 0, transmission: 0, specular: 0.6 } },
  { id: 'ceramic', label: 'Glazed ceramic', p: { color: '#F2F0EC', metalness: 0, roughness: 0.3, clearcoat: 0.8, clearcoatRoughness: 0.04, transmission: 0, specular: 1 } },
  { id: 'rubber', label: 'Matte rubber', p: { color: '#26272A', metalness: 0, roughness: 0.85, clearcoat: 0, transmission: 0, specular: 0.4 } },
  { id: 'screen', label: 'Display under glass', p: { color: '#050507', metalness: 0, roughness: 0.6, clearcoat: 1, clearcoatRoughness: 0.012, transmission: 0, specular: 0, emissiveIntensity: 1 } },
];

export type ShotType = 'turntable' | 'push_in' | 'orbit' | 'hero_tilt' | 'rack_focus';
export const SHOT_TYPES: { id: ShotType; label: string }[] = [
  { id: 'turntable', label: 'Turntable' }, { id: 'push_in', label: 'Push-in' }, { id: 'orbit', label: 'Orbit reveal' },
  { id: 'hero_tilt', label: 'Hero tilt' }, { id: 'rack_focus', label: 'Rack focus' },
];

export type StudioEdit = {
  operation: 'apply_material' | 'light_highlight' | 'shot' | 'set_studio' | 'explode' | 'drop_to_floor' | 'place_on' | 'auto_studio' | 'soften';
  /** place_on: the object to rest on */ on_target?: string;
  target?: string;
  /** apply_material */ preset?: string; p?: Record<string, unknown>;
  /** light_highlight: a world point on the product and its surface normal */ at?: [number, number, number]; normal?: [number, number, number];
  size?: number; intensity?: number; distance?: number; name?: string; color?: string;
  /** shot */ shot?: ShotType; start?: number; duration?: number; degrees?: number; amount?: number; from?: number; to?: number;
  /** set_studio */ on?: boolean;
};

type Edit3D = (args: any, meta?: any) => { commands: any[]; id?: string };

const comp = (PM: any) => PM.curComp?.() || PM.proj;
const camLayer = (PM: any) => comp(PM).layers.find((l: any) => layer3DRole(l) === 'camera' && l.on !== false);
const vec = (layer: any, prefix = 'position') => new THREE.Vector3(...['x', 'y', 'z'].map(a => Number(layer?.p?.[`${prefix}.${a}`]?.v ?? 0)) as [number, number, number]);
const aimOf = (layer: any) => {
  const p = layer?.d?.data?.camera?.p || layer?.d?.data?.light?.p || {};
  return new THREE.Vector3(Number(p.targetX?.v ?? 0), Number(p.targetY?.v ?? 0), Number(p.targetZ?.v ?? 0));
};

/** Where a softbox must sit so the eye sees its reflection at `point`. */
export function highlightPlacement(point: THREE.Vector3, normal: THREE.Vector3, eye: THREE.Vector3, distance: number): THREE.Vector3 {
  const n = normal.clone().normalize();
  const v = eye.clone().sub(point).normalize();
  const r = n.clone().multiplyScalar(2 * v.dot(n)).sub(v);
  return point.clone().add(r.multiplyScalar(distance));
}

/**
 * The product surface under a point of the frame (u, v in 0..1, y down): its
 * world position and normal, through the given camera.
 */
export function surfaceAt(scene: THREE.Scene, camera: THREE.Camera, u: number, v: number): { point: [number, number, number]; normal: [number, number, number]; id: string } | null {
  scene.updateMatrixWorld(true); camera.updateMatrixWorld(true);
  const ray = new THREE.Raycaster();
  ray.setFromCamera(new THREE.Vector2(u * 2 - 1, 1 - v * 2), camera);
  for (const hit of ray.intersectObjects(scene.children, true)) {
    const object: any = hit.object;
    const id = object.userData?.sceneId;
    if (!object.isMesh || !id || !hit.face) continue;
    let visible = true;
    for (let o: any = object; o; o = o.parent) if (!o.visible) visible = false;
    if (!visible) continue;
    const normal = hit.face.normal.clone().applyNormalMatrix(new THREE.Matrix3().getNormalMatrix(object.matrixWorld)).normalize();
    if (normal.dot(ray.ray.direction) > 0) normal.negate();
    return { point: [hit.point.x, hit.point.y, hit.point.z], normal: [normal.x, normal.y, normal.z], id };
  }
  return null;
}

function keys(target: string, path: string, frames: [number, number][]): any {
  return { type: 'replace_keyframes', target, path, keyframes: frames.map(([time, value]) => ({ time, value, ease: 'ease-in-out', hold: false })), expression: null };
}

export function studioCommands(PM: any, args: StudioEdit, edit3d: Edit3D): { commands: any[]; id?: string } {
  switch (args.operation) {
    case 'apply_material': {
      const preset = MATERIAL_PRESETS.find(m => m.id === args.preset);
      if (!preset && !args.p) throw new Error(`Unknown material preset: ${args.preset}. Presets: ${MATERIAL_PRESETS.map(m => m.id).join(', ')}`);
      return edit3d({ operation: 'update_object', target: args.target, patch: { material: { p: { ...(preset?.p || {}), ...(args.p || {}) } } } });
    }
    case 'light_highlight': {
      if (!args.at || !args.normal) throw new Error('light_highlight needs at [x,y,z] and normal [x,y,z] on the product');
      const camera = camLayer(PM);
      const eye = camera ? vec(camera) : new THREE.Vector3(3, 2, 6);
      const point = new THREE.Vector3(...args.at), normal = new THREE.Vector3(...args.normal);
      const distance = args.distance ?? Math.max(2, eye.distanceTo(point) * 0.9);
      const at = highlightPlacement(point, normal, eye, distance);
      const size = args.size ?? distance * 0.6;
      return edit3d({
        operation: 'add_light', name: args.name || 'Softbox',
        light: { type: 'area', name: args.name || 'Softbox', p: {
          x: at.x, y: at.y, z: at.z, targetX: point.x, targetY: point.y, targetZ: point.z,
          width: size, height: size * 0.62, intensity: args.intensity ?? 6, color: args.color ?? '#FFFFFF',
        } },
      });
    }
    case 'explode': {
      // A model group's parts move out from its centre along their own offsets.
      const group = PM.L(args.target);
      const members = comp(PM).layers.filter((l: any) => layer3DRole(l) === 'object' && (PM.groupAncestors?.(l) || []).some((g: any) => g.id === group?.id));
      if (!members.length) throw new Error('Explode needs a group of models');
      const runtime = compositionRuntime(PM, PM.time);
      const centre = runtime.worldBounds(members.map((m: any) => m.id)).getCenter(new THREE.Vector3());
      const size = runtime.worldBounds(members.map((m: any) => m.id)).getSize(new THREE.Vector3()).length() || 1;
      const amount = (args.amount ?? 0.5) * size;
      const start = args.start ?? PM.time ?? 0, end = start + Math.max(0.1, args.duration ?? 2);
      const commands: any[] = [];
      for (const m of members) {
        const own = runtime.worldBounds([m.id]).getCenter(new THREE.Vector3());
        const dir = own.clone().sub(centre);
        if (dir.lengthSq() < 1e-12) dir.set(0, 1, 0);
        dir.normalize().multiplyScalar(amount);
        const from = vec(m);
        for (const a of ['x', 'y', 'z'] as const) commands.push(keys(m.id, `position.${a}`, [[start, from[a]], [end, from[a] + dir[a]]]));
      }
      return { commands, id: group.id };
    }
    case 'drop_to_floor': case 'place_on': {
      // Rest a model on the floor (y = 0) or on top of another model, keeping x/z.
      const layer = PM.L(args.target);
      if (!layer || layer3DRole(layer) !== 'object') throw new Error('Choose a model to place');
      const runtime = compositionRuntime(PM, PM.time);
      const box = runtime.worldBounds([layer.id]);
      if (box.isEmpty()) throw new Error('The model has no geometry yet');
      let top = 0;
      const p = vec(layer);
      if (args.operation === 'place_on') {
        const under = runtime.worldBounds([String(args.on_target)]);
        if (under.isEmpty()) throw new Error('Choose a model to place it on');
        top = under.max.y;
        const c = under.getCenter(new THREE.Vector3()), own = box.getCenter(new THREE.Vector3());
        p.x += c.x - own.x; p.z += c.z - own.z;
      }
      p.y += top - box.min.y;
      return edit3d({ operation: 'update_object', target: layer.id, patch: { p: { x: p.x, y: p.y, z: p.z } } });
    }
    case 'auto_studio': {
      // A finished-looking start: camera framing every model, key/fill/rim softboxes, the seamless set.
      const runtime = compositionRuntime(PM, PM.time);
      const models = comp(PM).layers.filter((l: any) => layer3DRole(l) === 'object').map((l: any) => l.id);
      const box = runtime.worldBounds(models);
      if (box.isEmpty()) throw new Error('Add a model first');
      const centre = box.getCenter(new THREE.Vector3()), radius = Math.max(box.getSize(new THREE.Vector3()).length() / 2, 1e-3);
      const fov = 30, distance = radius / Math.sin(THREE.MathUtils.degToRad(fov / 2)) * 1.15;
      const eye = centre.clone().add(new THREE.Vector3(0.35, 0.55, 1).normalize().multiplyScalar(distance));
      const commands: any[] = [];
      let id: string | undefined;
      const push = (r: { commands: any[]; id?: string }) => { commands.push(...r.commands); id ??= r.id; return r.id; };
      const camera = camLayer(PM);
      if (camera) push(edit3d({ operation: 'set_camera', target: camera.id, patch: { p: { x: eye.x, y: eye.y, z: eye.z, targetX: centre.x, targetY: centre.y, targetZ: centre.z, fov } } }));
      else push(edit3d({ operation: 'add_camera', camera: { camera: { p: { x: eye.x, y: eye.y, z: eye.z, targetX: centre.x, targetY: centre.y, targetZ: centre.z, fov } }, environment: { shadowCatcher: true } } }));
      const rig: [string, number, number, number, number][] = [['Key softbox', -40, 50, 6, 1.4], ['Fill softbox', 60, 20, 2.2, 1.8], ['Rim strip', 150, 25, 7, 0.9]];
      for (const [name, az, el, intensity, scale] of rig) {
        const a = THREE.MathUtils.degToRad(az), e = THREE.MathUtils.degToRad(el), d = distance * 0.9;
        const at = centre.clone().add(new THREE.Vector3(Math.sin(a) * Math.cos(e), Math.sin(e), Math.cos(a) * Math.cos(e)).multiplyScalar(d));
        push(edit3d({ operation: 'add_light', name, light: { type: 'area', name, p: { x: at.x, y: at.y, z: at.z, targetX: centre.x, targetY: centre.y, targetZ: centre.z, width: radius * 2.2 * scale, height: radius * (name === 'Rim strip' ? 0.35 : 1.4) * scale, intensity } } }));
      }
      // A new camera carries the set itself; an existing one switches it on.
      if (camera) push(edit3d({ operation: 'set_studio', target: camera.id, on: true }));
      return { commands, id };
    }
    case 'soften': {
      // Scale a softbox: larger is softer.
      const light = PM.L(args.target);
      const data = light?.d?.data?.light;
      if (data?.type !== 'area') throw new Error('Choose a softbox (area light)');
      const k = Math.max(0.1, Math.min(10, args.amount ?? 1.15));
      return edit3d({ operation: 'update_light', target: light.id, patch: { p: { width: Number(data.p.width.v) * k, height: Number(data.p.height.v) * k } } });
    }
    case 'set_studio':
      return edit3d({ operation: 'set_environment', patch: { shadowCatcher: args.on !== false, ...(args.color ? { background: args.color } : {}) } });
    case 'shot': {
      const start = args.start ?? PM.time ?? 0, duration = Math.max(0.1, args.duration ?? 4), end = start + duration;
      const commands: any[] = [];
      switch (args.shot) {
        case 'turntable': {
          const layer = PM.L(args.target) || PM.L(PM.sel?.layers?.[0]);
          if (!layer || layer3DRole(layer) !== 'object' && layer.type !== 'group') throw new Error('A turntable needs a model or model group');
          const from = Number(layer.p?.['rotation.y']?.v ?? 0);
          commands.push(keys(layer.id, 'rotation.y', [[start, from], [end, from + (args.degrees ?? 360)]]));
          return { commands, id: layer.id };
        }
        case 'push_in': case 'orbit': case 'hero_tilt': {
          const camera = camLayer(PM);
          if (!camera) throw new Error('Add a camera first');
          const eye = vec(camera), aim = aimOf(camera), offset = eye.clone().sub(aim);
          const samples = args.shot === 'push_in' ? 2 : 9;
          const frames: Record<'x' | 'y' | 'z', [number, number][]> = { x: [], y: [], z: [] };
          for (let i = 0; i < samples; i++) {
            const k = i / (samples - 1), t = start + duration * k;
            let p = eye.clone();
            if (args.shot === 'push_in') p = aim.clone().add(offset.clone().multiplyScalar(1 - (args.amount ?? 0.3) * k));
            if (args.shot === 'orbit') p = aim.clone().add(offset.clone().applyAxisAngle(new THREE.Vector3(0, 1, 0), THREE.MathUtils.degToRad((args.degrees ?? 90) * k)));
            if (args.shot === 'hero_tilt') {
              const axis = new THREE.Vector3().crossVectors(offset, new THREE.Vector3(0, 1, 0)).normalize();
              p = aim.clone().add(offset.clone().applyAxisAngle(axis, THREE.MathUtils.degToRad(-(args.degrees ?? 25) * (1 - k))));
            }
            frames.x.push([t, p.x]); frames.y.push([t, p.y]); frames.z.push([t, p.z]);
          }
          for (const a of ['x', 'y', 'z'] as const) commands.push(keys(camera.id, `position.${a}`, frames[a]));
          return { commands, id: camera.id };
        }
        case 'rack_focus': {
          const camera = camLayer(PM);
          if (!camera) throw new Error('Add a camera first');
          const eye = vec(camera);
          const near = args.from ?? eye.distanceTo(aimOf(camera)) * 0.7;
          const target = args.target ? PM.L(args.target) : null;
          const far = args.to ?? (target ? eye.distanceTo(vec(target)) : near * 1.6);
          commands.push(keys(camera.id, 'camera.focusDistance', [[start, near], [end, far]]));
          return { commands, id: camera.id };
        }
        default: throw new Error(`Unknown shot: ${args.shot}. Shots: ${SHOT_TYPES.map(s => s.id).join(', ')}`);
      }
    }
    default: throw new Error('Unknown Studio operation');
  }
}
