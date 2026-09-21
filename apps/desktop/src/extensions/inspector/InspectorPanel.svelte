<script lang="ts">
  import { untrack } from 'svelte';
  import { inspectorSelection } from './multi-edit';
  import type { PanelProps, PowermoveAPI } from 'powermove';
  import { provideInspectorContext } from './context';
  import CompositionSection from './CompositionSection.svelte';
  import ContentSection from './ContentSection.svelte';
  import EffectsSection from './EffectsSection.svelte';
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

<div class="insp" data-svelte-panel={panelId} data-inspector-refresh={inspectorRefresh.version}>
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
        <EffectsSection layer={firstLayer} />
        <ContributedSections {api} after="effects" layerIds={selectedLayers.map(layer => layer.id)} />
        <MasksSection layer={firstLayer} />
      {/if}
      {#if firstLayer.type === 'video' || firstLayer.type === 'precomp'}<RetimingSection layer={firstLayer} />{/if}
      <LayerOptions layer={firstLayer} />
    </div>
  {/if}
</div>

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

  .insp {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 0 var(--pad) 24px;
  }

  .insp :global(.row.split .k) {
    width: 90px;
    font-size: 11px;
    line-height: 1.2;
  }
  .insp :global(.row.split) { gap: 6px; }
  /* Tall rows (the text field) keep their label and stopwatch on the same
     28px band as every single-line row, so the diamond and the title stay
     centered on each other while the field grows below. */
  .insp :global(.row.split:has(textarea)) { height: auto; min-height: 54px; align-items: start; }
  .insp :global(.row.split:has(textarea) > .k) { height: 28px; }
  .insp :global(.row.split:has(textarea) > .stopwatch) { margin-top: 5px; }
  /* Rows without a keyframe diamond keep their label at the same x, but the
     control column starts where it does on animated rows (18px diamond + 8px
     gap), so every well in a section shares one left edge. */
  .insp :global(.row.split:not(:has(> .stopwatch)) > .k) { width: 114px; }
  .insp :global(.property-stopwatch) { width: 18px; height: 24px; }

  /* Panel actions use the same flat material as property fields. */
  .insp :global(.chip) {
    min-width: 0;
    max-width: 100%;
    height: var(--ctl-h);
    background: var(--bg-field);
    box-shadow: none;
    white-space: normal;
  }

  .insp :global(.chip:hover) { background: var(--bg-row-hi); color: var(--tx); }
  .insp :global(.chip:active) { background: var(--bg-row); transform: none; }
  .insp :global(.chip:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .insp :global(.chip:disabled) { opacity: var(--disabled); pointer-events: none; }
  .insp :global(.chip.ghost) { background: transparent; }
  .insp :global(.chip.ghost:hover) { background: var(--ink-1); }
  .insp :global(.chip.solid) { background: var(--accent); color: var(--on-accent); }
  .insp :global(.chip.solid:hover) { background: var(--accent-hover); }

  .insp :global(.chip.wide) {
    width: 100%;
    margin: 0 0 4px;
  }

  .insp :global(.stopwatch) {
    display: grid;
    flex: none;
    width: 18px;
    height: 18px;
    place-items: center;
    border-radius: var(--r-xs);
    color: var(--tx-4);
    transition: background var(--dur-1), color var(--dur-1);
  }

  .insp :global(.stopwatch:hover) {
    background: var(--ink-1);
    color: var(--tx-2);
  }

  .insp :global(.stopwatch.on) {
    color: var(--accent);
  }

  .insp :global(.stopwatch svg) {
    width: 11px;
    height: 11px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
  }

  .insp :global(.property-stopwatch.at-key svg) { fill: currentColor; }

  .insp :global(.stopwatch.property-stopwatch svg) { width: 14px; height: 14px; stroke-width: 1.5; }

  .insp :global(.kf i) { width: 5px; height: 5px; }

  .insp :global(.grp) {
    margin-left: 0;
  }

  .insp :global(.grp .row) {
    margin-left: 12px;
  }

  .insp :global(.twirl) {
    display: grid;
    flex: none;
    width: 14px;
    height: 14px;
    place-items: center;
    color: var(--tx-3);
    transition: transform var(--dur-2) var(--ease-io);
  }

  .insp :global(.twirl.open) {
    transform: rotate(90deg);
  }

  .insp :global(.empty) {
    padding: 28px 16px;
    color: var(--tx-3);
    font-size: var(--fs-sm);
    line-height: var(--lh);
    text-align: center;
    text-wrap: pretty;
  }

  .insp :global(.inspector-header) {
    height: 48px;
    margin: 0 calc(-1 * var(--pad)) 8px;
    padding: 0 12px 0 16px;
    border-bottom: 0;
    border-radius: 0;
    background: transparent;
  }
  .insp :global(.inspector-header .k) { font-size: 12px; }

  .insp :global(.inspector-header:hover) {
    background: transparent;
  }

  .insp :global(.inspector-header .k) {
    color: var(--tx);
    font-weight: var(--fw-medium);
  }

  .insp :global(.inspector-header .k .sub) {
    color: var(--tx-3);
    font-weight: var(--fw-regular);
  }

  .insp :global(.inspector-header .iconbtn) {
    width: 24px;
    height: 24px;
    color: var(--tx-3);
  }
</style>
