import type { EditResult, PowermoveAPI, MenuContribution } from 'powermove';
import { PRIMITIVES, LIGHT_TYPES } from './scene-model';
import type { Primitive, LightType } from './scene-types';
import { addAmbientLight, addEnvironmentLight, addFog, addScatter, addTerrain, addWorld, scatterSources, worldLayer, SKY_PRESETS } from './environment-ui';
export const MODEL_DEFINITION_ID='powermove.3d.object';
export const MODEL_ACCEPT='.obj,.mtl,.glb,.gltf,.step,.stp,.iges,.igs,.brep,.ply,.splat,.png,.jpg,.jpeg,.webp';
const MODEL_FILE=/\.(obj|glb|gltf|step|stp|iges|igs|brep|ply|splat)$/i;
export const is3DLayer=(layer:any)=>['powermove.3d.object','powermove.3d.light','powermove.3d.camera'].includes(layer?.d?.definition);
const run=(api:PowermoveAPI,args:any,label:string)=>{
  const result=api.scene3d.edit(args,{label,origin:'command'});
  if(!result.ok)api.ui.toast(result.message,{error:true});
  else{const id=(result.data.result as any)?.id;if(id)api.selection.select([id]);}
  api.transport.invalidate();return result;
};
export function addPrimitive(api:PowermoveAPI,primitive:Primitive='box',_layerId?:unknown):string|null {
  const name=PRIMITIVES.find(p=>p.id===primitive)?.label || 'Model';
  const result=run(api,{operation:'add_object',object:{source:{primitive},name}},`Add ${name}`);
  return result.ok?(result.data.result as any)?.id ?? null:null;
}
export function addLight(api:PowermoveAPI,type:LightType='point',_layerId?:unknown):string|null {
  if(type==='ambient')return addAmbientLight(api);
  if(type==='environment')return addEnvironmentLight(api);
  const name=`${LIGHT_TYPES.find(p=>p.id===type)?.label || 'Point'} light`;
  const result=run(api,{operation:'add_light',light:{type,name}},`Add ${name}`);
  return result.ok?(result.data.result as any)?.id ?? null:null;
}
export function addCamera(api:PowermoveAPI):EditResult{return run(api,{operation:'add_camera'},'Add camera');}
export async function importModelIntoScene(api:PowermoveAPI,supplied?:unknown,_layerId?:unknown):Promise<EditResult>{
  try{
    const files=supplied instanceof File?[supplied]:Array.isArray(supplied)&&supplied.every(f=>f instanceof File)?supplied:await api.assets.pick({accept:MODEL_ACCEPT,multiple:true});
    if(!files.length)return {ok:false,message:'Import cancelled'};
    const source=files.find(f=>MODEL_FILE.test(f.name));if(!source)throw new Error('Choose an OBJ, GLB, glTF, STEP, IGES or Gaussian splat (PLY, .splat) model');
    const file=await api.scene3d.prepareImport(files),asset=await api.assets.import(file,{layerDefinition:MODEL_DEFINITION_ID});
    const object={source:{assetId:asset.id},name:source.name.replace(MODEL_FILE,''),useSourceMaterials:!/\.obj$/i.test(file.name)};
    // The first model in an empty composition arrives in a finished studio:
    // resting on the floor, framed, lit and on the seamless set — one Undo.
    const empty=!api.model.curComp().layers.some((l:any)=>['powermove.3d.camera','powermove.3d.light'].includes(l.d?.definition));
    if(!empty)return run(api,{operation:'add_object',object},`Import ${source.name}`);
    const label=`Import ${source.name}`;
    api.edit.begin(label,{origin:'command'});
    try{
      const added=api.scene3d.edit({operation:'add_object',object},{origin:'command'});if(!added.ok)throw new Error(added.message);
      const id=(added.data.result as any)?.id;
      for(const args of [{operation:'drop_to_floor',target:id},{operation:'auto_studio'}]){const r=api.scene3d.edit(args as any,{origin:'command'});if(!r.ok)throw new Error(r.message);}
      const result=api.edit.commit(label);if(!result.ok)throw new Error(result.message);
      if(id)api.selection.select([id]);api.transport.invalidate();
      return {...result,data:{...result.data,result:(added.data as any)?.result}};
    }catch(error){api.edit.cancel();throw error;}
  }catch(error){const message=error instanceof Error?error.message:String(error);api.ui.toast(message,{error:true});return {ok:false,message};}
}
export function duplicateItems(api:PowermoveAPI,_layerId?:unknown,ids?:string[]):boolean{return batch(api,ids || api.selection.layers(),'duplicate','Duplicate 3D layers');}
export function removeItems(api:PowermoveAPI,_layerId?:unknown,ids?:string[]):boolean{return batch(api,ids || api.selection.layers(),'remove','Delete 3D layers');}
function batch(api:PowermoveAPI,ids:string[],operation:'remove'|'duplicate',label:string):boolean{
  const targets=ids.filter(id=>is3DLayer(api.model.layer(id)));if(!targets.length)return false;
  const copies:string[]=[];api.edit.begin(label,{origin:'command'});
  try{for(const target of targets){const result=api.scene3d.edit({operation,target},{origin:'command'});if(!result.ok)throw new Error(result.message);const id=(result.data.result as any)?.id;if(id)copies.push(id);}
    const result=api.edit.commit(label);if(!result.ok)throw new Error(result.message);api.selection.select(operation==='duplicate'?copies:[]);return true;
  }catch(error){api.edit.cancel();api.ui.toast(error instanceof Error?error.message:String(error),{error:true});return false;}
}
/** Add 3D Layer ▸: shapes and models, then whole environments (World, Terrain,
    Scatter, Fog), then lights and the camera. Plain rows: one icon repeated on
    every shape would say nothing. */
export const addMenuItems=(api:PowermoveAPI,_layerId?:unknown):MenuContribution[]=>{
  const {models,surfaces}=scatterSources(api);
  const world=worldLayer(api);
  /* Scatter asks "what" (a model) and then "on what" (terrain first). */
  const scatter:MenuContribution=models.length&&surfaces.length>1
    ?{label:'Scatter',submenu:[{header:'Copies of'},...models.map(m=>({label:m.name,submenu:[{header:'Over'},...[...surfaces].sort((a,b)=>Number(b.terrain)-Number(a.terrain)).filter(s=>s.id!==m.id).map(s=>({label:s.name,run:()=>addScatter(api,m.id,s.id)}))] as MenuContribution[]}))]}
    :{label:models.length?'Scatter (add a Terrain to cover first)':'Scatter (add a model to copy first)',disabled:true};
  return [
    ...PRIMITIVES.map(p=>({label:p.label,run:()=>addPrimitive(api,p.id)})),
    '-',
    {label:'Import 3D Model…',run:()=>importModelIntoScene(api)},'-',
    world?{label:'World',run:()=>api.selection.select([world.id])}:{label:'World',submenu:SKY_PRESETS.map(p=>({label:p.label,run:()=>addWorld(api,p.id)}))},
    {label:'Terrain',run:()=>addTerrain(api)},
    scatter,
    {label:'Fog Volume',run:()=>addFog(api)},'-',
    ...LIGHT_TYPES.map(l=>({label:`${l.label} Light`,run:()=>addLight(api,l.id)})),
    {label:'Camera',run:()=>addCamera(api)}
  ];
};
