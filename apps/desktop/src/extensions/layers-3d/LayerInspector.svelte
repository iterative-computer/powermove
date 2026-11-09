<script lang="ts">
  import { untrack } from 'svelte';
  import type { ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
  import SceneField from './SceneField.svelte';
  import MaterialsSection from './MaterialsSection.svelte';
  import PartsSection from './PartsSection.svelte';
  import WorldSection from './WorldSection.svelte';
  import MaterialOptionsRows from './MaterialOptionsRows.svelte';
  import { FOG_FIELDS, TERRAIN_FIELDS, REGION_FIELDS, SCATTER_FIELDS, IRIS_FIELDS, SKY_PRESET_VALUES, environmentKind, worldLayer, addWorld, scatterSources, environmentSourceChoices, pickEnvironmentImage, sourceKey, sourceFromKey } from './environment-ui';
  import { MATERIAL_FIELDS, LIGHT_TYPES, lightFields, cameraFields, ENVIRONMENT_FIELDS } from './scene-model';
  import type { SceneUiState } from './scene-state.svelte';
  let { api,ui,layerId }:{api:PowermoveAPI;ui:SceneUiState;layerId:string}=$props();
  const {Row,Section,ToggleField,SelectField,ColorField,NumField}=untrack(()=>api.ui.controls);
  const layer=$derived((ui.version,api.model.layer(layerId)) as Layer|null);
  const data=$derived((ui.version,(layer?.d as any)?.data));
  const object=$derived(data?.object),light=$derived(data?.light),camera=$derived(data?.camera),environment=$derived(data?.environment);
  const value=(node:any,key:string)=>node?.p?.[key];
  const assetName=(id:string)=>(api.project.get().assets[id] as any)?.name;
  const kind=$derived(environmentKind(object));
  const world=$derived((ui.version,camera?worldLayer(api):null));
  const sources=$derived((ui.version,kind==='scatter'?scatterSources(api):null));
  const ev=(prop:any,path:string)=>prop?api.anim.evP(layer!,prop,ui.time,path):undefined;
  /* Terrain regions list only the limits they use; ±100000 means "no limit". */
  const regionFields=(region:any)=>REGION_FIELDS.filter(f=>{const v=region.p[f.key]?.v;return f.key==='color' || (f.key==='minSlope'?v>0:f.key==='maxSlope'?v<90:f.key==='minHeight'?v>-100000:f.key==='maxHeight'?v<100000:false);});
  const TERRAIN_LOOKS=[{id:'hills',label:'Rolling hills',p:{amplitude:3,frequency:.12,octaves:5,ridged:false}},{id:'mountains',label:'Mountains',p:{amplitude:7,frequency:.08,octaves:7,ridged:true}},{id:'dunes',label:'Dunes',p:{amplitude:1.4,frequency:.18,octaves:3,ridged:false}},{id:'flat',label:'Nearly flat',p:{amplitude:.4,frequency:.1,octaves:3,ridged:false}}];
  /* `_version` re-reads the terrain after edits. */
  const terrainLook=(terrain:any,_version?:number)=>TERRAIN_LOOKS.find(look=>Object.entries(look.p).every(([k,v])=>{const c=terrain?.p?.[k];return c && !c.kf?.length && (typeof v==='number'?Math.abs(Number(c.v)-v)<1e-6:c.v===v);}))?.id ?? null;
  const heightChoices=$derived((ui.version,[{v:'noise',label:'Generated'},...(typeof object?.terrain?.heightSource==='string'?[{v:`asset:${object.terrain.heightSource}`,label:assetName(object.terrain.heightSource) ?? 'Image'}]:[]),{v:'pick',label:'Height image…'},...environmentSourceChoices(api)]));
  async function chooseHeight(next:unknown){
    if(next==='noise')return edit('update_terrain',{heightSource:null},'Terrain height');
    if(next==='pick'){const id=await pickEnvironmentImage(api);if(id)edit('update_terrain',{heightSource:id},'Terrain height image');return;}
    edit('update_terrain',{heightSource:sourceFromKey(String(next))},'Terrain height');
  }
  const optionValue=(key:string):unknown=>key==='castsShadows'?(object?.castShadow?'on':'off'):key==='acceptsShadows'?object?.receiveShadow!==false:true;
  /* Cameras made before iris existed have no iris channels: the first edit
     installs all four (set_camera fills defaults), then sets the value. */
  const IRIS_DEFAULTS:Record<string,number>={irisShape:0,irisRotation:0,irisRoundness:0,irisAspect:1};
  function installIris(key:string,v:unknown,label:string){
    api.edit.begin(label,{origin:'inspector'});
    try{
      const r=api.scene3d.edit({operation:'set_camera',target:layerId,patch:{projection:camera.projection}},{origin:'inspector'});if(!r.ok)throw new Error(r.message);
      const s=api.edit.apply({type:'set_property',target:layerId,path:`camera.${key}`,value:Number(v),time:api.transport.time(),mode:'auto',preserveHandEdits:false} as any,{origin:'inspector'});if(!s.ok)throw new Error(s.message);
      const c=api.edit.commit(label);if(!c.ok)throw new Error(c.message);
    }catch(error){api.edit.cancel();api.ui.toast(error instanceof Error?error.message:String(error),{error:true});}
    api.transport.invalidate();
  }
  function environmentLightEdit(patch:any,label:string){
    const p=patch.preset?{...SKY_PRESET_VALUES[patch.preset as keyof typeof SKY_PRESET_VALUES]}:{};
    edit('update_light',{world:{...patch,p}},label);
  }
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
  {#if kind==='world'}
    <WorldSection {api} {ui} {layer} world={object.world} edit={(patch,label)=>studioEdit({operation:'set_world',patch},label)} />
  {:else if kind==='fog'}
    <Section {api} title="Fog Volume" />
    {#each FOG_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`fog.${field.key}`} channel={object.fog.p[field.key]} />{/each}
  {:else if kind==='terrain'}
    <Section {api} title="Terrain" />
    <!-- Four shapes in an even grid; the one the terrain matches reads as chosen. -->
    <div class="shots looks" role="group" aria-label="Terrain shape">{#each TERRAIN_LOOKS as look (look.id)}<button type="button" class="chip" aria-pressed={terrainLook(object.terrain,ui.version)===look.id} onclick={()=>edit('update_terrain',{p:look.p},look.label)}>{look.label}</button>{/each}</div>
    <Row {api} label="Shape from"><SelectField {api} get={()=>object.terrain.heightSource?sourceKey(object.terrain.heightSource):'noise'} options={heightChoices} edit={{mode:'set',label:'Terrain height',set:(v:unknown)=>void chooseHeight(v)}} label="Terrain shape source" /></Row>
    {#each TERRAIN_FIELDS.filter(f=>!object.terrain.heightSource || !['frequency','octaves','ridged','seed'].includes(f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`terrain.${field.key}`} channel={object.terrain.p[field.key]} />{/each}
    {#each object.terrain.regions as region, i (i)}
      <Section {api} title={i===0?'Ground cover':`Ground cover ${i+1}`} />
      {#each regionFields(region) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`terrain.region${i}.${field.key}`} channel={region.p[field.key]} />{/each}
    {/each}
    {#if !object.terrain.regions.length}<p class="note">One material covers the ground. Agents and the Terrain defaults add rock on steep slopes and grass below.</p>{/if}
  {:else if kind==='scatter'}
    <Section {api} title="Scatter" />
    <Row {api} label="Copies of"><SelectField {api} get={()=>object.scatter.model} options={[...(sources?.models.some(m=>m.id===object.scatter.model)?[]:[{v:object.scatter.model,label:'Missing model'}]),...(sources?.models ?? []).map(m=>({v:m.id,label:m.name}))]} edit={{mode:'set',label:'Scatter model',set:(v:unknown)=>edit('update_scatter',{model:v},'Scatter model')}} label="Scatter copies of" /></Row>
    <Row {api} label="Over"><SelectField {api} get={()=>object.scatter.surface} options={[...(sources?.surfaces.some(m=>m.id===object.scatter.surface)?[]:[{v:object.scatter.surface,label:'Missing surface'}]),...(sources?.surfaces ?? []).map(m=>({v:m.id,label:m.name}))]} edit={{mode:'set',label:'Scatter surface',set:(v:unknown)=>edit('update_scatter',{surface:v},'Scatter surface')}} label="Scatter over" /></Row>
    <div class="shots" role="group" aria-label="Scatter actions"><button type="button" class="chip" onclick={()=>edit('update_scatter',{p:{seed:Math.round(Number(ev(object.scatter.p.seed,'scatter.seed') ?? 0))+1}},'New variation')}>New variation</button></div>
    {#each SCATTER_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`scatter.${field.key}`} channel={object.scatter.p[field.key]} />{/each}
  {:else if object}
    <Section {api} title="Model" />
    <div class="shots" role="group" aria-label="Model actions"><button type="button" class="chip" onclick={()=>studioEdit({operation:'drop_to_floor'},'Drop to floor')}>Drop to floor</button><button type="button" class="chip" onclick={()=>studioEdit({operation:"shot",shot:"turntable",duration:4},"Turntable")}>Turntable</button></div>
    <Row {api} label="Source"><span class="k">{object.source.assetId?assetName(object.source.assetId) ?? 'Model':object.source.primitive ?? (object.source.lathe?'Lathe':object.source.extrude?'Extrusion':'Mesh')}</span></Row>
    {#if object.source.assetId}<Row {api} label="From file"><span class="from-file" title="Use the materials stored in the model file. Turn off to give the whole model one simple material."><ToggleField {api} get={()=>object.useSourceMaterials} edit={structural('update_object','useSourceMaterials','Model materials')} label="Use the model file’s materials" /></span></Row>{/if}
    {#if !object.useSourceMaterials}
      <Section {api} title="Material" />
      <div class="shelf" role="group" aria-label="Finish">{#each studio.materials as finish (finish.id)}<button type="button" class="finish" title={finish.label} aria-label={finish.label} style={`--swatch:${swatches[finish.id]?'transparent':finish.p.color};${swatches[finish.id]?`background-image:url(${swatches[finish.id]});background-size:cover`:""}`} onclick={()=>studioEdit({operation:"apply_material",preset:finish.id},finish.label)}></button>{/each}</div>
      {#each baseFields as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`m.${field.key}`} channel={value(object.material,field.key)} />{/each}
    {/if}
    <MaterialOptionsRows {api} {ui} {layer} options={object.materialOptions} fallback={optionValue} />
    <PartsSection {api} {ui} {layer} />
    <MaterialsSection {api} {ui} {layer} />
  {:else if light}
    <Section {api} title="Light" />
    <Row {api} label="Type"><SelectField {api} get={()=>light.type} options={LIGHT_TYPES.map(t=>({v:t.id,label:t.label}))} edit={structural('update_light','type','Light type')} label="Light type" /></Row>
    {#each lightFields(light.type).filter(f=>!['x','y','z'].includes(f.key) && value(light,f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`light.${field.key}`} channel={value(light,field.key)} />{/each}
    {#if light.type==='environment' && light.world}<WorldSection {api} {ui} {layer} world={light.world} title="Environment" edit={environmentLightEdit} />{/if}
    {#if light.type!=='ambient' && light.type!=='environment'}<Row {api} label="Cast shadows"><ToggleField {api} get={()=>light.castShadow} edit={structural('update_light','castShadow','Cast shadows')} label="Cast shadows" /></Row>{/if}
  {:else if camera}
    <Section {api} title="Camera" />
    <Row {api} label="Projection"><SelectField {api} get={()=>camera.projection} options={[{v:'perspective',label:'Perspective'},{v:'orthographic',label:'Orthographic'}]} edit={structural('set_camera','projection','Camera projection')} label="Projection" /></Row>
    {#each cameraFields(camera.projection).filter(f=>!['x','y','z'].includes(f.key) && value(camera,f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`camera.${field.key}`} channel={value(camera,field.key)} />{/each}
    <Section {api} title="Lens" />
    <div class="wrap-labels">
    {#each IRIS_FIELDS as field (field.key)}
      {#if value(camera,field.key)}<SceneField {api} {ui} {layer} {field} path={`camera.${field.key}`} channel={value(camera,field.key)} />
      {:else if field.kind==='select'}<Row {api} label={field.label}><SelectField {api} get={()=>String(IRIS_DEFAULTS[field.key])} options={(field.options ?? []).map(o=>({v:String(o.v),label:o.label}))} edit={{mode:'set',label:field.label,set:(v:unknown)=>installIris(field.key,v,field.label)}} label={field.label} /></Row>
      {:else}<Row {api} label={field.label}><div class="well"><NumField {api} angle={field.unit==='°'} get={()=>IRIS_DEFAULTS[field.key]} edit={{mode:'set',label:field.label,set:(v:unknown)=>installIris(field.key,v,field.label)}} label={field.label} ariaLabel={field.label} step={field.step ?? 1} min={field.min} max={field.max} unit={field.unit} /></div></Row>{/if}
    {/each}
    </div>
    {#if world}
      <WorldSection {api} {ui} layer={world} world={world.d.data.object.world} edit={(patch,label)=>{const r=api.scene3d.edit({operation:'set_world',patch},{label,origin:'inspector'});if(!r.ok)api.ui.toast(r.message,{error:true});api.transport.invalidate();}} />
    {:else}
      <Section {api} title="World" />
      <div class="shots" role="group" aria-label="World"><button type="button" class="chip" onclick={()=>{addWorld(api);api.selection.select([layerId]);}}>Add sky</button></div>
    {/if}
    <Section {api} title="Shot" />
    <div class="shots" role="group" aria-label="Studio"><button type="button" class="chip" onclick={()=>studioEdit({operation:'auto_studio'},'Set up studio')}>Set up studio</button></div>
    <div class="shots" role="group" aria-label="Shots">{#each studio.shots.filter(shot=>shot.id!=='turntable') as shot (shot.id)}<button type="button" class="chip" onclick={()=>studioEdit({operation:'shot',shot:shot.id,duration:4},shot.label)}>{shot.label}</button>{/each}</div>
    <Row {api} label="Final quality"><SelectField {api} get={()=>String(environment.samples ?? 64)} options={[{v:'16',label:'Draft · 16'},{v:'64',label:'Standard · 64'},{v:'256',label:'High · 256'},{v:'1024',label:'Best · 1024'}]} edit={{mode:'set',label:'Final quality',set:(v:unknown)=>edit('set_environment',{samples:Number(v)},'Final quality')}} label="Final quality" /></Row>
    <Row {api} label="Studio set"><ToggleField {api} get={()=>!!environment.shadowCatcher} edit={{mode:'set',label:'Studio set',set:(v:unknown)=>studioEdit({operation:'set_studio',on:!!v},'Studio set')}} label="Studio set" /></Row>
    <Section {api} title="Scene" />
    {#each ENVIRONMENT_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`environment.${field.key}`} channel={value(environment,field.key)} />{/each}
    <Row {api} label="Shadows"><ToggleField {api} get={()=>environment.shadows} edit={structural('set_environment','shadows','Shadows')} label="Shadows" /></Row>
    <Row {api} label="Background"><ToggleField {api} get={()=>!!environment.background} edit={{mode:'set',label:'Background',set:(v:unknown)=>edit('set_environment',{background:v?'#161616':null},'Background')}} label="Background" />{#if environment.background}<ColorField {api} get={()=>environment.background} edit={structural('set_environment','background','Background color')} label="Background color" />{/if}</Row>
  {/if}
{/if}
<style>
  .shelf{display:grid;grid-template-columns:repeat(auto-fill,minmax(22px,1fr));gap:6px;padding:2px var(--panel-edge-inset,10px) 8px}
  .shots{display:flex;flex-wrap:wrap;gap:6px;padding:2px var(--panel-edge-inset,10px) 8px}
  .looks{display:grid;grid-template-columns:repeat(2,minmax(0,1fr))}
  .looks .chip{justify-content:center;text-align:center}
  .looks .chip[aria-pressed="true"]{background:var(--accent-dim);color:var(--tx);box-shadow:inset 0 0 0 1px color-mix(in srgb,var(--accent) 55%,transparent)}
  .finish{aspect-ratio:1;width:100%;padding:0;border:0;border-radius:var(--r-sm,6px);background:var(--swatch);cursor:pointer}
  .finish:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .from-file { display: contents; }
  /* After Effects' long names wrap rather than lose their ends. */
  .wrap-labels :global(.row .k){white-space:normal;overflow:visible;text-overflow:clip;line-height:1.15}
  .note{margin:0;padding:2px var(--panel-edge-inset,10px) 8px;font-size:var(--fs-xs,11px);color:var(--tx-3);line-height:1.35}
</style>
