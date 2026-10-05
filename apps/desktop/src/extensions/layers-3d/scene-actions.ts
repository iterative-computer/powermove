import type { EditResult, PowermoveAPI, MenuContribution } from 'powermove';
import { PRIMITIVES, LIGHT_TYPES } from './scene-model';
import type { Primitive, LightType } from './scene-types';
export const MODEL_DEFINITION_ID='powermove.3d.object';
export const MODEL_ACCEPT='.blend,.obj,.mtl,.glb,.gltf,.png,.jpg,.jpeg,.webp';
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
  const name=`${LIGHT_TYPES.find(p=>p.id===type)?.label || 'Point'} light`;
  const result=run(api,{operation:'add_light',light:{type,name}},`Add ${name}`);
  return result.ok?(result.data.result as any)?.id ?? null:null;
}
export function addCamera(api:PowermoveAPI):EditResult{return run(api,{operation:'add_camera'},'Add camera');}
export async function importModelIntoScene(api:PowermoveAPI,supplied?:unknown,_layerId?:unknown):Promise<EditResult>{
  try{
    const files=supplied instanceof File?[supplied]:Array.isArray(supplied)&&supplied.every(f=>f instanceof File)?supplied:await api.assets.pick({accept:MODEL_ACCEPT,multiple:true});
    if(!files.length)return {ok:false,message:'Import cancelled'};
    const source=files.find(f=>/\.(blend|obj|glb|gltf)$/i.test(f.name));if(!source)throw new Error('Choose a Blender, OBJ, GLB or glTF model');
    if(/\.blend$/i.test(source.name)){const result=await api.scene3d.model({operation:'import_blend',file:source,name:source.name.replace(/\.blend$/i,'')});if(!result.ok)throw new Error(result.message);return result;}
    const file=await api.scene3d.prepareImport(files),asset=await api.assets.import(file,{layerDefinition:MODEL_DEFINITION_ID});
    return run(api,{operation:'add_object',object:{source:{assetId:asset.id},name:source.name.replace(/\.(obj|glb|gltf)$/i,''),useSourceMaterials:!/\.obj$/i.test(file.name)}},`Import ${source.name}`);
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
export const addMenuItems=(api:PowermoveAPI,_layerId?:unknown):MenuContribution[]=>[
  ...api.scene3d.modelRecipes.map(recipe=>({label:recipe.label,icon:'cube',run:()=>api.commands.run(`3d.model.${recipe.id}`)})), '-',
  ...PRIMITIVES.map(p=>({label:p.label,icon:'cube',run:()=>addPrimitive(api,p.id)})),
  {label:'Import 3D model…',icon:'download',run:()=>importModelIntoScene(api)},'-',
  ...LIGHT_TYPES.map(l=>({label:`${l.label} light`,icon:'sun',run:()=>addLight(api,l.id)})),
  {label:'Camera',icon:'camera',run:()=>addCamera(api)}
];
