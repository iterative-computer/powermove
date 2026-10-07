<script lang="ts">
  import {untrack,onDestroy} from 'svelte';
  import type {PowermoveAPI} from 'powermove';
  import type {SceneUiState} from './scene-state.svelte';
  let {api,ui}:{api:PowermoveAPI;ui:SceneUiState}=$props();
  const {Section,Row,SelectField,NumField,ToggleField}=untrack(()=>api.ui.controls);
  const settings=$derived((ui.version,api.scene3d.getRendering()));
  let preview=$state(untrack(()=>api.scene3d.previewState()));
  const subscription=untrack(()=>api.scene3d.onPreviewChange(value=>preview=value));onDestroy(()=>subscription.dispose());
  let status=$state<{available:boolean;version?:string;error?:string}|null>(null);
  void untrack(()=>api.scene3d.blenderStatus().then(value=>status=value));
  function edit(key:string,label:string){return {mode:'set' as const,label,set:(value:unknown)=>{const result=api.scene3d.setRendering({[key]:value});if(!result.ok)api.ui.toast(result.message,{error:true});}};}
  async function render(){try{api.scene3d.setPreviewMode('rendered');await api.scene3d.renderFrame();}catch(error){api.ui.toast(error instanceof Error?error.message:String(error),{error:true});}}
</script>
<Section {api} title="Rendering" />
<Row {api} label="Engine"><SelectField {api} label="3D render engine" get={()=>settings.enabled?settings.engine:'native'} options={[{v:'native',label:'Native'},{v:'eevee',label:'EEVEE'},{v:'cycles',label:'Cycles'}]} edit={{mode:'set',label:'3D render engine',set:(value:unknown)=>{const result=api.scene3d.setRendering({enabled:value!=='native',...(value!=='native'?{engine:value as 'eevee'|'cycles'}:{})});if(!result.ok)api.ui.toast(result.message,{error:true});}}} /></Row>
{#if settings.enabled}
  <Row {api} label="Samples"><NumField {api} label="Render samples" get={()=>settings.samples} edit={edit('samples','Render samples')} min={1} max={4096} step={1} /></Row>
  <Row {api} label="Preview samples"><NumField {api} label="Preview samples" get={()=>settings.previewSamples} edit={edit('previewSamples','Preview samples')} min={1} max={512} step={1} /></Row>
  <Row {api} label="Preview size"><SelectField {api} label="3D preview size" get={()=>settings.previewScale} edit={edit('previewScale','3D preview size')} options={[{v:.25,label:'Quarter'},{v:.5,label:'Half'},{v:1,label:'Full'}]} /></Row>
  {#if settings.engine==='cycles'}
    <Row {api} label="Device"><SelectField {api} label="Cycles device" get={()=>settings.device} edit={edit('device','Cycles device')} options={[{v:'auto',label:'Automatic'},{v:'cpu',label:'CPU'},{v:'gpu',label:'GPU'}]} /></Row>
    <Row {api} label="Denoise"><ToggleField {api} label="Denoise" get={()=>settings.denoise} edit={edit('denoise','Denoise')} /></Row>
  {/if}
{/if}
{#if status && !status.available}
  <Row {api} label="Blender"><button type="button" class="chip" onclick={async()=>{try{status=await api.scene3d.chooseBlender();}catch(error){api.ui.toast(String(error),{error:true});}}}>Choose Blender…</button></Row>
{:else}
  <Row {api} label="Preview"><button type="button" class="chip" disabled={preview.busy} onclick={render}>{preview.busy?'Rendering…':'Render frame'}</button>{#if preview.busy}<button type="button" class="chip" onclick={()=>api.scene3d.cancelRender()}>Cancel</button>{/if}</Row>
{/if}
{#if preview.error}<Row {api} label="Render"><span class="k" role="status">{preview.error}</span></Row>{/if}
