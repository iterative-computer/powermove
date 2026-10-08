import {layer3DRole} from './layers';
import {unitChannelConverters,type ChannelConverter} from './space';

/*
 * Projects saved before 3D layers shared the composition's pixel space kept
 * models, lights, cameras and their model groups in scene units. They are
 * converted once on load; `project.space3d === 'px'` marks a converted (or new)
 * project.
 */

export const PX_SPACE='px';
export const needsSpaceMigration=(project:any)=>!!project&&project.space3d!==PX_SPACE;

function convertProp(prop:any,convert:ChannelConverter):void {
  if(!prop||typeof prop!=='object'||typeof prop.v!=='number')return;
  prop.v=convert(prop.v);
  for(const key of Array.isArray(prop.kf)?prop.kf:[])if(typeof key?.v==='number')key.v=convert(key.v);
}

/** Convert every unit-space 3D layer (and model group) in one composition's layer list. */
export function migrateLayers3DSpace(layers:any[],comp:any):number {
  const byId=new Map(layers.map(layer=>[layer.id,layer]));
  const members=(group:any)=>layers.filter(layer=>layer.group===group.id);
  const legacyGroup=(group:any):boolean=>!!group&&group.type==='group'&&(!!group.d?.modeling||(!!group.threeD&&members(group).length>0&&members(group).every(child=>layer3DRole(child)||legacyGroup(child))));
  const unit=new Set(layers.filter(layer=>layer3DRole(layer)||legacyGroup(layer)).map(layer=>layer.id));
  if(!unit.size)return 0;
  for(const id of unit){
    const layer=byId.get(id)!;
    // Positions inside a converted parent or group stay relative to it.
    const relative=(layer.parent&&unit.has(layer.parent))||(layer.group&&unit.has(layer.group));
    const converters=unitChannelConverters(comp,!!relative);
    for(const [path,prop] of Object.entries<any>(layer.p||{}))if(converters[path])convertProp(prop,converters[path]!);
    const content=layer.d?.data?.light?.p||layer.d?.data?.camera?.p;
    for(const [key,prop] of Object.entries<any>(content||{}))if(converters[key])convertProp(prop,converters[key]!);
  }
  return unit.size;
}
