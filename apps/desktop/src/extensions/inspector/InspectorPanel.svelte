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
  import LayerOptions from './LayerOptions.svelte';
  import MasksSection from './MasksSection.svelte';
  import ShaderUniforms from './ShaderUniforms.svelte';
  import StructuredSection from './StructuredSection.svelte';
  import RetimingSection from './RetimingSection.svelte';
  import TransformSection from './TransformSection.svelte';
  import { inspectorRefresh } from './refresh.svelte.js';

  let { panelId, api }: PanelProps & { api: PowermoveAPI } = $props();
  const { PM, doc, sel } = provideInspectorContext(untrack(() => api));
  let fontsVersion = $state(0);

  const selectedLayers = $derived<any[]>(
    (inspectorRefresh.version, doc.tick.structure, doc.proj,
      inspectorSelection(PM, sel.layers.map((id) => doc.proj?.layers?.find((layer: any) => layer.id === id)).filter(Boolean)))
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
        {selectedLayers.length} layers selected · shared edits · drag to offset
      </div>
    {/if}
    <div class="inspector-layer" data-inspector-layer={firstLayer.id}>
      {#if firstLayer.type === 'group'}
        <div class="group-actions">
          <button class="chip" onclick={() => PM.selectLayers(PM.expandGroups([firstLayer.id]).filter((id: string) => id !== firstLayer.id))}>Select contents</button>
          <button class="chip" onclick={() => PM.cmd('ungroupLayers')}>Ungroup</button>
        </div>
      {:else}
        <ContentSection {PM} layer={firstLayer} {fontsVersion} />
      {/if}
      {#if firstLayer.type !== 'audio'}<TransformSection {PM} layer={firstLayer} />{/if}
      <StructuredSection {PM} layer={firstLayer} />
      {#if firstLayer.type !== 'audio'}
        {#if firstLayer.type === 'shader'}<ShaderUniforms {PM} layer={firstLayer} />{/if}
        {#if firstLayer.type === 'extension'}<ExtensionLayerParams {PM} layer={firstLayer} />{/if}
        <EffectsSection {PM} layer={firstLayer} />
        <MasksSection {PM} layer={firstLayer} />
      {/if}
      {#if firstLayer.type === 'video' || firstLayer.type === 'precomp'}<RetimingSection {PM} layer={firstLayer} />{/if}
      <LayerOptions {PM} layer={firstLayer} />
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
  .insp :global(.row.split:has(textarea)) { height: auto; min-height: 54px; align-items: start; }
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
