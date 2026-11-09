import * as THREE from 'three';
import { compositionRuntime } from './service';
import { isExtrudedLayer, isShared3DPlane } from './extrusion-data';
import { layer3DRole } from './layers';
import {usesSceneUnits} from './control-space';
import { nativeSceneBridge } from './native-transform';
import { parent3D, world3D } from '../../legacy/core/space-3d';
import type { Bounds3D } from '../../kernel/api';
import type { EditMeta, EditResult } from '../types/commands';

/** Both group membership and transform parenting contribute to the scope. */
export function boundsScope(PM:any,ids:readonly string[]) {
  const layers:any[]=(PM.curComp?.() || PM.proj).layers;
  const byId=new Map(layers.map(layer=>[layer.id,layer]));
  const selected=new Set(ids);
  if(!selected.size)throw new Error('Select visual layers first');
  for(const id of selected)if(!byId.has(id))throw new Error(`Layer not found: ${id}`);
  const ancestors=new Map<string,Set<string>>(),visiting=new Set<string>();
  const visit=(id:string):Set<string>=>{
    if(visiting.has(id))throw new Error('Cyclic layer hierarchy');
    const cached=ancestors.get(id);if(cached)return cached;
    const layer=byId.get(id);if(!layer)throw new Error(`Missing hierarchy parent: ${id}`);
    visiting.add(id);const out=new Set<string>();
    for(const parent of [layer.parent,layer.group].filter(Boolean)){
      out.add(parent);for(const ancestor of visit(parent))out.add(ancestor);
    }
    visiting.delete(id);ancestors.set(id,out);return out;
  };
  for(const layer of layers)visit(layer.id);
  const roots=layers.filter(layer=>selected.has(layer.id)&&![...visit(layer.id)].some(id=>selected.has(id)));
  const rootIds=new Set(roots.map(layer=>layer.id));
  const affected=layers.filter(layer=>rootIds.has(layer.id)||[...visit(layer.id)].some(id=>rootIds.has(id)));
  return {layers,byId,roots,affected,ancestors};
}

/** Unprojected scene units: X right, Y up, Z toward the default camera.
 * Native artwork is converted using the very same bridge as mesh rendering. */
export function geometryBounds(PM:any,ids:readonly string[],time=PM.time || 0):Bounds3D|null {
  if(!Number.isFinite(time))throw new Error('Bounds time must be finite');
  const scope=boundsScope(PM,ids),comp=PM.curComp?.() || PM.proj;
  const bridge=nativeSceneBridge(comp),box=new THREE.Box3(),meshIds:string[]=[];
  PM.beginEval?.(time);
  for(const layer of scope.affected){
    if(layer.type==='group'||layer.type==='null'||layer.type==='audio'||!PM.active(layer,time))continue;
    const role=layer3DRole(layer);if(role==='camera'||role==='light')continue;
    if(role==='object'||isExtrudedLayer(layer)||isShared3DPlane(comp,layer)){meshIds.push(layer.id);continue;}
    const bounds=PM.GL?.bounds?.(layer,time);
    if(!bounds)throw new Error(`Cannot measure ${layer.name}`);
    const matrix=bridge.clone().multiply(new THREE.Matrix4().fromArray(layer.threeD?world3D(PM,layer,time):affine4(PM.worldMatrix(layer,time))));
    for(const x of [bounds.x0,bounds.x1])for(const y of [bounds.y0,bounds.y1])box.expandByPoint(new THREE.Vector3(x,y,0).applyMatrix4(matrix));
  }
  if(meshIds.length)box.union(compositionRuntime(PM,time).geometryBounds(meshIds));
  if(box.isEmpty())return null;
  if(![...box.min.toArray(),...box.max.toArray()].every(Number.isFinite))throw new Error('Geometry has non-finite bounds');
  return {x0:box.min.x,y0:box.min.y,z0:box.min.z,x1:box.max.x,y1:box.max.y,z1:box.max.z};
}

function affine4(m:number[]):number[]{return [m[0]!,m[1]!,0,0,m[2]!,m[3]!,0,0,0,0,1,0,m[4]!,m[5]!,0,1];}
const labels=['Top Left','Top Center','Top Right','Center Left','Center','Center Right','Bottom Left','Bottom Center','Bottom Right'];

/** Lift existing null controllers and preserve entire parent branches. The
 * frontier sits immediately below the nearest shared parent/group context. */
function controllerRoots(scope:ReturnType<typeof boundsScope>):any[] {
  const candidates=new Set(scope.roots.map(layer=>layer.id));
  for(const layer of scope.roots)for(const id of scope.ancestors.get(layer.id)!)if(scope.byId.get(id)?.type==='null')candidates.add(id);
  const common=new Set(scope.ancestors.get([...candidates][0]!)!);
  for(const id of candidates)for(const ancestor of common)if(!scope.ancestors.get(id)!.has(ancestor))common.delete(ancestor);
  for(let attempt=0;attempt<=scope.layers.length;attempt++){
    const covered=new Set(candidates);
    for(const id of candidates)for(const ancestor of scope.ancestors.get(id)!)if(!common.has(ancestor))covered.add(ancestor);
    const roots=scope.layers.filter(layer=>covered.has(layer.id)&&![...scope.ancestors.get(layer.id)!].some(id=>covered.has(id)));
    const context=(layer:any)=>JSON.stringify([layer.parent||null,layer.group||null]);
    if(roots.every(layer=>context(layer)===context(roots[0])))return roots;
    let removed=false;
    for(const layer of roots)for(const id of [layer.parent,layer.group])if(common.delete(id))removed=true;
    if(!removed)break;
  }
  throw new Error('Cannot insert a shared controller into this hierarchy');
}

/** Insert one identity controller without rebasing animated source channels. */
export function createBoundsNull(PM:any,ids:readonly string[],index:number,options:{replace?:boolean}={},meta:EditMeta={}):EditResult {
  try {
    if(!Number.isInteger(index)||index<0||index>8)throw new Error('Choose one of the nine bounds positions');
    if(meta.baseRevision!=null&&meta.baseRevision!==(PM.proj.revision || 0))throw new Error('Project changed; click again');
    const time=PM.time || 0,scope=boundsScope(PM,ids),comp=PM.curComp?.() || PM.proj;
    let existing:any,roots:any[];
    if(options.replace){
      const root=scope.roots[0];
      existing=scope.roots.length===1&&root.type==='null'?root:scope.byId.get(root.parent);
      if(!existing||existing.type!=='null'||scope.roots.some(layer=>layer!==existing&&layer.parent!==existing.id))throw new Error('Select a null or its direct children to replace its pivot');
      if(!scope.roots.includes(existing)&&scope.layers.some(layer=>(layer.parent===existing.id||layer.group===existing.id)&&!scope.roots.includes(layer)))throw new Error('Select every direct child of the null to replace its pivot');
      requireIdentityNull(existing);
      for(const axis of ['x','y','z'])for(const channel of ['position','anchor'])if(existing.locked_intent?.[`${channel}.${axis}`])throw new Error(`Protected null channel: ${channel}.${axis}`);
      roots=[existing];
    }else roots=controllerRoots(scope);
    const moved=boundsScope(PM,roots.map(layer=>layer.id));
    const protectedLayers=new Set([...scope.affected,...moved.affected,...roots.flatMap(layer=>[...scope.ancestors.get(layer.id)!].map(id=>scope.byId.get(id)))]);
    for(const layer of protectedLayers)if(layer.lock)throw new Error(`Unlock ${layer.name} first`);
    if(!options.replace)for(const layer of scope.layers){
      if(PM.allProps(layer).some(({prop}:any)=>prop.expr&&PM.compileExpr(prop.expr)?.needsIdx))throw new Error('Index-dependent expressions must be made independent of layer order before inserting a null');
    }
    // Do not infer an editable pivot through a collapsed existing hierarchy.
    for(const layer of scope.roots){
      const matrix=new THREE.Matrix4().fromArray(parent3D(PM,layer,time));
      if(!Number.isFinite(matrix.determinant())||Math.abs(matrix.determinant())<1e-12)throw new Error('Parent transform has zero scale');
    }
    const bounds=geometryBounds(PM,ids,time);if(!bounds)throw new Error('The selection has no active visual geometry');
    const selected=ids.map(id=>scope.byId.get(id)!);
    const from=Math.min(...selected.map(layer=>layer.from)),end=Math.max(...selected.map(layer=>layer.from+layer.dur));
    const model=existing?usesSceneUnits(PM,existing):moved.affected.some(layer=>!!layer3DRole(layer));
    const threeD=existing?!!existing.threeD:model||moved.affected.some(layer=>layer.threeD);
    const root=roots[0],probe=existing||{id:'bounds-null-probe',type:'null',threeD,parent:root.parent,group:root.group,d:{boundsNullSpace:model?'scene':'composition'}};
    const parent=new THREE.Matrix4().fromArray(threeD?parent3D(PM,probe,time):affine4(PM.transformParentMatrix(probe,time)));
    if(!Number.isFinite(parent.determinant())||Math.abs(parent.determinant())<1e-12)throw new Error('Parent transform has zero scale');
    const point=new THREE.Vector3(bounds.x0+(bounds.x1-bounds.x0)*(index%3)/2,bounds.y1-(bounds.y1-bounds.y0)*Math.floor(index/3)/2,(bounds.z0+bounds.z1)/2);
    if(!model)point.applyMatrix4(nativeSceneBridge(comp).invert());
    point.applyMatrix4(parent.invert());if(!threeD)point.z=0;
    return PM.Edit.mutate(`Bounds Null ${options.replace?'Replace':'Create'}`,()=>{
      const node=existing||PM.mkLayer('null',{name:`${selected.map(layer=>layer.name).join(' + ')} - Bounds Null - ${labels[index]}`,from,dur:end-from,d:{boundsNullSpace:model?'scene':'composition'}});
      for(const [i,axis] of ['x','y','z'].entries())for(const channel of ['position','anchor']){
        node.p[`${channel}.${axis}`] ||= PM.P(0);node.p[`${channel}.${axis}`].v=point.getComponent(i);
      }
      if(!existing){
        node.threeD=threeD;node.parent=root.parent||null;node.group=root.group||null;
        scope.layers.splice(Math.min(...roots.map(layer=>scope.layers.indexOf(layer))),0,node);
        for(const layer of roots)layer.parent=node.id;
      }
      PM.touch();PM.selectLayers([node.id]);return {nullIds:[node.id]};
    },{...meta,baseRevision:PM.proj.revision || 0});
  }catch(error){return {ok:false,message:error instanceof Error?error.message:String(error)};}
}

function requireIdentityNull(layer:any):void {
  if(layer.lock)throw new Error(`Unlock ${layer.name} first`);
  for(const [key,prop] of Object.entries<any>(layer.p)){
    if(!/^(position\.|anchor\.|scale\.|rotation($|\.)|orientation\.|skew)/.test(key))continue;
    if(prop.expr||prop.kf?.length)throw new Error('Replace requires a static identity null; create a new bounds null to retain this animation');
  }
  for(const axis of ['x','y','z'])if((layer.p[`position.${axis}`]?.v || 0)!==(layer.p[`anchor.${axis}`]?.v || 0)||(layer.p[`scale.${axis}`]?.v ?? 100)!==100)throw new Error('Replace requires a static identity null');
  for(const key of ['rotation','rotation.x','rotation.y','orientation.x','orientation.y','orientation.z','skew'])if(layer.p[key]?.v)throw new Error('Replace requires a static identity null');
}
