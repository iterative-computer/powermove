<script lang="ts">
  import { inspectorContext } from './context';
  import AnimatedRow from './AnimatedRow.svelte';
  import ChannelRow from './ChannelRow.svelte';
  import { enableTimeRemap } from 'powermove';
  import { evaluatedValue } from 'powermove';
  const { api,doc,transport }=inspectorContext();
  const { Section,ToggleField }=api.ui.controls;
  let {PM,layer}: {PM:any;layer:any}=$props();
  const enabled=$derived((doc.tick.values,doc.proj,transport.time,evaluatedValue(PM,layer,layer.d.timeRemap,transport.time,'c.timeRemap')));
</script>
<Section title="Time" />
<div style="display:flex;gap:6px;flex-wrap:wrap">
  <button class="chip" onclick={()=>enableTimeRemap(PM,layer)}>Time remap</button>
  <button class="chip" onclick={()=>enableTimeRemap(PM,layer,'freeze')}>Freeze frame</button>
  <button class="chip" onclick={()=>enableTimeRemap(PM,layer,'reverse')}>Reverse</button>
</div>
{#if layer.d.sourceTime}
  <AnimatedRow {PM} {layer} path="c.timeRemap" label="Time remap">
    <ToggleField {PM} get={()=>!!enabled} label="Time remap" edit={{mode:'command',label:'Time remap',origin:'inspector',command:(value:any)=>({type:'set_property',target:layer.id,path:'c.timeRemap',value:!!value,preserveHandEdits:false})}} />
  </AnimatedRow>
  <ChannelRow {PM} {layer} channel="c.sourceTime" property={layer.d.sourceTime} label="Source time" step={1/PM.proj.fps} unit="s" precision={3} />
  <p style="font-size:11px;color:var(--tx-3)">Remapped audio is muted. Separate the soundtrack to edit its timing independently.</p>
{/if}
