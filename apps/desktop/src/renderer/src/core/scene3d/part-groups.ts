import * as THREE from 'three';
import { compositionRuntime } from './service';
import { GROUP_PART_KEY, PART_DEFAULTS, PART_FIELDS, listLayerParts, partNode, partsRoot, partPath } from './model-parts';

const neutral = () => Object.fromEntries(PART_FIELDS.map(field => [field, { v: PART_DEFAULTS[field], kf: [], expr: null }]));
export function editPartGroup(PM: any, layer: any, content: any, args: { operation: string; parts?: string[]; part?: string; name?: string; patch?: Record<string,unknown> }): string | undefined {
  const parts = content.object.parts ||= {}, tree = listLayerParts(PM, layer), byKey = new Map(tree.map(part => [part.key, part]));
  if (args.operation === 'move_parts') {
    const destination=args.part || null;
    if(destination && !parts[destination]?.members)throw new Error('Drop onto a part group');
    const requested=new Set(args.parts || []);
    if(!requested.size || [...requested].some(key=>!byKey.has(key)))throw new Error('Choose parts from this model');
    const ancestors=(key:string)=>{
      const chain:string[]=[];
      for(let parent=byKey.get(key)?.parent;parent;parent=byKey.get(parent)?.parent){
        if(chain.includes(parent))throw new Error('Invalid part hierarchy');chain.push(parent);
      }
      return chain;
    };
    const keys=[...requested].filter(key=>!ancestors(key).some(parent=>requested.has(parent)));
    if(destination && keys.some(key=>key===destination || ancestors(destination).includes(key)))throw new Error('A group cannot contain itself or its ancestors');
    // File nodes retain their glTF hierarchy for animation and skin bindings.
    // Membership can move across any custom groups sharing that file level.
    const fileLevel=(key:string)=>GROUP_PART_KEY.test(key)?ancestors(key).find(parent=>!GROUP_PART_KEY.test(parent)) ?? null
      :key.includes('_')?key.slice(0,key.lastIndexOf('_')):null;
    const level=destination?fileLevel(destination):fileLevel(keys[0]!);
    if(keys.some(key=>fileLevel(key)!==level))throw new Error('Move parts into a group at the same model level');
    const frame=(parent:string|null)=>{
      const matrix=new THREE.Matrix4();
      for(let key=parent;key && GROUP_PART_KEY.test(key);key=byKey.get(key)?.parent ?? null){
        const group=parts[key],v=Object.fromEntries(PART_FIELDS.map(field=>[field,PM.evP(layer,group.p[field],PM.time,partPath(key,field))]));
        matrix.premultiply(groupMatrix(group,v));
      }
      return matrix;
    };
    const targetFrame=frame(destination);
    if(Math.abs(targetFrame.determinant())<1e-12)throw new Error('Increase the destination group’s zero scale before adding parts');
    const inverse=targetFrame.invert();
    // Snapshot every correction before changing memberships or removing groups.
    const moves=keys.filter(key=>byKey.get(key)!.parent!==(destination || level)).map(key=>({key,
      matrix:inverse.clone().multiply(frame(byKey.get(key)!.parent)).multiply(new THREE.Matrix4().fromArray(parts[key]?.matrix || new THREE.Matrix4().elements))}));
    for(const {key,matrix} of moves){
      const part=parts[key] ||= {name:byKey.get(key)!.name,p:neutral()};
      const previous=byKey.get(key)!.parent;
      if(previous && parts[previous]?.members)parts[previous].members=parts[previous].members.filter((member:string)=>member!==key);
      part.matrix=matrix.toArray();
      if(part.members)part.parent=destination || level;
      if(destination)parts[destination].members.push(key);
    }
    // Removing a final member removes its empty controller, including empty ancestors.
    let empty:string|undefined;
    while((empty=Object.keys(parts).find(key=>parts[key].members?.length===0))){
      for(const group of Object.values<any>(parts))if(group.members)group.members=group.members.filter((member:string)=>member!==empty);
      delete parts[empty];
    }
    return destination || undefined;
  }
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
  if(args.operation==='set_part_group_anchor'){
    const patch=args.patch || {};
    if(!Object.keys(patch).length || Object.keys(patch).some(axis=>!['x','y','z'].includes(axis)))throw new Error('Choose an anchor point axis');
    const anchor=group.pivot.slice();
    for(const [axis,value] of Object.entries(patch)){
      if(typeof value!=='number' || !Number.isFinite(value) || Math.abs(value)>100000)throw new Error('Enter a finite anchor point coordinate');
      anchor[['x','y','z'].indexOf(axis)]=value;
    }
    const v=Object.fromEntries(PART_FIELDS.map(field=>[field,PM.evP(layer,group.p[field],PM.time,partPath(key,field))]));
    const shift=new THREE.Vector3().fromArray(group.pivot).sub(new THREE.Vector3().fromArray(anchor));
    const turned=shift.clone().applyEuler(new THREE.Euler(...[v.rx,v.ry,v.rz].map(THREE.MathUtils.degToRad) as [number,number,number],'XYZ')).multiplyScalar(v.scale/100);
    // Rebase the static frame at the playhead; position and rotation keys stay intact.
    const correction=shift.sub(turned);
    group.matrix=new THREE.Matrix4().fromArray(group.matrix || new THREE.Matrix4().elements)
      .multiply(new THREE.Matrix4().makeTranslation(correction.x,correction.y,correction.z)).toArray();
    group.pivot=anchor;return key;
  }
  if (Object.values<any>(group.p).some(prop => prop.kf?.length || prop.expr)) throw new Error('Remove group animation before ungrouping');
  const v = Object.fromEntries(Object.entries<any>(group.p).map(([field, prop]) => [field, prop.v]));
  const matrix = groupMatrix(group,v);
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

/** Groups act in model coordinates; the retained matrix precedes their channels. */
function groupMatrix(group:any,v:Record<string,any>):THREE.Matrix4 {
  const pivot=group.pivot;
  return new THREE.Matrix4().fromArray(group.matrix || new THREE.Matrix4().elements)
    .multiply(new THREE.Matrix4().compose(new THREE.Vector3(pivot[0]+v.x,pivot[1]+v.y,pivot[2]+v.z),
      new THREE.Quaternion().setFromEuler(new THREE.Euler(...[v.rx,v.ry,v.rz].map(THREE.MathUtils.degToRad) as [number,number,number],'XYZ')),
      new THREE.Vector3().setScalar(v.scale/100)))
    .multiply(new THREE.Matrix4().makeTranslation(-pivot[0],-pivot[1],-pivot[2]));
}
