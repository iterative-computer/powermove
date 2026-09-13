import { is3DLayer, world3D, local3D, perspectiveAmount, CHANNELS_3D } from './space-3d';
import type { PMRegistry } from '../registry';
import { preserveWorldPose, worldPoseValues } from './parenting';

/** Membership supplies a transform space without changing parent links or layer time. */
export function groupAncestors(layer: any, layers: any[], byId?: Map<string, any>): any[] {
  if (!layer?.group) return [];
  const result: any[] = [], seen = new Set([layer.id]);
  let id = layer.group;
  while (id && !seen.has(id)) {
    seen.add(id);
    const group = byId ? byId.get(id) : layers.find(item => item.id === id && item.type === 'group');
    if (!group || group.type !== 'group') break;
    result.push(group); id = group.group;
  }
  return result;
}

export function installLayerGroups(PM: PMRegistry): void {
  let indexes = new WeakMap<any[], { version: number; length: number; byId: Map<string, any>; ancestors: Map<any, any[]>; spans: Map<string, any> }>();
  const indexFor = (layers: any[]) => {
    const version = PM.animVersion?.() ?? -1;
    let index = indexes.get(layers);
    if (!index || index.version !== version || index.length !== layers.length || version === -1) {
      index = { version, length: layers.length, byId: new Map(layers.map(layer => [layer.id, layer])), ancestors: new Map(), spans: new Map() };
      indexes.set(layers, index);
    }
    return index;
  };
  const ancestors = (layer: any, layers: any[]): any[] => {
    if (!layer?.group) return [];
    const index = indexFor(layers);
    let result = index.ancestors.get(layer);
    if (!result) { result = groupAncestors(layer, layers, index.byId); index.ancestors.set(layer, result); }
    return result;
  };
  PM.bus?.on?.('layers', () => { indexes = new WeakMap(); });
  PM.bus?.on?.('project', () => { indexes = new WeakMap(); });
  const worldPose = (layer: any, time: number): number[] => is3DLayer(PM,layer) ? world3D(PM,layer,time) : PM.worldMatrix(layer,time);
  PM.groupBounds = (group: any, time: number) => {
    if (!PM.worldMatrix) return null;
    const m = PM.worldMatrix(group, time), det = m[0]*m[3]-m[1]*m[2];
    if (Math.abs(det) < 1e-10) return null;
    const inv = [m[3]/det,-m[1]/det,-m[2]/det,m[0]/det,(m[2]*m[5]-m[3]*m[4])/det,(m[1]*m[4]-m[0]*m[5])/det];
    const points: {x:number;y:number}[] = [];
    for (const child of PM.curComp().layers) {
      if (child.type === 'group' || !ancestors(child, PM.curComp().layers).some(item => item.id === group.id) || !PM.active(child,time)) continue;
      const bounds = PM.GL?.bounds?.(child,time);
      const world = PM.worldMatrix(child,time), local = PM.mul(inv,world);
      if (bounds) for (const x of [bounds.x0,bounds.x1]) for (const y of [bounds.y0,bounds.y1]) points.push({x:local[0]*x+local[2]*y+local[4],y:local[1]*x+local[3]*y+local[5]});
      else if (child.type !== 'audio') points.push({x:local[4],y:local[5]});
    }
    if (!points.length) return null;
    const x0=Math.min(...points.map(p=>p.x)), x1=Math.max(...points.map(p=>p.x));
    const y0=Math.min(...points.map(p=>p.y)), y1=Math.max(...points.map(p=>p.y));
    return {x0,y0,x1,y1,w:x1-x0,h:y1-y0,ax:0,ay:0};
  };
  PM.groupAncestors = (layer: any, layers = PM.curComp().layers) => ancestors(layer, layers);
  PM.transformRoots = (ids: string[]) => {
    const layers = PM.curComp().layers;
    const selected = layers.filter((layer: any) => ids.includes(layer.id) && layer.type !== 'audio' && !layer.lock && !ancestors(layer,layers).some(group => group.lock));
    const selectedIds = new Set(selected.map((layer: any) => layer.id));
    return selected.filter((layer: any) => {
      const seen = new Set<string>();
      for (let current = layer; current && !seen.has(current.id); current = layers.find((item: any) => item.id === current.parent)) {
        seen.add(current.id);
        if (current !== layer && selectedIds.has(current.id) || ancestors(current,layers).some(group => selectedIds.has(group.id))) return false;
      }
      return true;
    });
  };
  PM.groupSpan = (group: any) => {
    const index = indexFor(PM.proj.layers);
    if (index.spans.has(group.id)) return index.spans.get(group.id);
    const ids = new Set(PM.expandGroups([group.id]));
    const children = PM.proj.layers.filter((layer: any) => ids.has(layer.id) && layer.type !== 'group');
    const from = children.length ? Math.min(...children.map((layer: any) => layer.from)) : 0;
    const end = children.length ? Math.max(...children.map((layer: any) => layer.from + layer.dur)) : PM.proj.dur;
    const span = { from, dur: end - from }; index.spans.set(group.id, span); return span;
  };
  PM.expandGroups = (ids: string[]) => PM.proj.layers.filter((layer: any) => ids.includes(layer.id)
    || ancestors(layer, PM.proj.layers).some(group => ids.includes(group.id))).map((layer: any) => layer.id);
  PM.normalizeGroupStack = () => {
    const layers = PM.proj.layers, ordered: any[] = [], seen = new Set<string>();
    const visit = (parent: string | null) => {
      for (const layer of layers) {
        if ((layer.group || null) !== parent || seen.has(layer.id)) continue;
        seen.add(layer.id); ordered.push(layer);
        if (layer.type === 'group') visit(layer.id);
      }
    };
    visit(null);
    for (const layer of layers) if (!seen.has(layer.id)) { layer.group = null; ordered.push(layer); }
    PM.proj.layers = ordered;
  };
  const rebase = (layers: any[], poses: Map<string, number[]>) => {
    const done = new Set<string>();
    const visit = (layer: any) => {
      if (done.has(layer.id)) return;
      done.add(layer.id);
      const parent = layers.find(item => item.id === layer.parent);
      if (parent) visit(parent);
      if (layer.type !== 'audio') preserveWorldPose(PM,layer,poses.get(layer.id)!,PM.time);
      PM.touch();
    };
    layers.forEach(visit);
  };
  const changed = () => { PM.normalizeGroupStack(); PM.ProjectIndex?.invalidate(); PM.bus.emit('layers'); PM.invalidate(); };
  PM.groupLayers = (ids: string[], name = 'Group') => {
    const layers = PM.proj.layers;
    const selected = layers.filter((layer: any) => ids.includes(layer.id));
    if (!selected.length) throw new Error('Select layers to group');
    if (new Set(ids).size !== selected.length) throw new Error('Layer not found');
    if (selected.some((layer: any) => layer.lock || ancestors(layer, layers).some(group => group.lock))) throw new Error('Unlock layers before grouping');
    const roots = selected.filter((layer: any) => !ancestors(layer, layers).some(group => ids.includes(group.id)));
    const parent = roots[0].group || null;
    if (roots.some((layer: any) => (layer.group || null) !== parent)) throw new Error('Select layers in the same group');
    const group = PM.mkLayer('group', { name: String(name || 'Group').trim() || 'Group' });
    group.group = parent; group.collapsed = false;
    group.from = 0; group.dur = PM.proj.dur;
    // Keep selected layers in stack order and gather them at the first selected row.
    layers.splice(layers.indexOf(roots[0]), 0, group);
    roots.forEach((layer: any) => { layer.group = group.id; });
    changed();
    const bounds = PM.groupBounds(group, PM.time);
    if (bounds) {
      group.p['position.x'].v = group.p['anchor.x'].v = (bounds.x0 + bounds.x1) / 2;
      group.p['position.y'].v = group.p['anchor.y'].v = (bounds.y0 + bounds.y1) / 2;
    }
    PM.touch?.(); PM.selectLayers(group.id); return group;
  };
  PM.ungroupLayers = (ids: string[]) => {
    const groups = PM.proj.layers.filter((layer: any) => ids.includes(layer.id) && layer.type === 'group');
    if (!groups.length) throw new Error('Select a group to ungroup');
    if (groups.some((group: any) => group.lock || ancestors(group, PM.proj.layers).some(parent => parent.lock))) throw new Error('Unlock the group first');
    if (PM.proj.layers.some((layer: any) => layer.lock && ancestors(layer,PM.proj.layers).some(group => ids.includes(group.id)))) throw new Error('Unlock the group contents before ungrouping');
    const styled = groups.find((group: any) => group.fx?.length || group.masks?.length
      || group.matteSource || group.transitionIn || group.transitionOut
      || typeof group.blend === 'object' || group.blend && group.blend !== 'normal'
      || typeof group.mblur === 'object' || group.mblur === true);
    if (styled) throw new Error(`Remove visual compositing from “${styled.name}” before ungrouping`);
    const children: string[] = [];
    for (const group of groups) {
      const members = PM.proj.layers.filter((layer: any) => layer.group === group.id);
      // Ungrouping animation bakes editable channel keys at composition frames.
      // Include the playhead so this operation never jumps between frames either.
      const hasAnimation = (layer: any) => Object.values(layer.p).some((prop: any) => prop.kf.length || prop.expr);
      const changes3D = is3DLayer(PM,group) && local3D(PM,group,PM.time).some((value,index)=>Math.abs(value-[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1][index]!)>1e-8);
      const changesPose = changes3D || PM.localMatrix(group,PM.time).some((value: number,index: number) => Math.abs(value-[1,0,0,1,0,0][index]!)>1e-8) || PM.ev(group,'opacity',PM.time)!==100;
      const animated = hasAnimation(group) || changesPose && PM.proj.layers.some(hasAnimation);
      const times = animated ? [...new Set([PM.time, ...Array.from({length:Math.ceil(PM.proj.dur*PM.proj.fps)+1}, (_,i)=>i/PM.proj.fps)])].sort((a,b)=>a-b) : [];
      const samples = times.map(time => ({time, values:new Map<string,{world:number[];opacity:number;perspective:number}>(members.filter((layer:any)=>layer.type!=='audio').map((layer:any)=>[layer.id,{world:worldPose(layer,time),perspective:perspectiveAmount(PM,layer,time),opacity:PM.ev(layer,'opacity',time)*PM.ev(group,'opacity',time)/100}]))}));
      const poses = new Map<string,number[]>(members.filter((layer: any) => layer.type !== 'audio').map((layer: any) => [layer.id, worldPose(layer,PM.time)]));
      const perspectives = new Map(members.map((layer: any) => [layer.id, perspectiveAmount(PM,layer,PM.time)]));
      for (const layer of members) {
        layer.group = group.group || null; children.push(layer.id);
        if (poses.get(layer.id)?.length === 16) {
          for (const [key,fallback] of Object.entries(CHANNELS_3D)) layer.p[key] ||= PM.P(fallback);
          layer.threeD = true;
          layer.p.perspective = PM.P(perspectives.get(layer.id));
        }
        const opacity = layer.p.opacity, factor = PM.ev(group,'opacity',PM.time) / 100;
        if (opacity && factor !== 1) {
          if (opacity.expr) opacity.expr = `(${opacity.expr}) * ${factor}`;
          else { opacity.v *= factor; for (const key of opacity.kf) { key.v *= factor; if (key.inEase) key.inEase.speed *= factor; if (key.outEase) key.outEase.speed *= factor; } }
        }
      }
      PM.touch();
      if (animated) {
        const ordered: any[] = [], seen = new Set<string>();
        const visit = (layer:any) => {
          if (seen.has(layer.id)) return;
          seen.add(layer.id);
          for (let parent=PM.L(layer.parent), guard=0; parent && guard++<256; parent=PM.L(parent.parent)) {
            if (members.includes(parent)) visit(parent);
            for (const ancestor of ancestors(parent,PM.proj.layers)) if (members.includes(ancestor)) visit(ancestor);
          }
          if (layer.type!=='audio') ordered.push(layer);
        };
        members.forEach(visit);
        const channelsFor=(layer:any)=>['position.x','position.y','scale.x','scale.y','rotation','skew','opacity', ...(poses.get(layer.id)?.length===16 ? ['perspective','position.z','scale.z','rotation.x','rotation.y','orientation.x','orientation.y','orientation.z'] : [])];
        for (const layer of ordered) for (const key of channelsFor(layer)) layer.p[key]={v:layer.p[key].v,kf:[],expr:null};
        for (const sample of samples) for (const layer of ordered) {
          const source=sample.values.get(layer.id)!;
          const values: Record<string,number>={...worldPoseValues(PM,layer,source.world,sample.time,undefined,true),opacity:source.opacity,perspective:source.perspective};
          for (const key of channelsFor(layer)) {
            const prop=layer.p[key]; let value=values[key]!;
            if ((key==='rotation' || key.startsWith('rotation.')) && prop.kf.length) value += Math.round((prop.kf[prop.kf.length-1].v-value)/360)*360;
            prop.kf.push(PM.KF(sample.time-layer.from,value,'linear')); prop.v=value;
          }
          PM.touch();
        }
      } else rebase(members, poses);
      PM.proj.layers = PM.proj.layers.filter((layer: any) => layer.id !== group.id);
    }
    changed(); PM.selectLayers(children.filter(id => PM.L(id))); return { ids: children };
  };
  PM.moveToGroup = (ids: string[], target: string | null) => {
    const group = target ? PM.L(target) : null;
    if (target && (!group || group.type !== 'group')) throw new Error('Group not found');
    const selected = PM.proj.layers.filter((layer: any) => ids.includes(layer.id));
    if (!selected.length || new Set(ids).size !== selected.length) throw new Error('Layer not found');
    if (group?.lock || group && ancestors(group, PM.proj.layers).some(parent => parent.lock) || selected.some((layer: any) => layer.lock || ancestors(layer, PM.proj.layers).some(parent => parent.lock))) throw new Error('Unlock layers before moving');
    if (group && (ids.includes(group.id) || ancestors(group, PM.proj.layers).some(item => ids.includes(item.id)))) throw new Error('Grouping would create a cycle');
    const roots = selected.filter((layer: any) => !ancestors(layer, PM.proj.layers).some(item => ids.includes(item.id)));
    const poses = new Map<string,number[]>(roots.filter((layer: any) => layer.type !== 'audio').map((layer: any) => [layer.id, worldPose(layer,PM.time)]));
    const perspectives = new Map(roots.map((layer: any) => [layer.id, perspectiveAmount(PM,layer,PM.time)]));
    roots.forEach((layer: any) => { layer.group = group?.id || null; });
    PM.touch(); rebase(roots, poses);
    for (const layer of roots) if (layer.threeD && !ancestors(layer,PM.proj.layers).some(group => group.threeD)) layer.p.perspective = PM.P(perspectives.get(layer.id));
    changed(); return { ids: selected.map((layer: any) => layer.id), group: target };
  };
}
