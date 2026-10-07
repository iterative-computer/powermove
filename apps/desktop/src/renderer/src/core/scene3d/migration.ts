import { parseScene,sceneProperties,SCENE3D_DEFINITION } from './schema';
import { sceneForLayer } from './service';
import { sceneCommands } from './operations';
import { layer3DRole,initializeLayer3D } from './layers';
import type { EditResult } from '../types/commands';

/** Explicit, undoable upgrade. Existing documents otherwise keep their original renderer. */
function legacySceneSource(PM:any,layerId:string):any {
  try {
    const layer=PM.L(layerId),definition=layer&&PM.layerDefinition?.(layer.d?.definition);
    if(definition?.renderer?.kind!=='mesh')throw new Error('Choose an imported legacy 3D model layer');
    if(layer.lock||(PM.groupAncestors?.(layer)||[]).some((g:any)=>g.lock))throw new Error('Unlock this model layer before converting it');
    const original=layer.d.params || {},background=original.background;
    if(background?.expr||background?.kf?.length)throw new Error('Move the animated background to a separate layer before converting this model');
    const raw=sceneForLayer(PM,layer,PM.time);if(!raw)throw new Error('The model asset is missing');
    const scene=parseScene(raw),object=scene.objects[0]!,light=scene.lights[0]!;
    const clone=(value:any)=>JSON.parse(JSON.stringify(value));
    const animated=(prop:any)=>!!(prop?.expr||prop?.kf?.length);
    for(const [old,key] of Object.entries({rotationX:'rx',rotationY:'ry',rotationZ:'rz',size:'sx'}))
      if(original[old])object.p[key]=clone(original[old]);
    object.p.sy=clone(object.p.sx);object.p.sz=clone(object.p.sx);
    for(const [old,key] of Object.entries({objectColor:'color',roughness:'roughness',metalness:'metalness'}))
      if(original[old])object.material.p[key]=clone(original[old]);
    if(original.lightColor)light.p.color=clone(original.lightColor);
    const fps=Math.max(1,PM.proj.fps || 30),frames=Math.ceil(layer.dur*fps);
    const expressions=sceneProperties({d:{data:{scene}}}).filter(({prop}:any)=>prop.expr);
    const bakeCamera=['yaw','pitch','distance'].some(k=>animated(original[k]));
    const bakeRotation=original.autoRotate?.v || animated(original.autoRotate);
    if((bakeCamera||bakeRotation||expressions.length)&&frames>2398)throw new Error('Animated legacy conversion supports up to 2,398 frames; split or trim this model first');
    const cameraKeys=['x','y','z'] as const;
    if(bakeCamera)for(const key of cameraKeys)scene.camera.p[key]!.kf=[];
    if(bakeRotation)object.p.ry!.kf=[];
    for(const {prop} of expressions){prop.kf=[];prop.expr=null;}
    if(bakeCamera||bakeRotation||expressions.length)for(let frame=0;frame<=frames;frame++){
      const t=Math.min(layer.dur,frame/fps),sample=sceneForLayer(PM,layer,layer.from+t)!;
      const evaluated=new Map(sceneProperties({d:{data:{scene:sample}}}).map((p:any)=>[p.key,p.prop.v]));
      for(const {key,prop} of expressions)if(key!=='o.model.ry'||!bakeRotation)prop.kf.push({t,v:evaluated.get(key)});
      if(bakeCamera)for(const key of cameraKeys)scene.camera.p[key]!.kf.push({t,v:sample.camera.p[key]!.v});
      if(bakeRotation)object.p.ry!.kf.push({t,v:sample.objects[0]!.p.ry!.v});
    }
    if(bakeRotation)object.p.ry!.expr=null;
    for(const {prop} of sceneProperties({d:{data:{scene}}}))for(const key of prop.kf)key.i=PM.uid?.('kf') || crypto.randomUUID();
    const checked=parseScene(scene);
    return checked;
  }catch(error){throw error;}
}

/** Split a legacy container into ordinary layers in one reversible transaction. */
export function convertLegacyScene(PM:any,layerId:string):EditResult {
  const layer=PM.L?.(layerId);
  if(!layer)return {ok:false,message:'3D layer not found'};
  if(layer3DRole(layer))return {ok:true,message:'Already a 3D layer',data:{result:{layerId,id:layerId}}};
  if(layer.lock || (PM.groupAncestors?.(layer)||[]).some((g:any)=>g.lock))return {ok:false,message:'Unlock this model layer before converting it'};
  try {
    const scene=layer.d.definition===SCENE3D_DEFINITION?parseScene(layer.d.data.scene):legacySceneSource(PM,layerId);
    const plan=sceneCommands(PM,{operation:'create',scene});
    const first=plan.commands.find((c:any)=>c.type==='add_layer'&&c.content?.definition==='powermove.3d.object') as any,madeId=first?.id;
    if(first){first.id=layerId;first.name=layer.name;}
    const created=(plan.commands as any[]).filter(c=>c.type==='add_layer').map(c=>{
      const next=PM.mkLayer('extension',{name:c.name});
      next.id=c.id;next.from=layer.from;next.dur=layer.dur;
      next.d={definition:c.content.definition,version:1,w:layer.d.w,h:layer.d.h,params:{},data:c.content.data};
      initializeLayer3D(next,c.properties);next.parent=c.parent===madeId?layerId:c.parent || null;
      if(c===first){next.fx=layer.fx;next.masks=layer.masks;next.blend=layer.blend;next.on=layer.on;next.group=layer.group;next.mblur=layer.mblur;}
      for(const prop of [...Object.values<any>(next.p),...sceneProperties(next).map(p=>p.prop)])for(const key of prop.kf || [])key.i=PM.uid('kf');
      return next;
    });
    return PM.Edit.mutate('Convert to 3D layers',()=>{
      const index=PM.proj.layers.indexOf(layer);PM.proj.layers.splice(index,1,...created.reverse());
      PM.selectLayers?.([first?layerId:plan.id]);PM.touch();return {id:first?layerId:plan.id,layerId:first?layerId:plan.id};
    },{origin:'interface'});
  }catch(error){return {ok:false,message:error instanceof Error?error.message:String(error)};}
}
