<script lang="ts">
  import { untrack } from 'svelte';
  import type { PanelProps, PowermoveAPI } from 'powermove';
  import { provideInspectorContext } from './context';
  import CompositionSection from './CompositionSection.svelte';
  import ContentSection from './ContentSection.svelte';
  import EffectsSection from './EffectsSection.svelte';
  import ExtensionLayerParams from './ExtensionLayerParams.svelte';
  import InspectorHeader from './InspectorHeader.svelte';
  import LayerOptions from './LayerOptions.svelte';
  import MasksSection from './MasksSection.svelte';
  import ShaderUniforms from './ShaderUniforms.svelte';
  import TransformSection from './TransformSection.svelte';
  import { inspectorRefresh } from './refresh.svelte.js';

  let { panelId, api }: PanelProps & { api: PowermoveAPI } = $props();
  const { PM, doc, sel } = provideInspectorContext(untrack(() => api));
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
  {#if firstLayer}<InspectorHeader {PM} layer={firstLayer} />{/if}

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
        {#if firstLayer.type === 'extension'}<ExtensionLayerParams {PM} layer={firstLayer} />{/if}
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
    gap: 0;
    padding-bottom: 12px;
  }

  /* Sections are full-bleed; the panel supplies the horizontal inset the
     section rules subtract back out. */
  .inspector-layer > :global(.sec) {
    margin-top: 12px;
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

  .insp :global(.chip.wide) {
    margin: 0 4px 4px;
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
