<script module lang="ts">
  /** Which parts are open in the tree, per model, for the session. */
  const expanded = new Map<string, boolean>();
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import type { EditCommand, Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import PartRow from './PartRow.svelte';
  import { partEdited, resetPart, togglePartVisible } from './parts';

  /* Parts: the pieces inside an imported model (a console's body, its two
     controllers…), each movable and keyframeable without splitting the
     model into layers. Pick one here, or double-click it in the viewer; the
     viewer's Move, Rotate and Scale handles then work on that part. */
  let { api, ui, layer }: { api: PowermoveAPI; ui: SceneUiState; layer: Layer } = $props();
  const { Section, Row, TextField } = untrack(() => api.ui.controls);

  let version = $state(0);
  $effect(() => {
    const subscription = api.scene3d.onActivePartChange(() => { version++; });
    return () => subscription.dispose();
  });
  const parts = $derived((ui.version, api.scene3d.listParts(layer.id)));
  const active = $derived.by(() => { void version; return api.scene3d.getActivePart(layer.id); });
  const selected = $derived.by(() => { void version; void ui.version; return api.scene3d.getSelectedParts(layer.id).filter(key => parts.some(part => part.key === key)); });
  const activePart = $derived(selected.length === 1 ? parts.find(part => part.key === active) ?? null : null);
  const byKey = $derived(new Map(parts.map(part => [part.key, part])));
  const siblings = (parent: string | null) => parts.filter(part => part.parent === parent).length;

  /* Wrappers with a single child open by themselves, as does the path to the
     part being edited, so the interesting level shows without clicking. */
  function isOpen(key: string): boolean {
    void version;
    const remembered = expanded.get(`${layer.id}:${key}`);
    if (remembered != null) return remembered;
    for (const selectedKey of selected) {
      for (let parent: string | null = selectedKey; parent; parent = byKey.get(parent)?.parent ?? null) if (parent === key) return true;
    }
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
  let rangeAnchor: string | null = null;
  function choose(key: string | null, event?: MouseEvent | KeyboardEvent): void {
    let keys = key ? [key] : [];
    if (key && event?.shiftKey && rangeAnchor) {
      const from = shown.findIndex(part => part.key === rangeAnchor), to = shown.findIndex(part => part.key === key);
      if (from >= 0 && to >= 0) keys = shown.slice(Math.min(from,to), Math.max(from,to)+1).map(part => part.key);
    } else if (key && (event?.metaKey || event?.ctrlKey)) keys = selected.includes(key) ? selected.filter(item => item !== key) : [...selected, key];
    if (!event?.shiftKey) rangeAnchor = key;
    api.scene3d.setSelectedParts(layer.id, keys);
    api.transport.invalidate();
  }
  function group(): void {
    const result = api.scene3d.groupParts(layer.id, selected);
    if (!result.ok) api.ui.toast(result.message, { error: true });
  }
  function renameCommand(value: unknown): EditCommand {
    const current=api.model.layer(layer.id)!, data=(current.d as any).data, key=activePart!.key;
    return {type:'set_content',target:layer.id,patch:{data:{...data,object:{...data.object,parts:{...data.object.parts,[key]:{...data.object.parts[key],name:String(value).trim().slice(0,160)||'Part Group'}}}}}};
  }
  const canGroup = $derived(selected.length > 1 && selected.every(key => byKey.get(key)?.parent === byKey.get(selected[0]!)?.parent));
  const animatedGroup = $derived(activePart?.group && Object.values<any>((layer.d as any)?.data?.object?.parts?.[activePart.key]?.p || {}).some(p => p.kf?.length || p.expr));
  function ungroup(): void {
    if (!activePart?.group) return;
    const result = api.scene3d.ungroupParts(layer.id, activePart.key);
    if (!result.ok) api.ui.toast(result.message, { error: true });
  }
  function onKey(event: KeyboardEvent, key: string | null): void {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'g') {
      event.preventDefault(); event.stopPropagation();
      if (event.shiftKey) ungroup(); else if (canGroup) group();
    } else if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(key, event); }
  }
  function menu(event: MouseEvent, key: string): void {
    event.preventDefault();
    if (!selected.includes(key)) choose(key);
    api.ui.menu({x:event.clientX,y:event.clientY}, [
      {label:'Group Parts',kb:'⌘G',disabled:api.scene3d.getSelectedParts(layer.id).length < 2,run:group},
      {label:'Ungroup Parts',kb:'⌘⇧G',disabled:!byKey.get(key)?.group,run:()=>{const result=api.scene3d.ungroupParts(layer.id,key);if(!result.ok)api.ui.toast(result.message,{error:true});}}
    ]);
  }
</script>

{#if parts.length}
  <Section {api} title="Parts">
    <button type="button" class="part-action" disabled={!canGroup} title={selected.length > 1 && !canGroup ? 'Select parts at the same level' : 'Group selected parts (⌘G)'} aria-label="Group selected parts" onclick={group}>Group</button>
  </Section>
  <div class="parts" data-parts-section>
    <div class="part-tree" role="tree" aria-label="Model parts" aria-multiselectable="true">
      <div class="part" role="treeitem" tabindex="0" aria-selected={!active} class:on={!active} data-part-key=""
        onclick={(event) => choose(null, event)} onkeydown={(event) => onKey(event, null)}>
        <span class="twirl-gap" aria-hidden="true"></span>
        <span class="part-icon" aria-hidden="true">{@html api.ui.icon('cube')}</span>
        <span class="part-name">Whole Model</span>
      </div>
      {#each shown as part (part.key)}
        {@const open = isOpen(part.key)}
        {@const visible = (ui.version, ui.time, api.scene3d.getPart(layer.id, part.key, ui.time).values.visible)}
        {@const edited = (ui.version, partEdited(layer, part.key))}
        <div class="part" role="treeitem" tabindex="0" aria-selected={selected.includes(part.key)} aria-expanded={part.children ? open : undefined}
          class:on={selected.includes(part.key)} class:hidden-part={!visible} data-part-key={part.key} style={`--depth:${part.depth + 1}`}
          title={part.name} onclick={(event) => choose(part.key, event)} oncontextmenu={(event) => menu(event, part.key)} onkeydown={(event) => onKey(event, part.key)}>
          {#if part.children}
            <button type="button" class="twirl" aria-label={`${open ? 'Collapse' : 'Expand'} ${part.name}`} onclick={(event) => toggle(part.key, event)}>{@html api.ui.icon(open ? 'chevD' : 'chev')}</button>
          {:else}<span class="twirl-gap" aria-hidden="true"></span>{/if}
          {#if part.group}<span class="part-icon" aria-hidden="true">{@html api.ui.icon('layers')}</span>{/if}
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
          {#if activePart.group}<button type="button" class="part-reset" disabled={!!animatedGroup} title={animatedGroup ? 'Remove group animation before ungrouping' : 'Ungroup parts and keep their placement'} onclick={ungroup}>Ungroup</button>{/if}
          {#if partEdited(layer, activePart.key)}<button type="button" class="part-reset" title={`Put ${activePart.name} back where the model file has it`} onclick={() => resetPart(api, layer.id, activePart.key, activePart.name)}>Reset</button>{/if}
        </div>
        {#if activePart.group}
          <Row {api} label="Name"><TextField {api} get={() => activePart?.name || ''} edit={{mode:'command',label:'Rename part group',origin:'inspector',command:renameCommand}} label="Part group name" /></Row>
        {/if}
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Position" fields={['x', 'y', 'z']} step={0.01} />
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Rotation" fields={['rx', 'ry', 'rz']} unit="°" />
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Scale" fields={['scale']} unit="%" min={0} />
      </div>
    {:else if selected.length > 1}
      <p class="part-hint">{selected.length} parts selected</p>
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
  .part-action { margin-left: auto; padding: 0 4px; color: var(--tx-2); background: transparent; border: 0; font: inherit; }
  .part-action:disabled, .part-reset:disabled { opacity: .4; cursor: default; }
  .part-hint { margin: 4px 6px 2px; color: var(--tx-3); font-size: 12px; line-height: 16px; }
</style>
