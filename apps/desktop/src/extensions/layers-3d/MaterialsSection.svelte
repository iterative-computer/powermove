<script module lang="ts">
  /** Which materials are open, per model, for the session. */
  const opened = new Map<string, boolean>();
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import type { Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import MaterialProperty from './MaterialProperty.svelte';
  import { openMaterialPicker } from './material-picker';
  import {
    MATERIAL_PROPERTY_SPECS, MATERIAL_SLOT_SPECS, assignMap, describeMap, materialEntries, materialKey, overrideCount,
    resetMaterial, slotSpec, statusNote, type MaterialEntry, type MaterialSlot
  } from './materials';

  /* Materials, as in After Effects 27's Layers as Materials: each of the
     model's materials by name, its slots with a source pop-up (a layer in
     this comp, a comp or an image — or Original), then the material's
     keyframeable options. Drop a comp or image from the Project panel on a
     slot to assign it. */
  let { api, ui, layer }: { api: PowermoveAPI; ui: SceneUiState; layer: Layer } = $props();
  const { Section, Row, Disclosure } = untrack(() => api.ui.controls);

  const entries = $derived((ui.version, materialEntries(api, layer.id)));
  const LAYER_SLOTS = MATERIAL_SLOT_SPECS.filter(spec => spec.layers);
  const IMAGE_SLOTS = MATERIAL_SLOT_SPECS.filter(spec => !spec.layers);
  let dropTarget = $state<string | null>(null);
  let openVersion = $state(0);

  const isOpen = (entry: MaterialEntry, position: number): boolean => { void openVersion; return opened.get(materialKey(layer.id, entry.index)) ?? position === 0; };
  function toggle(entry: MaterialEntry, position: number): void {
    opened.set(materialKey(layer.id, entry.index), !isOpen(entry, position));
    openVersion++;
  }

  function slotDrop(event: DragEvent, entry: MaterialEntry, slot: MaterialSlot): void {
    event.preventDefault();
    dropTarget = null;
    const payload = api.dnd.readAssetDrag(event.dataTransfer);
    if (!payload) return;
    const spec = slotSpec(slot);
    if (payload.kind === 'comp') {
      if (!spec.layers) { api.ui.toast(`${spec.label} takes an image. Drop the comp on Base Color or Emissive.`, { key: 'material-assign' }); return; }
      if (!api.scene3d.listValidCompositions(layer.id).some(comp => comp.id === payload.id)) { api.ui.toast(`${payload.name} contains this model, so using it would loop.`, { key: 'material-assign' }); return; }
      assignMap(api, layer.id, entry.index, slot, { compositionId: payload.id });
    } else if (payload.kind === 'image') assignMap(api, layer.id, entry.index, slot, payload.id);
    else api.ui.toast(spec.layers ? 'Add the clip to the timeline first, then choose its layer here.' : `${spec.label} takes an image.`, { key: 'material-assign' });
  }
  function slotOver(event: DragEvent, id: string): void {
    if (!api.dnd.hasAssetDrag(event.dataTransfer)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
    dropTarget = id;
  }
</script>

{#snippet slotRow(entry: MaterialEntry, slot: MaterialSlot)}
  {@const spec = slotSpec(slot)}
  {@const info = describeMap(api, layer.id, entry.index, slot, entry.maps[slot])}
  {@const note = statusNote(info.status)}
  {@const id = `${entry.index ?? 'all'}:${slot}`}
  <div class="mat-slot" class:drop={dropTarget === id} title={spec.tip} data-material-slot={id}
    role="group" aria-label={`${entry.name} ${spec.label}`}
    ondragover={(event) => slotOver(event, id)} ondragleave={() => { if (dropTarget === id) dropTarget = null; }}
    ondrop={(event) => slotDrop(event, entry, slot)}>
    <Row {api} label={spec.label}>
      <button type="button" class="pm-select sel mat-source" class:original={info.status === 'original'} class:warn={!!note}
        aria-haspopup="listbox" aria-expanded="false" aria-label={`${spec.label} source: ${info.label}`}
        data-tip={note ?? ''}
        onclick={(event) => openMaterialPicker(api, event.currentTarget as HTMLElement, { layerId: layer.id, materialIndex: entry.index, slot })}>
        {#if note}<span class="mat-source-icon warn" aria-hidden="true">{@html api.ui.icon('warning')}</span>
        {:else if info.icon}<span class="mat-source-icon" aria-hidden="true">{@html api.ui.icon(info.icon)}</span>
        {:else}<span class="mat-source-icon mat-ball" aria-hidden="true"></span>{/if}
        <span class="pm-select-label">{note ? `${info.label} · using original` : info.label}</span>
        <svg class="pm-select-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 6.25 3.5 3.5 3.5-3.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
    </Row>
  </div>
{/snippet}

{#if entries.length}
  <Section {api} title="Materials" />
  <div class="materials" data-materials-section>
    {#each entries as entry, position (entry.index ?? 'all')}
      {@const open = isOpen(entry, position)}
      {@const count = overrideCount(api, layer.id, entry)}
      {@const base = describeMap(api, layer.id, entry.index, 'color', entry.maps.color)}
      {@const bodyId = `material-${layer.id}-${entry.index ?? 'all'}`}
      <div class="mat" data-material-index={entry.index ?? 'all'}>
        <div class="row mat-head" class:open>
          <button type="button" class="mat-twirl" aria-expanded={open} aria-controls={bodyId} aria-label={`${open ? 'Collapse' : 'Expand'} ${entry.name}`} onclick={() => toggle(entry, position)}>
            <span class="twirl" aria-hidden="true">{@html api.ui.icon(open ? 'chevD' : 'chev')}</span>
            <span class="mat-name">{entry.name}</span>
          </button>
          {#if !open}<span class="mat-summary" class:quiet={base.status === 'original'}>{base.status === 'original' ? (count ? `${count} change${count === 1 ? '' : 's'}` : base.label) : base.label}</span>{/if}
          {#if open && count}<button type="button" class="mat-reset" title={`Put ${entry.name} back to the model’s original material`} onclick={() => resetMaterial(api, layer.id, entry.index)}>Reset</button>{/if}
        </div>
        {#if open}
          <div class="mat-body" id={bodyId}>
            {#each LAYER_SLOTS as spec (spec.id)}{@render slotRow(entry, spec.id)}{/each}
            <Disclosure label="Image Maps" count={IMAGE_SLOTS.filter(spec => entry.maps[spec.id] != null).length || undefined} remember={`material-maps:${materialKey(layer.id, entry.index)}`}>
              {#each IMAGE_SLOTS as spec (spec.id)}{@render slotRow(entry, spec.id)}{/each}
            </Disclosure>
            <div class="mat-options" role="group" aria-label={`${entry.name} options`}>
              {#each MATERIAL_PROPERTY_SPECS as spec (spec.key)}<MaterialProperty {api} {ui} {layer} index={entry.index} {spec} />{/each}
            </div>
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  /* Material headers follow Effect Controls' effect headers (30px, tinted
     while open); slots and options are 30px rows with a flexible label and a
     fixed field, so long names like "Emission Intensity" never clip. */
  .materials { display: flex; flex-direction: column; margin: 0 calc(8px - var(--pad)) 4px; }
  .mat + .mat { margin-top: 2px; }
  .mat-head { height: 30px; margin: 0; padding: 0 8px 0 6px; gap: 8px; border-radius: 7px; }
  .mat-head.open { background: var(--bg-row); }
  .mat-twirl { display: flex; align-items: center; gap: 8px; flex: 1; min-width: 0; height: 100%; padding: 0; border: 0; background: none; color: var(--tx); font: inherit; text-align: left; cursor: default; }
  .mat-twirl:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; border-radius: 5px; }
  .twirl { display: grid; place-items: center; width: 10px; height: 10px; flex: none; color: var(--tx-2); }
  .twirl :global(svg) { width: 10px; height: 10px; }
  .mat-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: var(--fw-semibold); line-height: 16px; }
  .mat-summary { flex: none; max-width: 50%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--tx-2); font-size: 12px; }
  .mat-summary.quiet { color: var(--tx-3); }
  .mat-reset { flex: none; padding: 0; border: 0; background: transparent; color: var(--tx-3); font-size: 12px; line-height: 16px; }
  .mat-reset:hover { color: var(--tx); }
  .mat-body { padding: 2px 0 6px; }
  .mat-body :global(.row.split) { height: 30px; margin: 0; padding: 0 6px 0 22px; gap: 8px; }
  .mat-body :global(.row.split > .k) { flex: 1; width: auto; min-width: 0; padding-left: 0; color: var(--tx-label); font-size: 12px; line-height: 16px; }
  .mat-body :global(.row.split:not(:has(> .stopwatch)) > .k) { padding-left: 21px; }
  .mat-body :global(.row.split > .stopwatch) { width: 13px; height: 24px; background: none; }
  .mat-body :global(.row.split > .stopwatch svg) { width: 13px; height: 13px; }
  .mat-body :global(.row.split > .vwrap) { flex: 0 0 114px; width: 114px; height: 24px; }
  .mat-body :global(.row.split :is(.num, .color-field)) { height: 24px; border-radius: 6px; font-size: 12px; }
  .mat-body :global(.row.split:has(> .stopwatch.on) .num) { color: var(--accent-tx); }
  .mat-body :global(.control-disclosure) { margin: 0; }
  .mat-body :global(.control-disclosure > summary) { padding: 7px 6px 7px 18px; font-size: 12px; color: var(--tx-label); }
  .mat-options { margin-top: 2px; padding-top: 4px; border-top: 1px solid var(--line); margin-left: 22px; }
  .mat-options :global(.row.split) { padding-left: 0; }

  .mat-slot { border-radius: 7px; transition: box-shadow var(--dur-1); }
  .mat-slot.drop { box-shadow: inset 0 0 0 1.5px var(--accent); background: var(--accent-dim); }
  .mat-source { width: 100%; height: 24px; display: flex; align-items: center; gap: 6px; padding: 0 22px 0 7px; border: 0; border-radius: 6px; background: var(--bg-field); color: var(--tx-value); font-family: var(--f-ui); font-size: 12px; }
  .mat-source:hover { background: color-mix(in srgb, var(--bg-field) 80%, var(--tx) 6%); }
  .mat-source.original { color: var(--tx-3); }
  .mat-source.warn { color: var(--warning); }
  .mat-source :global(.pm-select-chevron) { right: 6px; width: 10px; height: 10px; margin-top: -5px; }
  .mat-source-icon { display: grid; place-items: center; width: 13px; height: 13px; flex: none; color: var(--tx-2); }
  .mat-source-icon :global(svg) { width: 13px; height: 13px; }
  .mat-source-icon.warn { color: var(--warning); }
  .mat-ball::before { content: ''; width: 10px; height: 10px; border-radius: 50%; background: radial-gradient(circle at 35% 30%, rgb(255 255 255 / .95), rgb(var(--ink-rgb) / .32) 72%); box-shadow: inset 0 0 0 .5px rgb(var(--ink-rgb) / .28); }
</style>
