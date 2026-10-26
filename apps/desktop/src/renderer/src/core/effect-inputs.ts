import type { EffectLayerInput, EffectInputStage, EffectInputSpace, EffectInput } from './types/project';

export class EffectInputError extends Error { override name = 'EffectInputError'; }
/** Untimed keys retain the original API. Frame keys also identify time and the
 * exclusive owner effect boundary; complete Effects and a prefix never alias. */
export const inputKey = (id: string, stage: EffectInputStage, space: EffectInputSpace = 'composition', time?:number, prefix?:number) => JSON.stringify(time===undefined ? space==='composition'?[id,stage]:[id,stage,space] : [id,stage,space,time,prefix??null]);
export const MAX_FRAME_INPUTS=256;
export function temporalTimes(def:any, value:(key:string)=>unknown, time:number):number[]{
 const t=def.temporal;if(!t)return[];
 const count=Number(value(t.count)),step=Number(value(t.step));
 if(!Number.isFinite(count)||!Number.isFinite(step)||count<0||Math.round(count)>t.maxSamples||Math.abs(step)>3600)throw new EffectInputError('Temporal inputs exceed the sample or time limits');
 return Array.from({length:Math.round(count)},(_,i)=>{const at=time+(i+1)*step;if(!Number.isFinite(at))throw new EffectInputError('Invalid temporal input timestamp');return Object.is(at,-0)?0:at;});
}
/** Static disabled effects do not need a dependency graph or exact-video path.
 * Animated/expression enables remain conservative for history/export planning. */
export function effectMightRender(fx:any):boolean{
 if(fx.missing===true||fx.on===false)return false;
 const on=fx.on;return !(on&&typeof on==='object'&&on.v===false&&!on.kf?.length&&!on.expr);
}
export function hasLayerInputs(project: any, definition?:(fx:any)=>any): boolean {
  const pending = [project], seen = new Set<any>();
  while (pending.length) { const comp = pending.pop(); if (!comp || seen.has(comp)) continue; seen.add(comp);
    if ((comp.layers || []).some((layer:any)=>{
      if(layer.effectSpace!==undefined&&!['composition','source'].includes(layer.effectSpace))return true;
      return (layer.fx??[]).some((fx:any)=>effectMightRender(fx)&&(layer.effectSpace==='source'||Object.keys(fx.inputs??{}).length>0||!!definition?.(fx)?.temporal));
    })) return true;
    pending.push(...Object.values(comp.comps || {}));
  }
  return false;
}

/** Static references deliberately live outside Channel/animation data. */
export function validateLayerInputs(value: unknown): Record<string, EffectLayerInput> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new EffectInputError('Effect inputs must be an object');
  const result: Record<string, EffectLayerInput> = {};
  for (const [key, ref] of Object.entries(value as Record<string, any>)) {
    if (!/^[a-z][a-zA-Z0-9]*$/.test(key) || ['constructor','prototype','__proto__'].includes(key)
      || !ref || typeof ref !== 'object' || Array.isArray(ref)
      || !(ref.layerId === null || typeof ref.layerId === 'string' && ref.layerId.trim().length > 0 && ref.layerId.length <= 256)
      || !['source','masks','effects'].includes(ref.stage)
      || ref.space !== undefined && !['composition','source'].includes(ref.space)
      || Object.keys(ref).some(k => !['layerId','stage','space'].includes(k))) throw new EffectInputError(`Invalid layer input “${key}”`);
    result[key] = { layerId: ref.layerId, stage: ref.stage, ...(ref.space === undefined ? {} : {space:ref.space}) };
  }
  if (Object.keys(result).length > 4) throw new EffectInputError('At most four layer inputs are supported per effect');
  return result;
}

export const referenceParam = (param:{type?:string}) => ['layer','path','asset'].includes(param.type??'');
export const emptyReference = (type:string) => type==='path'?{kind:'path',layerId:null,pathId:null}:type==='asset'?{kind:'asset',assetId:null}:{layerId:null,stage:'source'};
export function validateEffectInputs(value:unknown):Record<string,EffectInput>{
 if(!value||typeof value!=='object'||Array.isArray(value))throw new EffectInputError('Effect inputs must be an object');
 const out:Record<string,EffectInput>={};
 const id=(value:any)=>value===null||typeof value==='string'&&value.trim().length>0&&value.length<=256;
 for(const[key,ref]of Object.entries<any>(value)){
  if(!/^[a-z][a-zA-Z0-9]*$/.test(key)||['constructor','prototype','__proto__'].includes(key)||!ref||typeof ref!=='object'||Array.isArray(ref))throw new EffectInputError('Invalid effect reference');
  if(ref.kind==='path'){
   if(!id(ref.layerId)||!id(ref.pathId)||(ref.layerId===null)!==(ref.pathId===null)||Object.keys(ref).some(k=>!['kind','layerId','pathId'].includes(k)))throw new EffectInputError('Invalid path reference');
   out[key]={kind:'path',layerId:ref.layerId,pathId:ref.pathId};
  }else if(ref.kind==='asset'){
   if(!id(ref.assetId)||Object.keys(ref).some(k=>!['kind','assetId'].includes(k)))throw new EffectInputError('Invalid asset reference');out[key]={kind:'asset',assetId:ref.assetId};
  }else Object.assign(out,validateLayerInputs({[key]:ref}));
 }
 if(Object.keys(out).length>4)throw new EffectInputError('At most four effect references are supported');return out;
}
export const referencedPath=(layer:any,id:string)=>{const mask=layer?.masks?.find((m:any)=>m.id===id);if(mask)return{mask,path:mask.path};const path=layer?.d?.paths?.find((p:any)=>p.id===id);return path?{path}:null;};

export function remapLayerInputs(layer: any, ids: ReadonlyMap<string, string>): void {
  for (const fx of layer.fx || []) for (const ref of Object.values<any>(fx.inputs || {}))
    if (ref && ids.has(ref.layerId)) ref.layerId = ids.get(ref.layerId);
}

type Node = { key: string; layer: any; stage: EffectInputStage; space: EffectInputSpace; time?:number; prefix?:number; deps: string[] };
type Options = {
  definition: (fx: any) => any;
  /** Frame planning evaluates every channel at its provider timestamp. Without
   * time, edit/export topology checks remain conservative and time-independent. */
  time?:number;
  value?:(layer:any,fx:any,key:string,time:number)=>unknown;
  assets?: {get(id:string):any;has?(id:string):boolean};
  enabled?: (layer: any, fx: any, time?:number) => boolean;
  active?: (layer: any, time?:number) => boolean;
  contentActive?: (layer: any, time?:number) => boolean;
  roots?: string[];
  allowMissing?: boolean;
  /** Edits check topology while retaining unsupported/hydrated data. */
  topologyOnly?: boolean;
};

/** Iterative phase DAG: source -> masks -> effects; group sources consume child
 * outputs. Requests are ordered before their consumers, without JS recursion. */
export function layerInputPlan(project: any, options: Options) {
  const layers: any[] = project.layers || [], byId = new Map(layers.map(l => [l.id, l]));
  const nodes = new Map<string, Node>(), requests = new Set<string>();
  const enabled = options.enabled ?? ((_l, fx) => effectMightRender(fx));
  const active = options.active ?? (() => true);
  const children = new Map<string, any[]>();
  for (const layer of layers) if (layer.group) {
    const list = children.get(layer.group) || []; list.push(layer); children.set(layer.group, list);
  }
  const available=(layer:any,time?:number)=>time===undefined || time>=0 && time<=project.dur+1e-6 && (layer.type==='group'||time>=layer.from-1e-6&&(time<layer.from+layer.dur-1e-6||Math.abs(time-project.dur)<=1e-6&&Math.abs(time-layer.from-layer.dur)<=1e-6));
  const add=(layer:any,stage:EffectInputStage,space:EffectInputSpace='composition',time=options.time,prefix?:number):string=>{
   const key=inputKey(layer.id,stage,space,time,prefix);
   if(!nodes.has(key)){
    if(options.time!==undefined&&nodes.size>=65536)throw new EffectInputError('Temporal dependency graph exceeds the node limit');
    nodes.set(key,{key,layer,stage,space,time,prefix,deps:[]});
   }
   return key;
  };
  const request=(layer:any,stage:EffectInputStage,space:EffectInputSpace,time?:number,prefix?:number)=>{
   const key=add(layer,stage,space,time,prefix);requests.add(key);
   if(options.time!==undefined&&requests.size>MAX_FRAME_INPUTS)throw new EffectInputError('Temporal inputs exceed the frame request limit');
   return key;
  };
  const expand = (node: Node) => {
    const {layer,time,space,prefix}=node;
    if(!available(layer,time))return;
    if(node.stage==='source'){
     if(space==='composition'&&layer.type==='group')for(const child of children.get(layer.id)||[])if((options.contentActive??active)(child,time))node.deps.push(add(child,'effects',space,time));
     return;
    }
    if(node.stage==='masks'){node.deps.push(add(layer,'source',space,time));return;}
    node.deps.push(add(layer,'masks',space,time));
    if(prefix===undefined&&space==='composition'&&layer.matteSource&&byId.has(layer.matteSource))node.deps.push(request(byId.get(layer.matteSource),'effects','composition',time));
    if(!options.topologyOnly&&node.layer.effectSpace!==undefined&&!['composition','source'].includes(node.layer.effectSpace))throw new EffectInputError(`${node.layer.name}: unsupported effect processing space`);
    if(!options.topologyOnly&&node.layer.effectSpace==='source'&&(node.layer.threeD||['group','adjustment','audio','null'].includes(node.layer.type)))throw new EffectInputError(`${node.layer.name}: source owner processing requires a 2D content layer`);
    for (const [index,fx] of (node.layer.fx || []).entries()) {
      if(prefix!==undefined&&index>=prefix)break;
      if (!enabled(node.layer, fx,time) || fx.missing) continue;
      const def = options.definition(fx);
      if (!def) continue;
      if(!options.topologyOnly&&prefix!==undefined&&def.backdrop)throw new EffectInputError(`${node.layer.name}: temporal prefixes requiring the surrounding backdrop are not supported yet`);
      if(!options.topologyOnly&&node.layer.effectSpace==='source'&&(!def.sourceProcessing||def.backdrop))throw new EffectInputError(`${node.layer.name}: ${def.label} does not declare source processing support`);
      if(def.temporal&&!options.topologyOnly){
       if((node.layer.fx??[]).slice(0,index).some((f:any)=>enabled(node.layer,f,time)&&options.definition(f)?.backdrop))throw new EffectInputError(`${node.layer.name}: temporal prefixes requiring the surrounding backdrop are not supported yet`);
       if(node.layer.threeD||['group','adjustment','audio','null'].includes(node.layer.type))throw new EffectInputError(`${node.layer.name}: temporal sampling requires a 2D content owner`);
       if(time!==undefined){
        if(!options.value)throw new EffectInputError('Temporal planning requires evaluated parameters');
        const times=temporalTimes(def,key=>options.value!(node.layer,fx,key,time),time);
        for(const at of times)node.deps.push(request(node.layer,'effects',node.layer.effectSpace==='source'?'source':space,at,index));
       }
      }
      if (def.backdrop) {
        const at = layers.indexOf(node.layer);
        for (let i = at + 1; i < layers.length; i++) if ((layers[i].group || null) === (node.layer.group || null) && active(layers[i],time)) node.deps.push(add(layers[i],'effects','composition',time));
      }
      for (const param of def.params || []) {
        if(!referenceParam(param))continue;
        if(param.type==='path'||param.type==='asset'){
          const ref=validateEffectInputs({[param.k]:fx.inputs?.[param.k]??emptyReference(param.type)})[param.k]!;
          if(ref.kind!==param.type)throw new EffectInputError(`Invalid reference type for ${param.label}`);
          if(ref.kind==='path'&&ref.layerId&&!referencedPath(byId.get(ref.layerId),ref.pathId!)) {if(!options.allowMissing)throw new EffectInputError(`${node.layer.name}: missing path reference`);}
          if(ref.kind==='asset'&&ref.assetId&&!options.topologyOnly){const asset=options.assets?.has?.(ref.assetId)===false?undefined:options.assets?.get(ref.assetId)??project.assets?.[ref.assetId];if(!asset&&!options.allowMissing)throw new EffectInputError(`${node.layer.name}: missing asset reference`);if(asset&&asset.kind!==(param.assetKind??'image'))throw new EffectInputError(`${node.layer.name}: effect asset inputs require ${param.assetKind??'image'}`);}
          continue;
        }

        const value = fx.inputs?.[param.k] ?? { layerId: null, stage: 'source' };
        const ref = validateLayerInputs({ [param.k]: value })[param.k]!;
        if (ref.layerId === null) continue;
        const target = byId.get(ref.layerId);
        if (!target) {
          if (!options.allowMissing) throw new EffectInputError(`${node.layer.name}: missing input layer “${ref.layerId}”`);
          continue;
        }
        if (!options.topologyOnly && (target.threeD || ['audio','null','adjustment'].includes(target.type)))
          throw new EffectInputError(`${node.layer.name}: ${target.name} (${target.type}${target.threeD ? ', 3D' : ''}) cannot be used as a layer input yet`);
        if (!options.topologyOnly && ref.space === 'source' && (target.type === 'group' || ref.stage === 'effects' && target.matteSource))
          throw new EffectInputError(`${target.name}: source-space group inputs and effects with external track mattes are not supported yet`);
        if (!options.topologyOnly && target.type === 'group') {
          const pending = [...children.get(target.id) || []], seen = new Set<string>();
          while (pending.length) {
            const child = pending.pop()!; if (seen.has(child.id)) continue; seen.add(child.id);
            if (child.threeD) throw new EffectInputError(`${target.name}: groups containing 3D sources cannot be used as inputs yet`);
            if (child.type === 'group') pending.push(...children.get(child.id) || []);
          }
        }
        if (!options.topologyOnly && ref.stage === 'effects' && (target.fx || []).some((f: any) => enabled(target, f,time) && options.definition(f)?.backdrop))
          throw new EffectInputError(`${target.name}: effects-stage inputs requiring the surrounding backdrop are not supported yet`);
        const key = request(target,ref.stage,ref.space??'composition',time); node.deps.push(key);
      }
    }
  };
  const roots=options.roots?.flatMap(key=>{const[id,stage,space='composition',time=options.time,prefix]=JSON.parse(key);return byId.has(id)?[add(byId.get(id),stage,space,time,prefix??undefined)]:[];});
  const state = new Map<string, number>(), order: Node[] = [];
  for (const root of roots ?? layers.filter(l=>active(l,options.time)).map(l => add(l,'effects'))) {
    if (state.get(root) === 2 || !nodes.has(root)) continue;
    const stack: Array<{ node: Node; at: number }> = [];
    const enter = (key: string) => { const node = nodes.get(key)!; expand(node); state.set(key, 1); stack.push({ node, at: 0 }); };
    enter(root);
    while (stack.length) {
      const current = stack[stack.length - 1]!;
      if (current.at === current.node.deps.length) { state.set(current.node.key, 2); order.push(current.node); stack.pop(); continue; }
      const dependency = current.node.deps[current.at++]!;
      if (state.get(dependency) === 1) {
        const start = stack.findIndex(s => s.node.key === dependency);
        throw new EffectInputError('Layer input cycle: ' + [...stack.slice(start).map(s => `${s.node.layer.name || s.node.layer.id} (${s.node.stage})${s.node.time===undefined?'':` @ ${s.node.time}${s.node.prefix===undefined?'':` before effect ${s.node.prefix+1}`}`}`), `${nodes.get(dependency)!.layer.name || nodes.get(dependency)!.layer.id} (${nodes.get(dependency)!.stage})`].join(' → '));
      }
      if (state.get(dependency) !== 2 && nodes.has(dependency)) enter(dependency);
    }
  }
  return { order, requests: order.filter(n => requests.has(n.key)), layerIds: new Set(order.map(n => n.layer.id)) };
}

export function assertInputBudget(count: number, frameBytes: number, budget: number): void {
  // Six working surfaces reserve the ordinary stack, masks and effect ping-pong.
  if ((count + 6) * frameBytes > budget) throw new EffectInputError('Layer inputs exceed the framebuffer memory budget. Reduce output size or references.');
}
