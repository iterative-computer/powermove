import { mount, unmount } from 'svelte';
import type { ExtensionLayerDefinition, PowermoveAPI } from 'powermove';
import { OBJ_MODEL } from './obj-model';
import { STUDIO_CUBE } from './studio-cube';
import Properties3D from './Properties3D.svelte';
import {addPrimitive,addLight,addCamera,importModelIntoScene,duplicateItems,removeItems,is3DLayer,addMenuItems} from './scene-actions';
import {PRIMITIVES,LIGHT_TYPES} from './scene-model';
import {createSceneUiState} from './scene-state.svelte';

export default function activate(api:PowermoveAPI):void {
  api.layers.register(STUDIO_CUBE);api.layers.register(OBJ_MODEL);
  // Read-only compatibility registration. New work always uses individual layers.
  api.layers.register({id:'powermove.3d.scene',label:'Legacy 3D Scene',version:1,icon:'cube',params:[],defaults:{scene:api.scene3d.createScene()} as any,renderer:{kind:'scene3d'}});
  const object=api.scene3d.createObject('model'),light=api.scene3d.createLight('light'),scene=api.scene3d.createScene();
  const {id:oi,name:on,parent:op,p:ot,...objectData}=object;
  const {x:lx,y:ly,z:lz,visible:lv,...lightP}=light.p;
  const {x:cx,y:cy,z:cz,...cameraP}=scene.camera.p;
  for(const def of [
    {id:'powermove.3d.object',label:'3D Model',icon:'cube',role:'object',data:{object:objectData}},
    {id:'powermove.3d.light',label:'Light',icon:'sun',role:'light',data:{light:{type:light.type,p:lightP,castShadow:true}}},
    {id:'powermove.3d.camera',label:'Camera',icon:'camera',role:'camera',data:{camera:{projection:scene.camera.projection,p:cameraP},environment:scene.environment}}
  ])api.layers.register({id:def.id,label:def.label,icon:def.icon,version:1,params:[],defaults:def.data as any,renderer:{kind:'layer3d',role:def.role}} as ExtensionLayerDefinition);
  const convert=(id?:unknown)=>{const target=typeof id==='string'?id:api.selection.layers()[0];if(!target)return false;const result=api.scene3d.convert(target);if(!result.ok)api.ui.toast(result.message,{error:true});api.transport.invalidate();return result;};
  const createModel=async(recipe:string)=>{const preset=api.scene3d.modelRecipes.find(r=>r.id===recipe);const result=await api.scene3d.model({operation:'create_model',recipe:{kind:recipe,parameters:preset?.parameters||{},modifiers:[]}});if(!result.ok)api.ui.toast(result.message,{error:true});return result;};
  const commands=[
    ...api.scene3d.modelRecipes.map(recipe=>({id:`3d.model.${recipe.id}`,label:`Add 3D ${recipe.label}`,run:()=>createModel(recipe.id)})),
    {id:'3d.regenerate-model',label:'Regenerate 3D Model',run:async()=>{const result=await api.scene3d.model({operation:'regenerate_model',target:api.selection.layers()[0]});if(!result.ok)api.ui.toast(result.message,{error:true});return result;}},
    {id:'3d.render-frame',label:'Render 3D Frame',run:()=>api.scene3d.setPreviewMode('rendered')},
    {id:'3d.add-studio-cube',label:'Add 3D Studio Cube',run:()=>api.project.apply({type:'add_layer',layerType:'extension',name:'3D Studio Cube',content:{definition:STUDIO_CUBE.id}},{label:'Add 3D Studio Cube'})},
    {id:'3d.new-model',label:'New 3D Model',run:()=>addPrimitive(api)},
    {id:'3d.add-object',label:'Add 3D Object',run:(primitive?:unknown)=>addPrimitive(api,PRIMITIVES.find(p=>p.id===primitive)?.id || 'box')},
    {id:'3d.add-light',label:'Add 3D Light',run:(type?:unknown)=>addLight(api,LIGHT_TYPES.find(l=>l.id===type)?.id || 'point')},
    {id:'3d.add-camera',label:'Add Camera',run:()=>addCamera(api)},
    {id:'3d.import-obj',label:'Import 3D Model…',run:(files?:unknown)=>importModelIntoScene(api,files)},
    {id:'3d.convert-layer',label:'Convert to 3D Layers',run:convert},
    {id:'3d.duplicate-selection',label:'Duplicate 3D Layers',run:()=>duplicateItems(api)},
    {id:'3d.delete-selection',label:'Delete 3D Layers',run:()=>removeItems(api)},
    {id:'3d.add-menu',label:'Add 3D Layer',run:(anchor?:unknown)=>api.ui.menu(anchor instanceof HTMLElement?anchor:{x:window.innerWidth/2,y:80},addMenuItems(api) as any)}
  ];
  for(const c of commands)api.commands.register({...c,category:'Layer'});
  api.inspector.registerSection({id:'3d-model',title:'3D',after:'content',when:({layerIds})=>layerIds.length===1&&(is3DLayer(api.model.layer(layerIds[0]!))||!!(api.model.layer(layerIds[0]!)?.d as any)?.modeling),
    build(target,{layerIds}){const ui=createSceneUiState(api),component=mount(Properties3D,{target,props:{api,ui,layerId:layerIds[0]!}});return()=>{void unmount(component);ui.dispose();};}});
  api.menus.contribute('timeline:context',()=>[{label:'Add 3D layer',icon:'cube',run:()=>api.commands.run('3d.add-menu')}] as any);
  api.menus.contribute('viewer:context',()=>[{label:'Add 3D layer',icon:'cube',run:()=>api.commands.run('3d.add-menu')}] as any);
  api.palette.registerProvider(query=>{
    const entries=[...commands,...PRIMITIVES.map(p=>({id:`3d.add-object.${p.id}`,label:`Add 3D ${p.label}`,run:()=>addPrimitive(api,p.id)})),...LIGHT_TYPES.map(l=>({id:`3d.add-light.${l.id}`,label:`Add ${l.label} Light`,run:()=>addLight(api,l.id)}))];
    return entries.filter(e=>!query.trim() || e.label.toLowerCase().includes(query.toLowerCase())).map(e=>({...e,category:'Layer'}));
  });
}
