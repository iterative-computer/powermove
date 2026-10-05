<script lang="ts">
  import {untrack} from 'svelte';
  import type {PowermoveAPI,Layer} from 'powermove';
  import type {SceneUiState} from './scene-state.svelte';
  import SceneField from './SceneField.svelte';
  import {MATERIAL_FIELDS,MAP_SLOTS} from './scene-model';
  let {api,ui,layer,object}:{api:PowermoveAPI;ui:SceneUiState;layer:Layer;object:any}=$props();
  const {Section,Row,SelectField,Disclosure,NumField}=untrack(()=>api.ui.controls);
  let selected=$state('base');
  const slots=$derived(object.slots?.length?object.slots:[{id:'base',name:'Surface',material:object.material}]);
  const slot=$derived(slots.find((s:any)=>s.id===selected)||slots[0]);
  const material=$derived(slot.material),shader=$derived(material.shader);
  const prefix=$derived(slot.id==='base'?'m':`slots.${slot.id}`);
  const shaderPrefix=$derived(slot.id==='base'?'shader':`${prefix}.shader`);
  const materials=$derived((ui.version,api.scene3d.materials()));
  const users=$derived(materials.find(m=>m.id===shader?.id)?.users||1);
  function set(next:any){const result=api.scene3d.setMaterial(layer.id,slot.id,next,!!shader&&next.shader?.id===shader.id&&users>1);if(!result.ok)api.ui.toast(result.message,{error:true});}
  function preset(id:string){const next=api.scene3d.createMaterial(id as any);set({...material,shader:next});}
  function textureMenu(key:string){const maps={...material.maps};return [{label:'Choose image…',icon:'image',run:async()=>{const [file]=await api.assets.pick({accept:'image/*'});if(!file)return;const asset=await api.assets.import(file);set({...material,maps:{...maps,[key]:asset.id}});}},...(maps[key]?[{label:'Remove',icon:'x',run:()=>{delete maps[key];set({...material,maps});}}]:[])];}
  const assetName=(id:string)=>(api.project.get().assets[id] as any)?.name||'Texture';
</script>
<Section {api} title="Material" />
{#if slots.length>1}<Row {api} label="Surface"><SelectField {api} label="Material slot" get={()=>slot.id} options={slots.map((s:any)=>({v:s.id,label:s.name}))} edit={{mode:'local',label:'Material slot',set:(v:unknown)=>selected=String(v)}} /></Row>{/if}
<Row {api} label="Material"><button type="button" class="chip" onclick={event=>api.ui.menu(event.currentTarget as HTMLElement,[...materials.map(m=>({label:m.name,run:()=>set(m.material)})),{label:'New material',run:()=>preset('surface')}])}>{shader?.name||slot.name}</button></Row>
{#if shader}
  <Row {api} label="Used by"><span class="k">{users}</span>{#if users>1}<button type="button" class="chip" onclick={()=>set({...material,shader:{...shader,id:`material_${crypto.randomUUID().replaceAll('-','')}`}})}>Make unique</button>{/if}</Row>
{/if}
<Row {api} label="Preset"><SelectField {api} label="Material preset" get={()=>shader?.preset||'custom'} options={[{v:'custom',label:shader?.source?'Blender material':'Custom'},...api.scene3d.materialPresets.map(p=>({v:p.id,label:p.label}))]} edit={{mode:'set',label:'Material preset',set:(v:unknown)=>{if(v!=='custom')preset(String(v));}}} /></Row>
{#if shader}
  {#each shader.inputs as input (input.id)}
    <SceneField {api} {ui} {layer} path={`${shaderPrefix}.${input.id}`} channel={shader.p[input.id]} field={{key:input.id,label:input.label,kind:input.kind==='number'?'num':input.kind,min:input.min,max:input.max,step:input.step}} />
  {/each}
{:else}
  {#each MATERIAL_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`${prefix}.${field.key}`} channel={material.p[field.key]} />{/each}
{/if}
<Disclosure label="Textures" remember={`textures:${layer.id}:${slot.id}`} changed={Object.keys(material.maps||{}).length>0}>
  {#each MAP_SLOTS as map (map.id)}<Row {api} label={map.label}><button type="button" class="chip" onclick={event=>api.ui.menu(event.currentTarget as HTMLElement,textureMenu(map.id))}>{material.maps?.[map.id]?assetName(material.maps[map.id]):'Choose…'}</button></Row>{/each}
  {#each ['scaleX','scaleY','offsetX','offsetY','rotation'] as key (key)}<Row {api} label={{scaleX:'Scale X',scaleY:'Scale Y',offsetX:'Offset X',offsetY:'Offset Y',rotation:'Rotation'}[key]}><NumField {api} label={`Texture ${key}`} get={()=>material.placement?.[key]??(key.startsWith('scale')?1:0)} min={key.startsWith('scale')?.001:undefined} step={key==='rotation'?1:.01} edit={{mode:'set',label:'Texture placement',set:(v:unknown)=>set({...material,placement:{scaleX:1,scaleY:1,offsetX:0,offsetY:0,rotation:0,...material.placement,[key]:v}})}} /></Row>{/each}
</Disclosure>
