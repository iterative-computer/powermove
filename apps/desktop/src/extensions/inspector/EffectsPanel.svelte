<script lang="ts">
  import { untrack } from 'svelte';
  import type { PanelProps, PowermoveAPI } from 'powermove';
  import { provideInspectorContext } from './context';
  import EffectsSection from './EffectsSection.svelte';
  import InspectorSurface from './InspectorSurface.svelte';
  import { inspectorSelection } from './multi-edit';
  import { inspectorRefresh } from './refresh.svelte.js';

  /* The applied-effects stack for the selected layer, split out of Properties
     so it can be docked on its own. Edits share the inspector's multi-edit
     context, exactly as they did when the stack lived inside Properties. */
  let { panelId, api }: PanelProps & { api: PowermoveAPI } = $props();
  const { doc, sel } = provideInspectorContext(untrack(() => api));
  const meta = untrack(() => api.model.TYPE_META) as Record<string, { label?: string; effects?: boolean }>;

  const selectedLayers = $derived<any[]>(
    (inspectorRefresh.version, doc.tick.structure, doc.proj,
      inspectorSelection(api, sel.layers.map((id) => api.model.layer(id)).filter((layer): layer is NonNullable<typeof layer> => layer !== null)))
  );
  const firstLayer = $derived(selectedLayers[0]);
</script>

<InspectorSurface {panelId}>
  {#if !firstLayer}
    <div class="empty">Select a layer to see its effects.</div>
  {:else if meta[firstLayer.type]?.effects === false}
    <div class="empty">{meta[firstLayer.type]?.label ?? 'This'} layers don’t take effects.</div>
  {:else}
    <div class="row inspector-header" data-effects-header>
      <div class="k"><span>{(doc.tick.structure, firstLayer.name)}</span></div>
    </div>
    {#if selectedLayers.length > 1}
      <div class="effects-selection-note" role="status">
        {selectedLayers.length} layers selected · showing {firstLayer.name || 'the first layer'}
      </div>
    {/if}
    {#key firstLayer.id}
      <div class="effects-layer" data-effects-layer={firstLayer.id}>
        <EffectsSection layer={firstLayer} />
      </div>
    {/key}
  {/if}
</InspectorSurface>

<style>
  .effects-layer { padding-bottom: 12px; }

  .effects-selection-note {
    padding: 0 16px 6px;
    margin: 0 calc(-1 * var(--pad));
    color: var(--tx-3);
    font-size: var(--fs-xs);
  }
</style>
