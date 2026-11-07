<script module lang="ts">
  /** Which surfaces are open, per layer, for the session. */
  const opened = new Map<string, boolean>();
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import type { Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import SurfaceProperty from './SurfaceProperty.svelte';
  import MaterialThumb from './MaterialThumb.svelte';
  import { layerDrop } from './layer-drop.svelte';
  import { openMaterialPicker } from './material-picker';
  import { MATERIAL_SLOT_SPECS, assignMap, describeMap, slotSpec, statusNote, type MaterialEntry, type MaterialSlot } from './materials';
  import {
    SURFACES, SURFACE_PROPERTY_SPECS, SURFACE_TIPS, resetSurface, surfaceAt, surfaceEntries, surfaceOverrideCount,
    surfacesMatchFront, useFrontForAll
  } from './extrusion';

  /* Material Options for an extruded layer: Front, Bevel, Side and Back, each
     laid out like a model's material in the Materials section — source pop-ups
     (a layer in this comp, a comp or an image), image-only maps, then Color and
     the six keyframeable options. "Use Front for All" copies Front onto the rest. */
  let { api, ui, layer }: { api: PowermoveAPI; ui: SceneUiState; layer: Layer } = $props();
  const { Section, Row, Disclosure } = untrack(() => api.ui.controls);

  const entries = $derived((ui.version, surfaceEntries(api, layer.id)));
  const matching = $derived((ui.version, surfacesMatchFront(api, layer.id)));
  const LAYER_SLOTS = MATERIAL_SLOT_SPECS.filter(spec => spec.layers);
  const IMAGE_SLOTS = MATERIAL_SLOT_SPECS.filter(spec => !spec.layers);
  let dropTarget = $state<string | null>(null);
  let openVersion = $state(0);

  const key = (entry: MaterialEntry) => `${layer.id}:${entry.index}`;
  const isOpen = (entry: MaterialEntry): boolean => { void openVersion; return opened.get(key(entry)) ?? entry.index === 0; };
  function toggle(entry: MaterialEntry): void { opened.set(key(entry), !isOpen(entry)); openVersion++; }

  function slotDrop(event: DragEvent, entry: MaterialEntry, slot: MaterialSlot): void {
    event.preventDefault();
    dropTarget = null;
    const payload = api.dnd.readAssetDrag(event.dataTransfer);
    if (!payload) return;
    const spec = slotSpec(slot);
    if (payload.kind === 'comp') {
      if (!spec.layers) { api.ui.toast(`${spec.label} takes an image. Drop the comp on Base Color or Emissive.`, { key: 'material-assign' }); return; }
      if (!api.scene3d.listValidCompositions(layer.id).some(comp => comp.id === payload.id)) { api.ui.toast(`${payload.name} contains this layer, so using it would loop.`, { key: 'material-assign' }); return; }
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
  {@const id = `${entry.index}:${slot}`}
  {@const hover = layerDrop.hover && layerDrop.hover.layerId === layer.id && layerDrop.hover.materialIndex === entry.index && layerDrop.hover.slot === slot ? layerDrop.hover : null}
  <!-- data-material-* address the slot for layer drags from the Timeline, as in the Materials section. -->
  <div class="mat-slot" class:is-drop={dropTarget === id || hover?.ok} class:is-refused={hover && !hover.ok} title={hover && !hover.ok ? hover.reason : spec.tip.replace('whole material', 'whole surface')}
    data-material-slot={id} data-material-layer={layer.id} data-material-slot-index={entry.index} data-material-slot-name={slot}
    data-layer-drop={hover ? (hover.ok ? 'valid' : 'invalid') : undefined}
    role="group" aria-label={`${entry.name} ${spec.label}`}
    ondragover={(event) => slotOver(event, id)} ondragleave={() => { if (dropTarget === id) dropTarget = null; }}
    ondrop={(event) => slotDrop(event, entry, slot)}>
    <Row {api} label={spec.label}>
      <button type="button" class="pm-select sel mat-source" class:original={info.status === 'original'} class:warn={!!note}
        aria-haspopup="listbox" aria-expanded="false" aria-label={`${entry.name} ${spec.label} source: ${info.label}`}
        data-tip={note ?? ''}
        onclick={(event) => openMaterialPicker(api, event.currentTarget as HTMLElement, { layerId: layer.id, materialIndex: entry.index, slot })}>
        {#if note}<span class="mat-source-icon warn" aria-hidden="true">{@html api.ui.icon('warning')}</span>
        {:else if info.status === 'ready'}
          <MaterialThumb {api} layerId={layer.id} map={entry.maps[slot]} refresh={ui.version}>
            {#snippet placeholder()}<span class="mat-source-icon" aria-hidden="true">{@html api.ui.icon(info.icon ?? 'image')}</span>{/snippet}
          </MaterialThumb>
        {:else if info.icon}<span class="mat-source-icon" aria-hidden="true">{@html api.ui.icon(info.icon)}</span>
        {:else}<span class="mat-source-icon mat-ball" aria-hidden="true"></span>{/if}
        <span class="pm-select-label">{note ? `${info.label} · using Color` : info.label}</span>
        <svg class="pm-select-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 6.25 3.5 3.5 3.5-3.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
    </Row>
  </div>
{/snippet}

{#if entries.length}
  <Section {api} title="Material Options">
    <button type="button" class="surf-all" disabled={matching} data-use-front
      title={matching ? 'Every surface already matches Front.' : 'Copy Front’s sources and options, keyframes included, onto Bevel, Side and Back.'}
      onclick={() => useFrontForAll(api, layer.id)}>Use Front for All</button>
  </Section>
  <div class="materials" data-surface-materials>
    {#each entries as entry (entry.index)}
      {@const surface = surfaceAt(entry.index)}
      {@const open = isOpen(entry)}
      {@const count = (ui.version, surfaceOverrideCount(api, layer.id, surface))}
      {@const base = describeMap(api, layer.id, entry.index, 'color', entry.maps.color)}
      {@const bodyId = `surface-${layer.id}-${surface}`}
      <div class="mat" data-surface={surface}>
        <div class="row mat-head" class:open title={SURFACE_TIPS[surface]}>
          <button type="button" class="mat-twirl" aria-expanded={open} aria-controls={bodyId} aria-label={`${open ? 'Collapse' : 'Expand'} ${entry.name}`} onclick={() => toggle(entry)}>
            <span class="twirl" aria-hidden="true">{@html api.ui.icon(open ? 'chevD' : 'chev')}</span>
            <span class="mat-name">{entry.name}</span>
          </button>
          {#if !open}<span class="mat-summary" class:quiet={base.status === 'original'}>{base.status === 'original' ? (count ? `${count} change${count === 1 ? '' : 's'}` : 'Layer color') : base.label}</span>{/if}
          {#if open && count}<button type="button" class="mat-reset" title={`Put ${entry.name} back to the layer’s own color and finish`} onclick={() => resetSurface(api, layer.id, surface)}>Reset</button>{/if}
        </div>
        {#if open}
          <div class="mat-body" id={bodyId}>
            {#each LAYER_SLOTS as spec (spec.id)}{@render slotRow(entry, spec.id)}{/each}
            <Disclosure label="Image Maps" count={IMAGE_SLOTS.filter(spec => entry.maps[spec.id] != null).length || undefined} remember={`surface-maps:${layer.id}:${surface}`}>
              {#each IMAGE_SLOTS as spec (spec.id)}{@render slotRow(entry, spec.id)}{/each}
            </Disclosure>
            <div class="mat-options" role="group" aria-label={`${entry.name} options`}>
              {#each SURFACE_PROPERTY_SPECS as spec (spec.key)}<SurfaceProperty {api} {ui} {layer} {surface} {spec} />{/each}
            </div>
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  /* The same geometry as the Materials section (MaterialsSection.svelte), so a
     model's materials and an extruded layer's surfaces read as one system. */
  .surf-all { flex: none; padding: 0; border: 0; background: transparent; color: var(--tx-3); font-size: 12px; font-weight: var(--fw-regular, 400); line-height: 16px; }
  .surf-all:hover:not(:disabled) { color: var(--tx); }
  .surf-all:disabled { opacity: .55; }
  .surf-all:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 3px; }
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
  /* Never the bare class "drop": closing a menu removes every .drop element. */
  .mat-slot.is-drop { box-shadow: inset 0 0 0 1.5px var(--accent); background: var(--accent-dim); }
  .mat-slot.is-refused { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--tx-3) 60%, transparent); background: rgb(var(--ink-rgb) / .04); }
  .mat-slot.is-refused :global(.mat-source) { opacity: .55; }
  .mat-source { width: 100%; height: 24px; display: flex; align-items: center; gap: 6px; padding: 0 22px 0 4px; border: 0; border-radius: 6px; background: var(--bg-field); color: var(--tx-value); font-family: var(--f-ui); font-size: 12px; }
  .mat-source:hover { background: color-mix(in srgb, var(--bg-field) 80%, var(--tx) 6%); }
  .mat-source.original { color: var(--tx-3); }
  .mat-source.warn { color: var(--warning); }
  .mat-source :global(.pm-select-chevron) { right: 6px; width: 10px; height: 10px; margin-top: -5px; }
  .mat-source-icon { display: grid; place-items: center; width: 16px; height: 16px; flex: none; color: var(--tx-2); }
  .mat-source-icon :global(svg) { width: 13px; height: 13px; }
  .mat-source-icon.warn { color: var(--warning); }
  .mat-ball::before { content: ''; width: 10px; height: 10px; border-radius: 50%; background: radial-gradient(circle at 35% 30%, rgb(255 255 255 / .95), rgb(var(--ink-rgb) / .32) 72%); box-shadow: inset 0 0 0 .5px rgb(var(--ink-rgb) / .28); }
</style>
