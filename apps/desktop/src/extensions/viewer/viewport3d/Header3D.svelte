<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import type { MenuContribution, PowermoveAPI } from 'powermove';
  import { icon } from './icons';
  import { viewMenu, selectMenu, addMenu, objectMenu, meshMenu, modeMenu, ORIENTATIONS, PIVOTS, SHADINGS } from './menus';

  /* Blender's 3D Viewport header: mode, menus, transform settings, overlays and shading. */
  let { api }: { api: PowermoveAPI } = $props();
  const viewport = untrack(() => api.scene3d.viewport);
  let vp = $state(untrack(() => viewport.state()));
  let preview = $state(untrack(() => api.scene3d.previewState()));
  let autoKey = $state(untrack(() => api.scene3d.getAutoKey()));
  const subscriptions = untrack(() => [
    viewport.onChange((next) => { vp = next; autoKey = api.scene3d.getAutoKey(); }),
    api.scene3d.onPreviewChange((next) => { preview = next; })
  ]);
  onDestroy(() => { for (const subscription of subscriptions) subscription.dispose(); });

  const open = (event: MouseEvent, items: () => MenuContribution[]) => api.ui.menu(event.currentTarget as HTMLElement, items());
  const orientation = $derived(ORIENTATIONS.find((entry) => entry.id === vp.orientation) ?? ORIENTATIONS[0]);
  const pivot = $derived(PIVOTS.find((entry) => entry.id === vp.pivot) ?? PIVOTS[3]);
  const orientationItems = (): MenuContribution[] => [{ header: 'Transform Orientation' },
    ...ORIENTATIONS.map((entry) => ({ label: entry.label, on: vp.orientation === entry.id, run: () => viewport.run('orientation', entry.id) }))];
  const pivotItems = (): MenuContribution[] => [{ header: 'Pivot Point' },
    ...PIVOTS.map((entry) => ({ label: entry.label, on: vp.pivot === entry.id, run: () => viewport.run('pivot', entry.id) }))];
</script>

<div class="vp-header" role="toolbar" aria-label="3D viewport header">
  <button type="button" class="mode" aria-haspopup="menu" aria-label={`Interaction mode: ${vp.editMode ? 'Edit Mode' : 'Object Mode'}`} title="Interaction Mode (Tab toggles Edit Mode)"
    onclick={(event) => open(event, () => modeMenu(api))}>{vp.editMode ? 'Edit Mode' : 'Object Mode'}</button>
  <button type="button" class="menu" aria-haspopup="menu" onclick={(event) => open(event, () => viewMenu(api))}>View</button>
  <button type="button" class="menu" aria-haspopup="menu" onclick={(event) => open(event, () => selectMenu(api))}>Select</button>
  <button type="button" class="menu" aria-haspopup="menu" onclick={(event) => open(event, () => addMenu(api))}>Add</button>
  {#if vp.editMode}
    <button type="button" class="menu" aria-haspopup="menu" onclick={(event) => open(event, () => meshMenu(api))}>Mesh</button>
  {:else}
    <button type="button" class="menu" aria-haspopup="menu" onclick={(event) => open(event, () => objectMenu(api))}>Object</button>
  {/if}
  <span class="sep" aria-hidden="true"></span>
  <button type="button" class="dropdown" aria-haspopup="menu" aria-label={`Transform orientation: ${orientation.label}`} title="Transform Orientation (,)"
    onclick={(event) => open(event, orientationItems)}>{@html `${icon('orientation')}<span>${orientation.label}</span>${icon('chevron', 12)}`}</button>
  <button type="button" class="dropdown" aria-haspopup="menu" aria-label={`Pivot point: ${pivot.label}`} title={`Pivot Point: ${pivot.label} (.)`}
    onclick={(event) => open(event, pivotItems)}>{@html icon(pivot.icon) + icon('chevron', 12)}</button>
  <button type="button" class="toggle" aria-pressed={vp.snap} aria-label="Snap" title="Snap (Shift Tab; hold Ctrl while transforming)"
    onclick={() => viewport.run('snapping.toggle')}>{@html icon('magnet')}</button>
  {#if vp.editMode}<span class="status" role="status">Verts {vp.editMode.selected}/{vp.editMode.total}</span>{/if}
  <span class="grow"></span>
  {#if preview.busy}<span class="status" role="status">Rendering…</span>{:else if preview.error}<span class="status error" title={preview.error}>Render failed</span>{/if}
  <button type="button" class="toggle" aria-pressed={vp.lockCamera} aria-label="Lock camera to view" title="Lock Camera to View: navigating in camera view moves the camera"
    onclick={() => viewport.run('view.lock-camera')}>{@html icon('lock')}</button>
  <button type="button" class="toggle" aria-pressed={vp.overlays} aria-label="Overlays" title="Viewport Overlays (Shift Alt Z)"
    onclick={() => viewport.run('overlays.toggle')}>{@html icon('overlays')}</button>
  <button type="button" class="toggle" aria-pressed={vp.xray} aria-label="X-ray" title="Toggle X-Ray (Alt Z): see through models in Solid and Wireframe"
    onclick={() => viewport.run('xray.toggle')}>{@html icon('xray')}</button>
  <div class="shading" role="radiogroup" aria-label="Viewport shading">
    {#each SHADINGS as shading (shading.id)}
      <button type="button" role="radio" aria-checked={vp.shading === shading.id} aria-label={shading.label}
        title={`${shading.label} (Z)${shading.id === 'material' ? ' · EEVEE' : shading.id === 'rendered' ? ` · ${preview.engine === 'cycles' ? 'Cycles' : 'EEVEE'}` : ''}`}
        onclick={() => viewport.run('shading', shading.id)}>{@html icon(shading.icon)}</button>
    {/each}
  </div>
  <button type="button" class="toggle record" aria-pressed={autoKey} aria-label="Auto keying" title="Auto Keying: transforms add keyframes"
    onclick={() => api.scene3d.setAutoKey(!autoKey)}>{@html icon('record', 14)}</button>
</div>

<style>
  .vp-header{display:flex;align-items:center;gap:2px;min-width:0;width:100%;overflow-x:auto;scrollbar-width:none;font:var(--fs-sm) var(--f-ui);color:var(--tx-2)}
  .mode{margin-right:4px;background:color-mix(in srgb,var(--tx) 5%,transparent)}
  button{all:unset;box-sizing:border-box;display:inline-flex;flex:none;align-items:center;gap:4px;height:24px;padding:0 8px;border-radius:var(--r-sm);color:var(--tx-2);cursor:pointer;white-space:nowrap}
  button:hover{background:color-mix(in srgb,var(--tx) 8%,transparent);color:var(--tx)}
  button:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}
  .dropdown{padding:0 6px;gap:5px;background:color-mix(in srgb,var(--tx) 5%,transparent)}
  .toggle{padding:0 6px}
  .toggle[aria-pressed="true"]{background:color-mix(in srgb,var(--accent) 26%,transparent);color:var(--tx)}
  .record[aria-pressed="true"]{color:#ff4b4b;background:color-mix(in srgb,#ff4b4b 18%,transparent)}
  .sep{width:1px;height:16px;margin:0 6px;background:color-mix(in srgb,var(--tx) 14%,transparent)}
  .grow{flex:1;min-width:8px}
  .status{padding:0 8px;color:var(--tx-3);white-space:nowrap}
  .status.error{color:#ff6b6b}
  .shading{display:inline-flex;gap:1px;padding:1px;border-radius:var(--r-sm);background:color-mix(in srgb,var(--tx) 5%,transparent);margin:0 2px}
  .shading button{padding:0 6px;height:22px}
  .shading button[aria-checked="true"]{background:var(--accent);color:var(--on-accent,#fff)}
  :global(.vp-header svg){flex:none}
</style>
