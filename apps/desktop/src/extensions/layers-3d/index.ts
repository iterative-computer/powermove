import { mount, unmount } from 'svelte';
import type { ExtensionLayerDefinition, PowermoveAPI } from 'powermove';
import { OBJ_MODEL } from './obj-model';
import { STUDIO_CUBE } from './studio-cube';
import LayerInspector from './LayerInspector.svelte';
import {addPrimitive,addLight,addCamera,importModelIntoScene,duplicateItems,removeItems,is3DLayer,addMenuItems} from './scene-actions';
import {PRIMITIVES,LIGHT_TYPES} from './scene-model';
import {createSceneUiState} from './scene-state.svelte';
import {openMaterialPicker,closeMaterialPicker} from './material-picker';
import {materialThumbnails,disposeMaterialThumbnails} from './material-thumbnails';
import {canAssignLayer,dropLayer,setLayerDropHover,slotAt,type SlotHover,type SlotRef,type AssignCheck} from './layer-drop.svelte';
import {materialEntries,describeMap,materialUsage,usageTitle,installProperty,propertyChannel,propertyPath,MATERIAL_PROPERTY_SPECS,MATERIAL_SLOT_SPECS,type MaterialEntry} from './materials';

/** Shared with the Timeline so its Materials rows match the inspector. */
export interface MaterialsService {
  entries:(layerId:string)=>MaterialEntry[];
  describe:(layerId:string,index:number|null,slot:any,map:any)=>ReturnType<typeof describeMap>;
  usage:(sourceId:string)=>ReturnType<typeof materialUsage>;
  usageTitle:typeof usageTitle;
  openPicker:(anchor:HTMLElement|{left:number;top:number;bottom:number;width:number},request:{layerId:string;materialIndex:number|null;slot:any})=>void;
  closePicker:()=>void;
  properties:typeof MATERIAL_PROPERTY_SPECS;
  slots:typeof MATERIAL_SLOT_SPECS;
  propertyPath:typeof propertyPath;
  propertyChannel:typeof propertyChannel;
  propertyValues:(layerId:string,index:number|null,time:number)=>Record<string,unknown>|null;
  installProperty:(layerId:string,index:number|null,key:any,value:unknown,keyAt?:number)=>unknown;
  /** A source's cached preview (PNG data URL), or null for the placeholder; asks the engine when missing. */
  thumbnail:(layerId:string,map:any)=>string|null;
  /** Called when previews arrive. */
  onThumbnails:(listener:()=>void)=>()=>void;
  /** Layer drags from the Timeline: the Properties slot under a point, the engine's preflight, the hover highlight, and the drop. */
  slotAt:(x:number,y:number)=>SlotRef|null;
  canAssignLayer:(target:SlotRef,sourceId:string)=>AssignCheck;
  setLayerDropHover:(hover:SlotHover|null)=>void;
  dropLayer:(target:SlotRef,sourceId:string)=>boolean;
}

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
  const commands=[
    {id:'3d.add-studio-cube',label:'Add 3D Studio Cube',run:()=>api.project.apply({type:'add_layer',layerType:'extension',name:'3D Studio Cube',content:{definition:STUDIO_CUBE.id}},{label:'Add 3D Studio Cube'})},
    {id:'3d.new-model',label:'New 3D Model',run:()=>addPrimitive(api)},
    {id:'3d.add-object',label:'Add 3D Object',run:(primitive?:unknown)=>addPrimitive(api,PRIMITIVES.find(p=>p.id===primitive)?.id || 'box')},
    {id:'3d.add-light',label:'Add 3D Light',run:(type?:unknown)=>addLight(api,LIGHT_TYPES.find(l=>l.id===type)?.id || 'point')},
    {id:'3d.add-camera',label:'Add Camera',run:()=>addCamera(api)},
    {id:'3d.import-obj',label:'Import 3D Model…',run:(files?:unknown)=>importModelIntoScene(api,files)},
    {id:'3d.convert-layer',label:'Convert to 3D Layers',run:convert},
    {id:'3d.duplicate-selection',label:'Duplicate 3D Layers',run:()=>duplicateItems(api)},
    {id:'3d.delete-selection',label:'Delete 3D Layers',run:()=>removeItems(api)},
    {id:'3d.add-menu',label:'Add 3D Layer',run:()=>api.ui.menu({x:window.innerWidth/2,y:80},addMenuItems(api) as any)}
  ];
  for(const c of commands)api.commands.register({...c,category:'Layer'});
  api.inspector.registerSection({id:'3d-model',title:'3D',after:'content',when:({layerIds})=>layerIds.length===1&&is3DLayer(api.model.layer(layerIds[0]!)),
    build(target,{layerIds}){const ui=createSceneUiState(api),component=mount(LayerInspector,{target,props:{api,ui,layerId:layerIds[0]!}});return()=>{void unmount(component);ui.dispose();};}});
  const materials:MaterialsService={
    entries:layerId=>materialEntries(api,layerId),
    describe:(layerId,index,slot,map)=>describeMap(api,layerId,index,slot,map),
    usage:sourceId=>materialUsage(api,sourceId),usageTitle,
    openPicker:(anchor,request)=>openMaterialPicker(api,anchor,request),closePicker:closeMaterialPicker,
    properties:MATERIAL_PROPERTY_SPECS,slots:MATERIAL_SLOT_SPECS,propertyPath,propertyChannel,
    propertyValues:(layerId,index,time)=>(api.scene3d.getMaterialProperties(layerId,index,time)?.values as any) ?? null,
    installProperty:(layerId,index,key,value,keyAt)=>installProperty(api,layerId,index,key,value,keyAt),
    thumbnail:(layerId,map)=>materialThumbnails(api).peek(layerId,map),
    onThumbnails:listener=>materialThumbnails(api).subscribe(listener),
    slotAt,canAssignLayer:(target,sourceId)=>canAssignLayer(api,target,sourceId),setLayerDropHover,
    dropLayer:(target,sourceId)=>dropLayer(api,target,sourceId)
  };
  const service=api.services?.register?.('materials',materials);
  api.onDispose?.(()=>{closeMaterialPicker();setLayerDropHover(null);disposeMaterialThumbnails();service?.dispose();});
  // A submenu at the pointer, rather than a second menu opened elsewhere.
  // Empty canvas and empty timeline only.
  api.menus.contribute('timeline:context',(ctx:any)=>ctx?.kind&&ctx.kind!=='empty'?[]:[{label:'Add 3D Layer',icon:'cube',submenu:addMenuItems(api)}]);
  api.menus.contribute('viewer:context',(ctx:any)=>ctx?.kind&&ctx.kind!=='empty'?[]:[{label:'Add 3D Layer',icon:'cube',submenu:addMenuItems(api)}]);
  api.palette.registerProvider(query=>{
    const entries=[...commands,...PRIMITIVES.map(p=>({id:`3d.add-object.${p.id}`,label:`Add 3D ${p.label}`,run:()=>addPrimitive(api,p.id)})),...LIGHT_TYPES.map(l=>({id:`3d.add-light.${l.id}`,label:`Add ${l.label} Light`,run:()=>addLight(api,l.id)}))];
    return entries.filter(e=>!query.trim() || e.label.toLowerCase().includes(query.toLowerCase())).map(e=>({...e,category:'Layer'}));
  });
}
