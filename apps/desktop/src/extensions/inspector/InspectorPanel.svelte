<script lang="ts">
  import { untrack } from 'svelte';
  import { inspectorSelection } from './multi-edit';
  import type { PanelProps, PowermoveAPI } from 'powermove';
  import { provideInspectorContext } from './context';
  import CompositionSection from './CompositionSection.svelte';
  import ContentSection from './ContentSection.svelte';
  import ExtensionLayerParams from './ExtensionLayerParams.svelte';
  import InspectorHeader from './InspectorHeader.svelte';
  import AlignmentStrip from './AlignmentStrip.svelte';
  import LayerOptions from './LayerOptions.svelte';
  import MasksSection from './MasksSection.svelte';
  import ShaderUniforms from './ShaderUniforms.svelte';
  import StructuredSection from './StructuredSection.svelte';
  import RetimingSection from './RetimingSection.svelte';
  import TransformSection from './TransformSection.svelte';
  import ContributedSections from './ContributedSections.svelte';
  import InspectorSurface from './InspectorSurface.svelte';
  import { inspectorRefresh } from './refresh.svelte.js';

  let { panelId, api }: PanelProps & { api: PowermoveAPI } = $props();
  const { doc, sel } = provideInspectorContext(untrack(() => api));
  let fontsVersion = $state(0);

  const selectedLayers = $derived<any[]>(
    (inspectorRefresh.version, doc.tick.structure, doc.proj,
      inspectorSelection(api, sel.layers.map((id) => api.model.layer(id)).filter((layer): layer is NonNullable<typeof layer> => layer !== null)))
  );
  const firstLayer = $derived(selectedLayers[0]);

  $effect(() => {
    const subscription = api.events.on('fonts', () => { fontsVersion++; });
    return () => subscription.dispose();
  });
</script>

<InspectorSurface {panelId}>
  {#if firstLayer}<InspectorHeader layer={firstLayer} />{/if}
  {#if firstLayer && selectedLayers.every((layer: any) => layer.type !== 'audio')}<AlignmentStrip layers={selectedLayers} />{/if}

  {#if selectedLayers.length === 0}
    <CompositionSection />
  {:else}
    {#if selectedLayers.length > 1}
      <div class="inspector-selection-note" role="status">
        {selectedLayers.length} layers selected · shared edits · drag to offset
      </div>
    {/if}
    <div class="inspector-layer" data-inspector-layer={firstLayer.id}>
      {#if firstLayer.type === 'group'}
        <div class="group-actions">
          <button class="chip" onclick={() => api.selection.select(api.groups.expand([firstLayer.id]).filter((id) => id !== firstLayer.id))}>Select contents</button>
          <button class="chip" onclick={() => api.commands.run('ungroupLayers')}>Ungroup</button>
        </div>
      {:else}
        <ContentSection layer={firstLayer} {fontsVersion} />
      {/if}
      <ContributedSections {api} after="content" layerIds={selectedLayers.map(layer => layer.id)} />
      {#if firstLayer.type !== 'audio'}<TransformSection layer={firstLayer} />{/if}
      <ContributedSections {api} after="transform" layerIds={selectedLayers.map(layer => layer.id)} />
      <StructuredSection layer={firstLayer} />
      {#if firstLayer.type !== 'audio'}
        {#if firstLayer.type === 'shader'}<ShaderUniforms layer={firstLayer} />{/if}
        {#if firstLayer.type === 'extension'}<ExtensionLayerParams layer={firstLayer} />{/if}
        <ContributedSections {api} after="effects" layerIds={selectedLayers.map(layer => layer.id)} />
        <MasksSection layer={firstLayer} />
      {/if}
      {#if firstLayer.type === 'video' || firstLayer.type === 'precomp'}<RetimingSection layer={firstLayer} />{/if}
      <LayerOptions layer={firstLayer} />
    </div>
  {/if}
</InspectorSurface>

<style>
  .group-actions { display:flex; gap:8px; padding:12px 16px 0; }
  .inspector-layer {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding-bottom: 12px;
  }

  /* Match Motioner's section rhythm: each property family starts on a clear,
     full-width rule instead of reading as one continuous list. */
  .inspector-layer > :global(.sec) {
    margin-top: 8px;
    border-top-color: var(--section-line);
  }

  .inspector-layer > :global(.sec:first-child) {
    margin-top: 0;
  }

  .inspector-selection-note {
    padding: 6px 16px;
    margin: 0 calc(-1 * var(--pad));
    color: var(--tx-3);
    font-size: var(--fs-xs);
  }

</style>
