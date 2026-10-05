<script lang="ts">
  import { inspectorContext } from './context';
  import ParameterList from './ParameterList.svelte';

  const { api, doc, inspector } = inspectorContext();
  const { Section } = api.ui.controls;

  let { layer }: { layer: any } = $props();

  let definitions = $state<any[]>([]);

  $effect(() => {
    doc.tick.structure;
    doc.proj;
    layer.d?.code;
    const service = inspector();
    service?.syncShaderUniforms(layer as any);
    definitions = service?.shaderDefinitions(layer as any) ?? [];
  });


</script>

{#if definitions.length}
  <Section {api} title="Shader" />
  <ParameterList {layer} prefix="u" properties={layer.d?.uniforms ?? {}} params={definitions.map(d=>({k:d.name,label:d.label,def:d.def,type:d.control==='color'?'color':d.control==='toggle'?'toggle':'number',min:d.min,max:d.max,step:(d.max-d.min)/200||.01}))} />
{/if}
