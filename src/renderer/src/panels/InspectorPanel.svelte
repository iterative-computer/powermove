<script lang="ts">
  import { doc } from '../state/document.svelte';
  import { sel } from '../state/selection.svelte';
  import type { PanelProps } from './registerSveltePanel';
  import CompositionSection from './inspector/CompositionSection.svelte';
  import ContentSection from './inspector/ContentSection.svelte';
  import EffectsSection from './inspector/EffectsSection.svelte';
  import InspectorHeader from './inspector/InspectorHeader.svelte';
  import LayerOptions from './inspector/LayerOptions.svelte';
  import MasksSection from './inspector/MasksSection.svelte';
  import ShaderUniforms from './inspector/ShaderUniforms.svelte';
  import TransformSection from './inspector/TransformSection.svelte';
  import { getInspectorRegistry, inspectorRefresh } from './inspector/refresh.svelte';

  let { panelId }: PanelProps = $props();
  const PM = getInspectorRegistry();
  let fontsVersion = $state(0);

  const selectedLayers = $derived<any[]>(
    (inspectorRefresh.version, doc.tick.structure, doc.proj,
      sel.layers.map((id) => doc.proj?.layers?.find((layer: any) => layer.id === id)).filter(Boolean))
  );
  const firstLayer = $derived(selectedLayers[0]);

  /* Phase 5 removal: fonts have no document tick yet, so this is the sole bus
     subscription in the Svelte inspector. */
  $effect(() => {
    const off = PM.bus?.on?.('fonts', () => { fontsVersion++; });
    return () => { if (typeof off === 'function') off(); };
  });
</script>

<div class="insp" data-svelte-panel={panelId} data-inspector-refresh={inspectorRefresh.version}>
  <InspectorHeader {PM} layer={firstLayer} />

  {#if selectedLayers.length === 0}
    <CompositionSection {PM} />
  {:else}
    {#if selectedLayers.length > 1}
      <div class="inspector-selection-note" role="status">
        {selectedLayers.length} layers selected · editing {firstLayer.name}
      </div>
    {/if}
    <div class="inspector-layer" data-inspector-layer={firstLayer.id}>
      <ContentSection {PM} layer={firstLayer} {fontsVersion} />
      {#if firstLayer.type !== 'audio'}
        <TransformSection {PM} layer={firstLayer} />
        {#if firstLayer.type === 'shader'}<ShaderUniforms {PM} layer={firstLayer} />{/if}
        <EffectsSection {PM} layer={firstLayer} />
        <MasksSection {PM} layer={firstLayer} />
      {/if}
      <LayerOptions {PM} layer={firstLayer} />
    </div>
  {/if}
</div>

<style>
  .inspector-layer {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .inspector-layer > :global(.sec:first-child) {
    margin-top: 2px;
  }

  .inspector-selection-note {
    padding: 4px 6px;
    color: var(--tx-3);
    font-size: var(--fs-xs);
  }
</style>
