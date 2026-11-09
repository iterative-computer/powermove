<script lang="ts">
  import { untrack } from 'svelte';
  import type { ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
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
  const studio=untrack(()=>api.scene3d.studio);
  // Swatches rendered by Studio replace the flat colors as they arrive.
  let swatches=$state<Record<string,string>>({});
  $effect(()=>{for(const finish of studio.materials)void studio.swatch(finish.id).then(url=>{if(url)swatches[finish.id]=url;});});
  function studioEdit(args:any,label:string){const result=api.scene3d.edit({target:layerId,...args},{label,origin:'inspector'});if(!result.ok)api.ui.toast(result.message,{error:true});}
  function edit(operation:any,patch:any,label:string){const result=api.scene3d.edit({operation,target:layerId,patch},{label,origin:'inspector'});if(!result.ok)api.ui.toast(result.message,{error:true});api.transport.invalidate();}
  const structural=(operation:any,key:string,label:string):ControlEditBinding=>({mode:'set',label,set:v=>edit(operation,{[key]:v},label)});
  /* Base fields the Materials options now cover show here only while they
     carry animation, so existing keyframes stay reachable. */
  const baseFields=$derived((ui.version,MATERIAL_FIELDS.filter(field=>!['roughness','metalness','opacity','emissive','emissiveIntensity'].includes(field.key) || value(object?.material,field.key)?.kf?.length || value(object?.material,field.key)?.expr)));
</script>
{#if layer && data}
  {#if object}
    <Section {api} title="Model" />
    <div class="shots" role="group" aria-label="Model actions"><button type="button" class="chip" onclick={()=>studioEdit({operation:'drop_to_floor'},'Drop to floor')}>Drop to floor</button><button type="button" class="chip" onclick={()=>studioEdit({operation:"shot",shot:"turntable",duration:4},"Turntable")}>Turntable</button></div>
    <Row {api} label="Source"><span class="k">{object.source.assetId?assetName(object.source.assetId) ?? 'Model':object.source.primitive ?? (object.source.lathe?'Lathe':object.source.extrude?'Extrusion':'Mesh')}</span></Row>
    {#if object.source.assetId}<Row {api} label="From file"><span class="from-file" title="Use the materials stored in the model file. Turn off to give the whole model one simple material."><ToggleField {api} get={()=>object.useSourceMaterials} edit={structural('update_object','useSourceMaterials','Model materials')} label="Use the model file’s materials" /></span></Row>{/if}
    {#if !object.useSourceMaterials}
      <Section {api} title="Material" />
      <div class="shelf" role="group" aria-label="Finish">{#each studio.materials as finish (finish.id)}<button type="button" class="finish" title={finish.label} aria-label={finish.label} style={`--swatch:${finish.p.color};${swatches[finish.id]?`background-image:url(${swatches[finish.id]});background-size:cover`:""}`} onclick={()=>studioEdit({operation:"apply_material",preset:finish.id},finish.label)}></button>{/each}</div>
      {#each baseFields as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`m.${field.key}`} channel={value(object.material,field.key)} />{/each}
    {/if}
    <Row {api} label="Cast shadows"><ToggleField {api} get={()=>object.castShadow} edit={structural('update_object','castShadow','Cast shadows')} label="Cast shadows" /></Row>
    <Row {api} label="Receive shadows"><ToggleField {api} get={()=>object.receiveShadow} edit={structural('update_object','receiveShadow','Receive shadows')} label="Receive shadows" /></Row>
    <PartsSection {api} {ui} {layer} />
    <MaterialsSection {api} {ui} {layer} />
  {:else if light}
    <Section {api} title="Light" />
    <Row {api} label="Type"><SelectField {api} get={()=>light.type} options={LIGHT_TYPES.map(t=>({v:t.id,label:t.label}))} edit={structural('update_light','type','Light type')} label="Light type" /></Row>
    {#each lightFields(light.type).filter(f=>!['x','y','z'].includes(f.key) && value(light,f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`light.${field.key}`} channel={value(light,field.key)} />{/each}
    <Row {api} label="Cast shadows"><ToggleField {api} get={()=>light.castShadow} edit={structural('update_light','castShadow','Cast shadows')} label="Cast shadows" /></Row>
  {:else if camera}
    <Section {api} title="Camera" />
    <Row {api} label="Projection"><SelectField {api} get={()=>camera.projection} options={[{v:'perspective',label:'Perspective'},{v:'orthographic',label:'Orthographic'}]} edit={structural('set_camera','projection','Camera projection')} label="Projection" /></Row>
    {#each cameraFields(camera.projection).filter(f=>!['x','y','z'].includes(f.key) && value(camera,f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`camera.${field.key}`} channel={value(camera,field.key)} />{/each}
    <Section {api} title="Shot" />
    <div class="shots" role="group" aria-label="Studio"><button type="button" class="chip" onclick={()=>studioEdit({operation:'auto_studio'},'Set up studio')}>Set up studio</button></div>
    <div class="shots" role="group" aria-label="Shots">{#each studio.shots.filter(shot=>shot.id!=='turntable') as shot (shot.id)}<button type="button" class="chip" onclick={()=>studioEdit({operation:'shot',shot:shot.id,duration:4},shot.label)}>{shot.label}</button>{/each}</div>
    <Row {api} label="Final quality"><SelectField {api} get={()=>String(environment.samples ?? 64)} options={[{v:'16',label:'Draft · 16'},{v:'64',label:'Standard · 64'},{v:'256',label:'High · 256'},{v:'1024',label:'Best · 1024'}]} edit={{mode:'set',label:'Final quality',set:(v:unknown)=>edit('set_environment',{samples:Number(v)},'Final quality')}} label="Final quality" /></Row>
    <Row {api} label="Studio set"><ToggleField {api} get={()=>!!environment.shadowCatcher} edit={{mode:'set',label:'Studio set',set:(v:unknown)=>studioEdit({operation:'set_studio',on:!!v},'Studio set')}} label="Studio set" /></Row>
    <Section {api} title="Environment" />
    {#each ENVIRONMENT_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`environment.${field.key}`} channel={value(environment,field.key)} />{/each}
    <Row {api} label="Shadows"><ToggleField {api} get={()=>environment.shadows} edit={structural('set_environment','shadows','Shadows')} label="Shadows" /></Row>
    <Row {api} label="Background"><ToggleField {api} get={()=>!!environment.background} edit={{mode:'set',label:'Background',set:(v:unknown)=>edit('set_environment',{background:v?'#161616':null},'Background')}} label="Background" />{#if environment.background}<ColorField {api} get={()=>environment.background} edit={structural('set_environment','background','Background color')} label="Background color" />{/if}</Row>
  {/if}
{/if}
<style>
  .shelf{display:grid;grid-template-columns:repeat(auto-fill,minmax(22px,1fr));gap:6px;padding:2px var(--panel-edge-inset,10px) 8px}
  .shots{display:flex;flex-wrap:wrap;gap:6px;padding:2px var(--panel-edge-inset,10px) 8px}
  .finish{aspect-ratio:1;width:100%;padding:0;border:0;border-radius:var(--r-sm,6px);background:var(--swatch);cursor:pointer}
  .finish:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .from-file { display: contents; }
</style>
