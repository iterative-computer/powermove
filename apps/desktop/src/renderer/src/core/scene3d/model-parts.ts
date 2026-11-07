import * as THREE from 'three';

/*
 * Parts of an imported model. A glTF keeps its node tree (a console body, two
 * controllers, a stick…). Each node is addressable by its child-index path
 * under the file's scene ("0_2_1"), so one model layer can move, turn, scale
 * or hide its own pieces without being split into several layers.
 *
 * Offsets are sparse layer channels (`pt.<key>.<field>`), applied on top of
 * the file's pose (and its baked animation). Position is measured in the
 * model's own units along the model's own axes, the same units as the layer's
 * Position at 100% scale; rotation turns the part about its own origin around
 * the model's axes; scale is a uniform percentage.
 */

export const PART_FIELDS = ['x', 'y', 'z', 'rx', 'ry', 'rz', 'scale', 'visible'] as const;
export type PartField = typeof PART_FIELDS[number];
export type PartValues = { x: number; y: number; z: number; rx: number; ry: number; rz: number; scale: number; visible: boolean };
export const PART_DEFAULTS: PartValues = { x: 0, y: 0, z: 0, rx: 0, ry: 0, rz: 0, scale: 100, visible: true };
export const PART_KEY = /^(?:0|[1-9][0-9]{0,3})(?:_(?:0|[1-9][0-9]{0,3})){0,63}$/;
export const PART_PATH = /^pt\.((?:0|[1-9][0-9]{0,3})(?:_(?:0|[1-9][0-9]{0,3})){0,63})\.(x|y|z|rx|ry|rz|scale|visible)$/;
export const partPath = (key: string, field: PartField): string => `pt.${key}.${field}`;

export interface ModelPart {
  key: string;
  name: string;
  parent: string | null;
  depth: number;
  /** Listed child parts. */
  children: number;
  mesh: boolean;
}

/** Set by the loader on the file's scene node; survives clones through userData. */
export const PARTS_ROOT_FLAG = 'powermovePartsRoot';

export function partsRoot(object: THREE.Object3D | null | undefined): THREE.Object3D | null {
  if (!object) return null;
  // The loader wraps the file scene as root → normalization → scene.
  const queue: [THREE.Object3D, number][] = [[object, 0]];
  while (queue.length) {
    const [node, depth] = queue.shift()!;
    if (node.userData?.[PARTS_ROOT_FLAG]) return node;
    if (depth < 3) for (const child of node.children) queue.push([child, depth + 1]);
  }
  return null;
}

/** glTF-named nodes and anything holding them. Unnamed multi-primitive
 * children move with their named node and are not listed separately. */
const listed = (node: THREE.Object3D): boolean => node.userData?.name !== undefined || node.children.some(listed);
const label = (node: THREE.Object3D, key: string): string => {
  const raw = typeof node.userData?.name === 'string' && node.userData.name.trim() ? node.userData.name : node.name;
  return (raw || `Part ${key.replaceAll('_', '.')}`).slice(0, 160);
};

export function listParts(root: THREE.Object3D | null): ModelPart[] {
  if (!root) return [];
  const out: ModelPart[] = [];
  const walk = (node: THREE.Object3D, key: string | null, depth: number) => {
    node.children.forEach((child, index) => {
      if (!listed(child) || out.length >= 4096) return;
      const childKey = key == null ? String(index) : `${key}_${index}`;
      let mesh = false;
      child.traverse((c: any) => { if (c.isMesh) mesh = true; });
      const entry: ModelPart = { key: childKey, name: label(child, childKey), parent: key, depth, children: child.children.filter(listed).length, mesh };
      out.push(entry);
      walk(child, childKey, depth + 1);
    });
  };
  walk(root, null, 0);
  return out;
}

export function partNode(root: THREE.Object3D | null, key: string): THREE.Object3D | null {
  if (!root || !PART_KEY.test(key)) return null;
  let node: THREE.Object3D | undefined = root;
  for (const index of key.split('_')) { node = node?.children[Number(index)]; if (!node) return null; }
  return node ?? null;
}

/** The part key of a node inside a model (the nearest listed ancestor-or-self). */
export function partKeyOf(root: THREE.Object3D | null, node: THREE.Object3D | null): string | null {
  if (!root || !node) return null;
  const path: number[] = [];
  let cur: THREE.Object3D | null = node;
  while (cur && cur !== root) {
    const parent: THREE.Object3D | null = cur.parent;
    if (!parent) return null;
    path.unshift(parent.children.indexOf(cur));
    cur = parent;
  }
  if (cur !== root || !path.length) return null;
  // Climb past unlisted nodes (unnamed primitives) to their named owner.
  let target: THREE.Object3D | null = node;
  while (target && target !== root && !listed(target)) { target = target.parent; path.pop(); }
  return path.length ? path.join('_') : null;
}

/** Remember the file pose of every node once, when the model is created. */
export function captureRestPose(object: THREE.Object3D): void {
  const root = partsRoot(object);
  root?.traverse(node => {
    if (node === root) return;
    node.userData.partRest = { position: node.position.clone(), quaternion: node.quaternion.clone(), scale: node.scale.clone(), visible: node.visible };
  });
}

/** Undo the previous frame's offsets before the model's own animation runs. */
export function restoreParts(object: THREE.Object3D): void {
  const root = partsRoot(object);
  root?.traverse(node => {
    const rest = node.userData.partRest;
    if (!node.userData.partApplied || !rest) return;
    node.position.copy(rest.position); node.quaternion.copy(rest.quaternion); node.scale.copy(rest.scale); node.visible = rest.visible;
    node.userData.partApplied = false;
  });
}

/** Linear map from a node's parent space into the model root's space. */
function parentToModel(node: THREE.Object3D, model: THREE.Object3D): THREE.Matrix4 {
  const m = new THREE.Matrix4();
  for (let cur = node.parent; cur && cur !== model; cur = cur.parent) { cur.updateMatrix(); m.premultiply(cur.matrix); }
  return m;
}
const rotationOf = (m: THREE.Matrix4): THREE.Quaternion => {
  const q = new THREE.Quaternion(); m.decompose(new THREE.Vector3(), q, new THREE.Vector3()); return q;
};
const rad = THREE.MathUtils.degToRad;

/** Apply evaluated offsets. Ancestors go first so children follow them. */
export function applyParts(object: THREE.Object3D, parts: Record<string, Partial<PartValues>> | undefined): void {
  const root = partsRoot(object);
  if (!root || !parts) return;
  const keys = Object.keys(parts).filter(key => PART_KEY.test(key)).sort((a, b) => a.split('_').length - b.split('_').length);
  for (const key of keys) {
    const node = partNode(root, key);
    if (!node) continue;
    const v = { ...PART_DEFAULTS, ...parts[key] };
    for (const field of ['x', 'y', 'z', 'rx', 'ry', 'rz', 'scale'] as const) if (!Number.isFinite(v[field])) v[field] = PART_DEFAULTS[field];
    // The base this frame: file rest pose, or the file's own animation.
    node.userData.partBase = { position: node.position.clone(), quaternion: node.quaternion.clone(), scale: node.scale.clone() };
    const toModel = parentToModel(node, object);
    if (v.x || v.y || v.z) {
      const inverse = new THREE.Matrix3().setFromMatrix4(toModel).invert();
      node.position.add(new THREE.Vector3(v.x, v.y, v.z).applyMatrix3(inverse));
    }
    if (v.rx || v.ry || v.rz) {
      const frame = rotationOf(toModel);
      const turn = new THREE.Quaternion().setFromEuler(new THREE.Euler(rad(v.rx), rad(v.ry), rad(v.rz), 'XYZ'));
      node.quaternion.premultiply(frame.clone().invert().multiply(turn).multiply(frame));
    }
    if (v.scale !== 100) node.scale.multiplyScalar(Math.max(0, v.scale) / 100);
    if (v.visible === false) node.visible = false;
    node.userData.partApplied = true;
  }
}

/** The offsets that put a part at a new local pose (for viewer gestures). */
export function partOffsetsForLocal(object: THREE.Object3D, node: THREE.Object3D, local: THREE.Matrix4): Omit<PartValues, 'visible'> {
  const base = node.userData.partBase || node.userData.partRest || { position: node.position, quaternion: node.quaternion, scale: node.scale };
  const position = new THREE.Vector3(), quaternion = new THREE.Quaternion(), scale = new THREE.Vector3();
  local.decompose(position, quaternion, scale);
  const toModel = parentToModel(node, object), frame = rotationOf(toModel);
  const offset = position.sub(base.position).applyMatrix3(new THREE.Matrix3().setFromMatrix4(toModel));
  const turn = frame.clone().multiply(quaternion).multiply(base.quaternion.clone().invert()).multiply(frame.clone().invert());
  const euler = new THREE.Euler().setFromQuaternion(turn, 'XYZ'), deg = THREE.MathUtils.radToDeg;
  const baseScale = Math.abs(base.scale.x) > 1e-12 ? base.scale.x : 1;
  return { x: offset.x, y: offset.y, z: offset.z, rx: deg(euler.x), ry: deg(euler.y), rz: deg(euler.z), scale: scale.x / baseScale * 100 };
}

/** The parts of a model layer's file, or [] while it loads or for primitives. */
export function listLayerParts(PM: any, layer: any): ModelPart[] {
  const assetId = layer?.d?.data?.object?.source?.assetId;
  const object3d = assetId ? PM.assets?.get?.(assetId)?.object3d : null;
  return listParts(partsRoot(object3d));
}

/* ── the part being edited (viewer gizmo, outline and Properties share it) ── */
const activeParts = new WeakMap<object, Map<string, string>>();
const partListeners = new WeakMap<object, Set<() => void>>();
export function getActivePart(PM: any, layerId: string): string | null {
  return activeParts.get(PM)?.get(layerId) ?? null;
}
export function setActivePart(PM: any, layerId: string, key: string | null): void {
  let map = activeParts.get(PM); if (!map) { map = new Map(); activeParts.set(PM, map); }
  if ((map.get(layerId) ?? null) === key) return;
  if (key == null) map.delete(layerId); else { if (!PART_KEY.test(key)) throw new Error('Invalid model part'); map.set(layerId, key); }
  for (const listener of partListeners.get(PM) || []) listener();
  PM.invalidate?.('render');
}
export function onActivePartChange(PM: any, listener: () => void): () => void {
  let set = partListeners.get(PM); if (!set) { set = new Set(); partListeners.set(PM, set); }
  set.add(listener); return () => { set!.delete(listener); };
}
/** Leaving a model (selecting something else) ends part editing on it. */
export function pruneActiveParts(PM: any, selected: readonly string[]): void {
  const map = activeParts.get(PM); if (!map) return;
  for (const id of [...map.keys()]) if (!selected.includes(id)) setActivePart(PM, id, null);
}
