import { canAnimateContent, evaluatedValue, isProperty } from 'powermove';

/** A selected group owns its selected members for shared inspector edits. */
export function inspectorSelection(PM: any, layers: any[]): any[] {
  const ids = new Set(layers.map(layer => layer.id));
  return layers.filter(layer => !(PM.groupAncestors?.(layer) || []).some((group: any) => ids.has(group.id)));
}

export function translatePath(primary: any, target: any, path: string): string | null {
  if (path.startsWith('c.')) return canAnimateContent(target, path.slice(2)) ? path : null;
  if (path.startsWith('l.')) return target.type === 'audio' && path !== 'l.on' ? null : path;
  if (target.p?.[path]) return path;
  if (path.startsWith('u.') || path.startsWith('x.')) return primary.type === target.type && (primary.type !== 'extension' || primary.d.definition === target.d.definition) ? path : null;
  if (/^(g|mp|ta|ts)\./.test(path)) return primary.id===target.id?path:null;
  if (path.startsWith('m.')) {
    const [,id,...rest] = path.split('.'), i = primary.masks.findIndex((m: any) => m.id === id);
    return target.masks?.[i] ? `m.${target.masks[i].id}.${rest.join('.')}` : null;
  }
  const [id,...rest] = path.split('.'), i = primary.fx.findIndex((f: any) => f.id === id);
  return i >= 0 && target.fx?.[i]?.type === primary.fx[i].type ? `${target.fx[i].id}.${rest.join('.')}` : null;
}

function currentValue(PM: any, layer: any, command: any) {
  if(command.type==='set_content'){const field=Object.keys(command.patch||{})[0];return field?evaluatedValue(PM,layer,layer.d[field],PM.time,'c.'+field):undefined;}
  if(command.type==='set_layer'){const field=Object.keys(command.patch||{})[0];const key=({visible:'on',motionBlur:'mblur'} as any)[field!]||field;return key?evaluatedValue(PM,layer,layer[key],PM.time,'l.'+key):undefined;}
  const path = command.path;
  if (command.type === 'set_property') {
    const prop = PM.findProp?.(layer,path) || layer?.p?.[path];
    if (prop) return PM.evP(layer,prop,command.time ?? PM.time,path);
    if (path.startsWith('c.')) return evaluatedValue(PM,layer,layer.d[path.slice(2)],PM.time,path);
    if (path.startsWith('l.')) return evaluatedValue(PM,layer,layer[path.slice(2)],PM.time,path);
  }
  return undefined;
}

/** The inspector owns selection expansion; agent and canvas commands stay explicit. */
export function inspectorPM(PM: any): any {
  let snapshot: Map<string,unknown> | null = null;
  let editing = false;
  let keySnapshot = new Map<string, { base: number; keys: any[] }>();
  const expandKeys = (commands: any[]) => commands.flatMap(command => {
    if (command.type !== 'set_property' || typeof command.value !== 'number' || command.mode === 'static') return [command];
    const layer = PM.L(command.target);
    if (!layer || layer.lock) return [command];
    const prop = PM.findProp?.(layer, command.path) || layer.p?.[command.path];
    const selected = new Set(PM.sel.keys || []);
    const keys = prop?.kf?.filter((key: any) => selected.has(key.i) && typeof key.v === 'number') || [];
    if (keys.length < 2) return [command];
    const id = `${layer.id}:${command.path}`;
    if (!keySnapshot.has(id)) keySnapshot.set(id, {
      base: currentValue(PM, layer, command),
      keys: keys.map((key: any) => ({ ...key }))
    });
    const initial = keySnapshot.get(id)!;
    return initial.keys.map(key => ({ ...command, mode: 'keyframe',
      time: layer.from + key.t, value: key.v + command.value - initial.base }));
  });
  const expand = (input: any, relative = false) => {
    const selected = inspectorSelection(PM, PM.selLayers()).filter((l: any) => !l.lock);
    const first = inspectorSelection(PM, PM.selLayers())[0];
    return ([] as any[]).concat(input).flatMap(command => {
      if (!first || command.target !== first.id || PM.selLayers().length < 2) return [command];
      if (!['set_property','set_expression','replace_keyframes','set_content','set_layer'].includes(command.type)) return [command];
      return selected.flatMap((target: any) => {
        const path = command.path ? translatePath(first,target,command.path) : null;
        if (command.path && !path) return [];
        if (command.type === 'set_content' && target.type !== first.type) return [];
        const next = { ...command, target: target.id, ...(path ? {path} : {}) };
        if (relative && typeof command.value === 'number' && path) {
          const key = `${target.id}:${path}`, firstKey = `${first.id}:${command.path}`;
          if (!snapshot!.has(key)) snapshot!.set(key,currentValue(PM,target,next));
          if (!snapshot!.has(firstKey)) snapshot!.set(firstKey,currentValue(PM,first,command));
          const own = snapshot!.get(key), base = snapshot!.get(firstKey);
          if (typeof own === 'number' && typeof base === 'number') next.value = own + command.value-base;
        }
        if(relative&&command.type==='set_content'){
          next.patch={...command.patch};
          for(const [field,value] of Object.entries(command.patch))if(typeof value==='number'){
            const key=`${target.id}:c.${field}`,firstKey=`${first.id}:c.${field}`,read=(l:any)=>currentValue(PM,l,{type:'set_content',patch:{[field]:value}});
            if(!snapshot!.has(key))snapshot!.set(key,read(target));if(!snapshot!.has(firstKey))snapshot!.set(firstKey,read(first));
            const own=snapshot!.get(key),base=snapshot!.get(firstKey);if(typeof own==='number'&&typeof base==='number')next.patch[field]=own+value-base;
          }
        }
        return [next];
      });
    });
  };
  const edit = new Proxy(PM.Edit, { get(target,key) {
    if (key === 'begin') return (...args: any[]) => { editing = true; snapshot = new Map(); keySnapshot = new Map(); return target.begin(...args); };
    if (key === 'dispatch') return (commands: any) => {const next=expandKeys(expand(commands,editing));return target.dispatch(Array.isArray(commands)||next.length!==1?next:next[0]);};
    if (key === 'apply') return (commands: any,...args: any[]) => {keySnapshot = new Map(); const next=expandKeys(expand(commands));return target.apply(Array.isArray(commands)||next.length!==1?next:next[0],...args);};
    if (key === 'commit' || key === 'cancel') return (...args: any[]) => { editing = false; snapshot = null; keySnapshot = new Map(); return target[key](...args); };
    return target[key];
  }});
  return new Proxy(PM, { get(target,key) {
    if (key === 'Edit') return edit;
    if(key==='inspectorApply')return PM.Edit.apply;
    if(key==='inspectorTargets')return (layer:any,path:string)=>{const selected=inspectorSelection(PM,PM.selLayers());return (selected.some((l:any)=>l.id===layer.id)?selected:[layer]).filter((l:any)=>!l.lock).flatMap((l:any)=>{const translated=translatePath(layer,l,path);return translated?[{layer:l,path:translated,prop:PM.findProp?.(l,translated)||l.p?.[translated]}]:[];});};
    if (key === 'inspectorMixed') return (binding: any,value: any) => {
      if (!binding || binding.mode !== 'command' || typeof binding.command !== 'function') return false;
      if(PM.selLayers().filter((l:any)=>!l.lock).length<2)return false;
      const commands = expand(binding.command(value));
      const values = commands.filter(c => ['set_property','set_content','set_layer'].includes(c.type)).map(c => currentValue(PM,PM.L(c.target),c));
      return values.length>1 && values.some(v => !Object.is(v,values[0]));
    };
    return target[key];
  }});
}
