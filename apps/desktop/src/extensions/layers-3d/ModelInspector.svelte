<script lang="ts">
  import {untrack} from 'svelte';
  import type {PowermoveAPI,Layer} from 'powermove';
  import type {SceneUiState} from './scene-state.svelte';
  import SceneField from './SceneField.svelte';
  let {api,ui,layerId}:{api:PowermoveAPI;ui:SceneUiState;layerId:string}=$props();
  const {Section,Row,TextField,NumField,ToggleField}=untrack(()=>api.ui.controls);
  const layer=$derived((ui.version,api.model.layer(layerId)) as Layer);
  const model=$derived((ui.version,(layer?.d as any)?.modeling));
  let busy=$state(false);
  function patchRecipe(patch:any){const result=api.edit.apply({type:'set_content',target:layerId,patch:{modeling:{...model,recipe:{...model.recipe,...patch}}}},{label:'Model settings',origin:'inspector'});if(!result.ok)api.ui.toast(result.message,{error:true});}
  async function regenerate(){
    const imported=model.recipe.kind==='mesh'&&!model.recipe.mesh;
    const [file]=imported?await api.assets.pick({accept:'.blend'}):[];
    if(imported&&!file)return;
    busy=true;
    try{const result=await api.scene3d.model({operation:imported?'import_blend':'regenerate_model',target:layerId,...(file?{file,name:layer.name}:{})});if(!result.ok)api.ui.toast(result.message,{error:true});}finally{busy=false;}
  }
  function modifier(index:number,patch:any){patchRecipe({modifiers:model.recipe.modifiers.map((m:any,i:number)=>i===index?{...m,...patch}:m)});}
  const defaults=[{type:'bevel',width:.04,segments:3},{type:'subdivision',levels:1},{type:'solidify',thickness:.1},{type:'array',count:3,offset:[1.2,0,0]},{type:'mirror',axes:[true,false,false]}];
</script>
{#if model}
  <Section {api} title="Model" />
  <Row {api} label="Shape"><span class="k">{api.scene3d.modelRecipes.find(r=>r.id===model.recipe.kind)?.label||'Custom model'}</span></Row>
  {#each Object.entries(model.recipe.parameters) as [key,value] (key)}
    {@const field=api.scene3d.modelFieldsFor(model.recipe.kind)[key]||{label:key,step:.1}}
    {#if model.p[key]}
      <SceneField {api} {ui} {layer} path={`model.${key}`} channel={model.p[key]} field={{key,label:field.label||key,kind:'num',min:field.min,max:field.max,step:field.step}} />
    {:else}
      <Row {api} label={field.label||key}>
        {#if typeof value==='string'}<TextField {api} label={field.label||key} get={()=>value} edit={{mode:'set',label:field.label||key,set:(v:unknown)=>patchRecipe({parameters:{...model.recipe.parameters,[key]:v}})}} />
        {:else}<NumField {api} label={field.label||key} get={()=>value} min={field.min} max={field.max} step={field.step} edit={{mode:'set',label:field.label||key,set:(v:unknown)=>patchRecipe({parameters:{...model.recipe.parameters,[key]:v}})}} />{/if}
      </Row>
    {/if}
  {/each}
  {#if model.recipe.kind!=='mesh'||model.recipe.mesh}
  <Section {api} title="Modifiers" />
  {#each model.recipe.modifiers as mod,index (index)}
    <Row {api} label={mod.type.charAt(0).toUpperCase()+mod.type.slice(1)}>
      {@const key=mod.type==='bevel'?'width':mod.type==='subdivision'?'levels':mod.type==='solidify'?'thickness':mod.type==='array'?'count':null}
      {#if key}<NumField {api} label={`${mod.type} ${key}`} get={()=>mod[key]} min={mod.type==='solidify'?-100:0} max={mod.type==='subdivision'?4:mod.type==='array'?128:100} step={['levels','count'].includes(key)?1:.01} edit={{mode:'set',label:`${mod.type} ${key}`,set:(v:unknown)=>modifier(index,{[key]:v})}} />{/if}
      <button type="button" class="chip" aria-label={`Remove ${mod.type}`} onclick={()=>patchRecipe({modifiers:model.recipe.modifiers.filter((_:any,i:number)=>i!==index)})}>Remove</button>
    </Row>
    {#if mod.type==='bevel'}<Row {api} label="Segments"><NumField {api} label="Bevel segments" get={()=>mod.segments} min={1} max={12} step={1} edit={{mode:'set',label:'Bevel segments',set:(v:unknown)=>modifier(index,{segments:v})}} /></Row>{/if}
    {#if mod.type==='array'}{#each ['X','Y','Z'] as axis,i}<Row {api} label={`Offset ${axis}`}><NumField {api} label={`Array offset ${axis}`} get={()=>mod.offset[i]} step={.1} edit={{mode:'set',label:`Array offset ${axis}`,set:(v:unknown)=>modifier(index,{offset:mod.offset.map((old:number,j:number)=>j===i?v:old)})}} /></Row>{/each}{/if}
    {#if mod.type==='mirror'}{#each ['X','Y','Z'] as axis,i}<Row {api} label={`Mirror ${axis}`}><ToggleField {api} label={`Mirror ${axis}`} get={()=>mod.axes[i]} edit={{mode:'set',label:`Mirror ${axis}`,set:(v:unknown)=>modifier(index,{axes:mod.axes.map((old:boolean,j:number)=>j===i?v:old)})}} /></Row>{/each}{/if}
  {/each}
  <Row {api} label="Modifier"><button type="button" class="chip" onclick={event=>api.ui.menu(event.currentTarget as HTMLElement,defaults.map(mod=>({label:mod.type.charAt(0).toUpperCase()+mod.type.slice(1),run:()=>patchRecipe({modifiers:[...model.recipe.modifiers,mod]})})))}>Add…</button></Row>
  {/if}
  <Row {api} label="Geometry"><button type="button" class="chip" disabled={busy} onclick={regenerate}>{busy?'Generating…':model.recipe.kind==='mesh'&&!model.recipe.mesh?'Replace source…':'Regenerate'}</button>{#if busy}<button type="button" class="chip" onclick={()=>api.scene3d.cancelRender()}>Cancel</button>{/if}</Row>
  {#if model.retired.length}<Row {api} label="Hidden parts"><span class="k">{model.retired.length}</span></Row>{/if}
{/if}
