<script lang="ts">
  import { inspectorContext } from './context';
  import AnimatedRow from './AnimatedRow.svelte';
  import ChannelRow from './ChannelRow.svelte';
  import { enableTimeRemap } from './retiming';
  import { evaluatedValue } from './multi-edit';
  const { api,doc,transport,mixed }=inspectorContext();
  const { Section,ToggleField }=api.ui.controls;
  let {layer}: {layer:any}=$props();
  const enabled=$derived((doc.tick.values,doc.proj,transport.time,evaluatedValue(api,layer,layer.d.timeRemap,transport.time,'c.timeRemap')));
</script>
<Section {api} title="Time" />
<div style="display:flex;gap:6px;flex-wrap:wrap">
  <button class="chip" onclick={()=>enableTimeRemap(api,layer)}>Time remap</button>
  <button class="chip" onclick={()=>enableTimeRemap(api,layer,'freeze')}>Freeze frame</button>
  <button class="chip" onclick={()=>enableTimeRemap(api,layer,'reverse')}>Reverse</button>
</div>
{#if layer.d.sourceTime}
  <AnimatedRow {layer} path="c.timeRemap" label="Time remap">
    <ToggleField {api} {mixed} get={()=>!!enabled} label="Time remap" edit={{mode:'command',label:'Time remap',origin:'inspector',command:(value:any)=>({type:'set_property',target:layer.id,path:'c.timeRemap',value:!!value,preserveHandEdits:false})}} />
  </AnimatedRow>
  <ChannelRow {layer} channel="c.sourceTime" property={layer.d.sourceTime} label="Source time" step={1/api.project.get().fps} unit="s" precision={3} />
  <p style="font-size:11px;color:var(--tx-3)">Remapped audio is muted. Separate the soundtrack to edit its timing independently.</p>
{/if}
