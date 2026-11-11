import * as THREE from 'three';
import { compositionRuntime } from './service';
import { GROUP_PART_KEY, PART_DEFAULTS, PART_FIELDS, listLayerParts, partNode, partsRoot } from './model-parts';

const neutral = () => Object.fromEntries(PART_FIELDS.map(field => [field, { v: PART_DEFAULTS[field], kf: [], expr: null }]));
export function editPartGroup(PM: any, layer: any, content: any, args: { operation: string; parts?: string[]; part?: string; name?: string }): string | undefined {
  const parts = content.object.parts ||= {}, tree = listLayerParts(PM, layer), byKey = new Map(tree.map(part => [part.key, part]));
  if (args.operation === 'group_parts') {
    const keys = [...new Set(args.parts || [])], selected = keys.map(key => byKey.get(key));
    if (keys.length < 2 || selected.some(part => !part)) throw new Error('Select at least two model parts');
    const parent = selected[0]!.parent;
    if (selected.some(part => part!.parent !== parent)) throw new Error('Select parts at the same level to group them');
    if (tree.length >= 4096) throw new Error('This model has reached its part limit');
    const model = compositionRuntime(PM, PM.time).objects.get(layer.id);
    if (!model) throw new Error('Wait for the model to load');
    model.updateWorldMatrix(true, true);
    const root = partsRoot(model), inverse = model.matrixWorld.clone().invert(), box = new THREE.Box3();
    for (const key of keys) {
      const node = partNode(root, key); if (!node) throw new Error('That part is not in this model');
      box.union(node.userData.partBounds || new THREE.Box3().setFromObject(node));
    }
    if (box.isEmpty()) throw new Error('The selected parts have no geometry');
    const pivot = box.getCenter(new THREE.Vector3()).applyMatrix4(inverse);
    // Group coordinates precede the existing ancestor group's transform.
    for (let ancestor = parent; ancestor; ancestor = byKey.get(ancestor)?.parent ?? null) {
      if (!GROUP_PART_KEY.test(ancestor)) continue;
      const handle = partNode(root, ancestor), origin = parts[ancestor]?.pivot;
      if (handle && origin) pivot.applyMatrix4(handle.matrix.clone().multiply(new THREE.Matrix4().makeTranslation(-origin[0], -origin[1], -origin[2])).invert());
      break;
    }
    const key = `group_${PM.uid('g')}`, name = String(args.name || 'Part Group').trim().slice(0, 160);
    parts[key] = { name, parent, members: keys, pivot: pivot.toArray(), p: neutral() };
    for(const member of keys)if(parts[member]?.members)parts[member].parent=key;
    if (parent && parts[parent]?.members) {
      const children: string[] = parts[parent].members, at = Math.min(...keys.map(key => children.indexOf(key)).filter(index => index >= 0));
      parts[parent].members = children.filter(key => !keys.includes(key)); parts[parent].members.splice(at, 0, key);
    }
    return key;
  }
  const key = args.part || '', group = parts[key];
  if (!GROUP_PART_KEY.test(key) || !group?.members) throw new Error('Choose a part group');
  if (args.operation === 'rename_part_group') {
    const name = String(args.name || '').trim(); if (!name || name.length > 160) throw new Error('Enter a group name');
    group.name = name; return key;
  }
  if (Object.values<any>(group.p).some(prop => prop.kf?.length || prop.expr)) throw new Error('Remove group animation before ungrouping');
  const v = Object.fromEntries(Object.entries<any>(group.p).map(([field, prop]) => [field, prop.v])), pivot = group.pivot;
  const matrix = new THREE.Matrix4().fromArray(group.matrix || new THREE.Matrix4().elements)
    .multiply(new THREE.Matrix4().compose(new THREE.Vector3(pivot[0] + v.x, pivot[1] + v.y, pivot[2] + v.z),
      new THREE.Quaternion().setFromEuler(new THREE.Euler(...[v.rx, v.ry, v.rz].map(THREE.MathUtils.degToRad) as [number,number,number], 'XYZ')),
      new THREE.Vector3().setScalar(v.scale / 100)))
    .multiply(new THREE.Matrix4().makeTranslation(-pivot[0], -pivot[1], -pivot[2]));
  const parent = byKey.get(key)?.parent ?? null;
  for (const member of group.members) {
    const part = parts[member] ||= { name: byKey.get(member)?.name || 'Part', p: neutral() };
    part.matrix = matrix.clone().multiply(new THREE.Matrix4().fromArray(part.matrix || new THREE.Matrix4().elements)).toArray();
    if (part.members) part.parent = parent;
    if (!v.visible) part.p.visible = { v: false, kf: [], expr: null };
  }
  if (parent && parts[parent]?.members) parts[parent].members = parts[parent].members.flatMap((member: string) => member === key ? group.members : [member]);
  delete parts[key]; return undefined;
}
