<script module lang="ts">
  /** Which parts are open in the tree, per model, for the session. */
  const expanded = new Map<string, boolean>();
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import type { Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import PartRow from './PartRow.svelte';
  import { partEdited, resetPart, togglePartVisible } from './parts';

  /* Parts: the pieces inside an imported model (a console's body, its two
     controllers…), each movable and keyframeable without splitting the
     model into layers. Pick one here, or double-click it in the viewer; the
     viewer's Move, Rotate and Scale handles then work on that part. */
  let { api, ui, layer }: { api: PowermoveAPI; ui: SceneUiState; layer: Layer } = $props();
  const { Section } = untrack(() => api.ui.controls);

  let version = $state(0);
  $effect(() => {
    const subscription = api.scene3d.onActivePartChange(() => { version++; });
    return () => subscription.dispose();
  });
  const parts = $derived((ui.version, api.scene3d.listParts(layer.id)));
  const active = $derived.by(() => { void version; return api.scene3d.getActivePart(layer.id); });
  const activePart = $derived(parts.find(part => part.key === active) ?? null);
  const byKey = $derived(new Map(parts.map(part => [part.key, part])));
  const siblings = (parent: string | null) => parts.filter(part => part.parent === parent).length;

  /* Wrappers with a single child open by themselves, as does the path to the
     part being edited, so the interesting level shows without clicking. */
  function isOpen(key: string): boolean {
    void version;
    const remembered = expanded.get(`${layer.id}:${key}`);
    if (remembered != null) return remembered;
    if (active && (active === key || active.startsWith(`${key}_`))) return true;
    const part = byKey.get(key);
    return !!part && (siblings(part.parent) === 1 || part.depth === 0 && parts.length <= 24);
  }
  function toggle(key: string, event: MouseEvent): void {
    event.stopPropagation();
    expanded.set(`${layer.id}:${key}`, !isOpen(key));
    version++;
  }
  const shown = $derived.by(() => {
    void version;
    return parts.filter(part => {
      for (let parent = part.parent; parent != null; parent = byKey.get(parent)?.parent ?? null) if (!isOpen(parent)) return false;
      return true;
    });
  });
  function choose(key: string | null): void {
    api.scene3d.setActivePart(layer.id, key);
    api.transport.invalidate();
  }
  function onKey(event: KeyboardEvent, key: string | null): void {
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(key); }
  }
</script>

{#if parts.length}
  <Section {api} title="Parts" />
  <div class="parts" data-parts-section>
    <div class="part-tree" role="tree" aria-label="Model parts">
      <div class="part" role="treeitem" tabindex="0" aria-selected={!active} class:on={!active} data-part-key=""
        onclick={() => choose(null)} onkeydown={(event) => onKey(event, null)}>
        <span class="twirl-gap" aria-hidden="true"></span>
        <span class="part-icon" aria-hidden="true">{@html api.ui.icon('cube')}</span>
        <span class="part-name">Whole Model</span>
      </div>
      {#each shown as part (part.key)}
        {@const open = isOpen(part.key)}
        {@const visible = (ui.version, ui.time, api.scene3d.getPart(layer.id, part.key, ui.time).values.visible)}
        {@const edited = (ui.version, partEdited(layer, part.key))}
        <div class="part" role="treeitem" tabindex="0" aria-selected={active === part.key} aria-expanded={part.children ? open : undefined}
          class:on={active === part.key} class:hidden-part={!visible} data-part-key={part.key} style={`--depth:${part.depth + 1}`}
          title={part.name} onclick={() => choose(part.key)} onkeydown={(event) => onKey(event, part.key)}>
          {#if part.children}
            <button type="button" class="twirl" aria-label={`${open ? 'Collapse' : 'Expand'} ${part.name}`} onclick={(event) => toggle(part.key, event)}>{@html api.ui.icon(open ? 'chevD' : 'chev')}</button>
          {:else}<span class="twirl-gap" aria-hidden="true"></span>{/if}
          <span class="part-name">{part.name}</span>
          {#if edited}<span class="part-dot" title="Moved or animated" aria-label="Changed"></span>{/if}
          <button type="button" class="part-eye" class:off={!visible} aria-label={`${visible ? 'Hide' : 'Show'} ${part.name}`} aria-pressed={!visible}
            title={visible ? 'Hide this part' : 'Show this part'} onclick={(event) => { event.stopPropagation(); togglePartVisible(api, layer.id, part.key, part.name); }}>
            {@html api.ui.icon(visible ? 'eye' : 'eyeoff')}
          </button>
        </div>
      {/each}
    </div>
    {#if activePart}
      <div class="part-edit" data-part-editor={activePart.key}>
        <div class="row part-head">
          <span class="part-title">{activePart.name}</span>
          {#if partEdited(layer, activePart.key)}<button type="button" class="part-reset" title={`Put ${activePart.name} back where the model file has it`} onclick={() => resetPart(api, layer.id, activePart.key, activePart.name)}>Reset</button>{/if}
        </div>
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Position" fields={['x', 'y', 'z']} step={0.01} />
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Rotation" fields={['rx', 'ry', 'rz']} unit="°" />
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Scale" fields={['scale']} unit="%" min={0} />
      </div>
    {:else}
      <p class="part-hint">Double-click the model in the viewer to pick a part, then drag it with the handles.</p>
    {/if}
  </div>
{/if}

<style>
  /* The tree reads like the Timeline's layer rows: 26px rows, indented by
     depth, the edited part tinted with the accent. */
  .parts { display: flex; flex-direction: column; margin: 0 calc(8px - var(--pad)) 6px; }
  .part-tree { display: flex; flex-direction: column; max-height: 232px; overflow-y: auto; padding: 2px 0; }
  .part { display: flex; align-items: center; gap: 6px; height: 26px; flex: none; padding: 0 4px 0 calc(6px + (var(--depth, 0) * 14px)); border-radius: 6px; color: var(--tx); cursor: default; }
  .part:hover { background: var(--bg-row); }
  .part:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .part.on { background: var(--accent-dim); color: var(--accent-tx); }
  .part.hidden-part .part-name { color: var(--tx-3); }
  .twirl, .twirl-gap { display: grid; place-items: center; width: 12px; height: 12px; flex: none; padding: 0; border: 0; background: none; color: var(--tx-2); }
  .twirl :global(svg) { width: 10px; height: 10px; }
  .part-icon { display: grid; place-items: center; width: 14px; height: 14px; margin-left: -4px; flex: none; color: var(--tx-2); }
  .part-icon :global(svg) { width: 13px; height: 13px; }
  .part.on .part-icon { color: inherit; }
  .part-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; line-height: 16px; }
  .part-dot { width: 6px; height: 6px; flex: none; border-radius: 50%; background: var(--accent); }
  .part-eye { display: grid; place-items: center; width: 20px; height: 20px; flex: none; padding: 0; border: 0; border-radius: 5px; background: none; color: var(--tx-3); opacity: 0; }
  .part-eye :global(svg) { width: 13px; height: 13px; }
  .part:hover .part-eye, .part-eye:focus-visible, .part-eye.off { opacity: 1; }
  .part-eye:hover { color: var(--tx); }
  .part-edit { margin-top: 6px; padding-top: 4px; border-top: 1px solid var(--line); }
  .part-head { height: 28px; margin: 0; padding: 0 6px; gap: 8px; }
  .part-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: var(--fw-semibold); color: var(--tx); }
  .part-reset { flex: none; padding: 0; border: 0; background: transparent; color: var(--tx-3); font-size: 12px; line-height: 16px; }
  .part-reset:hover { color: var(--tx); }
  .part-hint { margin: 4px 6px 2px; color: var(--tx-3); font-size: 12px; line-height: 16px; }
</style>
