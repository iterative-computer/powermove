<script lang="ts">
  import { untrack } from 'svelte';
  import type { ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
  import { chooseEnvironment } from './environment';
  import SceneField from './SceneField.svelte';
  import MaterialsSection from './MaterialsSection.svelte';
  import PartsSection from './PartsSection.svelte';
  import { MATERIAL_FIELDS, LIGHT_TYPES, lightFields, cameraFields, ENVIRONMENT_FIELDS } from './scene-model';
  import type { SceneUiState } from './scene-state.svelte';
  let { api,ui,layerId }:{api:PowermoveAPI;ui:SceneUiState;layerId:string}=$props();
  const {Row,Section,ToggleField,SelectField,ColorField}=untrack(()=>api.ui.controls);
  const layer=$derived((ui.version,api.model.layer(layerId)) as Layer|null);
  const data=$derived((ui.version,(layer?.d as any)?.data));
  const object=$derived(data?.object),light=$derived(data?.light),camera=$derived(data?.camera),environment=$derived(data?.environment);
  const value=(node:any,key:string)=>node?.p?.[key];
  const assetName=(id:string)=>(api.project.get().assets[id] as any)?.name;
  function edit(operation:any,patch:any,label:string){const result=api.scene3d.edit({operation,target:layerId,patch},{label,origin:'inspector'});if(!result.ok)api.ui.toast(result.message,{error:true});api.transport.invalidate();}
  const structural=(operation:any,key:string,label:string):ControlEditBinding=>({mode:'set',label,set:v=>edit(operation,{[key]:v},label)});
  /* Base fields the Materials options now cover show here only while they
     carry animation, so existing keyframes stay reachable. */
  const environmentOptions=$derived((ui.version,[{v:null,label:'None'},...Object.values(api.project.get().assets).filter((asset:any)=>asset.kind==='image' && /^(hdr|exr)$/i.test(asset.format || asset.name?.split('.').pop() || '')).map((asset:any)=>({v:asset.id,label:asset.name})),...(environment?.map && !api.project.get().assets[environment.map]?[{v:environment.map,label:'Missing image'}]:[]),{v:'__import__',label:'Import HDR / EXR…'}]));
  const baseFields=$derived((ui.version,MATERIAL_FIELDS.filter(field=>field.key==='color' || value(object?.material,field.key)?.kf?.length || value(object?.material,field.key)?.expr)));
</script>
{#if layer && data}
  {#if object}
    <Section {api} title="Model" />
    <Row {api} label="Source"><span class="k">{object.source.assetId?assetName(object.source.assetId) ?? 'Model':object.source.primitive ?? (object.source.lathe?'Lathe':object.source.extrude?'Extrusion':'Mesh')}</span></Row>
    {#if object.source.assetId}<Row {api} label="From file"><span class="from-file" title="Use the materials stored in the model file. Turn off to give the whole model one simple material."><ToggleField {api} get={()=>object.useSourceMaterials} edit={structural('update_object','useSourceMaterials','Model materials')} label="Use the model file’s materials" /></span></Row>{/if}
    {#if !object.useSourceMaterials}
      <Section {api} title="Material" />
      {#each baseFields as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`m.${field.key}`} channel={value(object.material,field.key)} />{/each}
    {/if}
    <Row {api} label="Cast shadows"><ToggleField {api} get={()=>object.castShadow} edit={structural('update_object','castShadow','Cast shadows')} label="Cast shadows" /></Row>
    <Row {api} label="Receive shadows"><ToggleField {api} get={()=>object.receiveShadow} edit={structural('update_object','receiveShadow','Receive shadows')} label="Receive shadows" /></Row>
    <PartsSection {api} {ui} {layer} />
    <MaterialsSection {api} {ui} {layer} />
  {:else if light}
    <Section {api} title="Light" />
    <Row {api} label="Type"><SelectField {api} get={()=>light.type} options={LIGHT_TYPES.map(t=>({v:t.id,label:t.label}))} edit={structural('update_light','type','Light type')} label="Light type" /></Row>
    {#each lightFields(light.type).filter(f=>!['x','y','z'].includes(f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`light.${field.key}`} channel={value(light,field.key)} />{/each}
    <Row {api} label="Cast shadows"><ToggleField {api} get={()=>light.castShadow} edit={structural('update_light','castShadow','Cast shadows')} label="Cast shadows" /></Row>
  {:else if camera}
    <Section {api} title="Camera" />
    <Row {api} label="Projection"><SelectField {api} get={()=>camera.projection} options={[{v:'perspective',label:'Perspective'},{v:'orthographic',label:'Orthographic'}]} edit={structural('set_camera','projection','Camera projection')} label="Projection" /></Row>
    {#each cameraFields(camera.projection).filter(f=>!['x','y','z'].includes(f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`camera.${field.key}`} channel={value(camera,field.key)} />{/each}
    <Section {api} title="Environment" />
    <Row {api} label="Map"><SelectField {api} get={()=>environment.map ?? null} options={environmentOptions} edit={structural('set_environment','map','Environment map')} onChange={(v:unknown)=>{if(v==='__import__'){void chooseEnvironment(api,layerId);return false;}}} label="Map" /></Row>
    {#each ENVIRONMENT_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`environment.${field.key}`} channel={value(environment,field.key)} />{/each}
    <Row {api} label="Shadows"><ToggleField {api} get={()=>environment.shadows} edit={structural('set_environment','shadows','Shadows')} label="Shadows" /></Row>
    <Row {api} label="Background"><ToggleField {api} get={()=>!!environment.background} edit={{mode:'set',label:'Background',set:(v:unknown)=>edit('set_environment',{background:v?'#161616':null},'Background')}} label="Background" />{#if environment.background}<ColorField {api} get={()=>environment.background} edit={structural('set_environment','background','Background color')} label="Background color" />{/if}</Row>
  {/if}
{/if}
<style>
  .from-file { display: contents; }
</style>
