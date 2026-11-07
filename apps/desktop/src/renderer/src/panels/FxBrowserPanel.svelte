<script lang="ts">
  import { kernelSignals } from '../kernel/signals.svelte';
  import { validMatteSource } from '../legacy/core/matte';
  import { fxBrowser, pushRecent, readRecent } from './fx-browser.svelte';
  import { applyPreset, presetDetail, type StoredPreset } from '../presets/model';
  import { tick } from 'svelte';
  import type { PanelProps } from './registerSveltePanel';

  interface Item { id: string; label: string; group: string }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  const query = $derived(fxBrowser.query);
  let recent = $state<string[]>(readRecent());
  let status = $state('');
  let selectedId = $state<string | null>(null);
  let list: HTMLDivElement | undefined = $state();
  let searchEl: HTMLInputElement | undefined = $state();

  /* My Presets: the user's saved animation presets (PM.Presets), listed
     first as After Effects lists Animation Presets. The engine announces
     every change on 'presets', including saves from another window. */
  const PRESETS = 'My Presets';
  const presetsAvailable = !!PM.Presets?.list;
  let presetVersion = $state(0);
  let renamingId = $state<string | null>(null);
  let renameValue = $state('');
  let selVersion = $state(0);
  $effect(() => PM.bus?.on?.('presets', () => { presetVersion += 1; }));
  $effect(() => PM.bus?.on?.('sel', () => { selVersion += 1; }));
  const hasSelection = $derived.by(() => { selVersion; return !!PM.firstSel?.(); });
  const presets = $derived.by<StoredPreset[]>(() => {
    presetVersion;
    return presetsAvailable ? (PM.Presets.list() as StoredPreset[]) : [];
  });
  const presetMatches = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    return needle ? presets.filter((preset) => preset.name.toLowerCase().includes(needle)) : presets;
  });

  /* The header's search action focuses the always-visible field. */
  $effect(() => {
    if (fxBrowser.searchOpen) { searchEl?.focus(); fxBrowser.searchOpen = false; }
  });

  const effects = $derived.by<Item[]>(() => {
    kernelSignals.effects;
    // Browsing needs metadata, not translated GLSL for every unused effect.
    const definitions=PM.Kernel?.effects?.list?.();
    if(Array.isArray(definitions))return definitions.map(({id,label,group})=>({id,label:label??id,group:group??'Other'}));
    return Object.entries((PM.FX ?? {}) as Record<string, { label: string; group: string }>).map(
      ([id, definition]) => ({ id, label: definition.label ?? id, group: definition.group ?? 'Other' })
    );
  });

  /* Track Matte is a layer setting, offered here as an effect (After Effects
     keeps it in a timeline column). Applying it mattes the layer to the one
     directly above, or the nearest layer above that can be a matte. */
  const TRACK_MATTE: Item = { id: 'track-matte', label: 'Track Matte', group: 'Matte' };
  const items = $derived(effects.some((item) => item.id === TRACK_MATTE.id) ? effects : [...effects, TRACK_MATTE]);
  const categories = $derived([...new Set(items.map((item) => item.group))]);
  const recentItems = $derived(recent.map((id) => items.find((item) => item.id === id)).filter((item): item is Item => !!item));
  const searching = $derived(query.trim().length > 0);
  /* A tree of families, as After Effects lists them; Recently Used leads once
     something has been applied. A search is one flat list of matches. */
  const sections = $derived.by<Array<{ group: string | null; items: Item[] }>>(() => {
    const needle = query.trim().toLowerCase();
    if (needle) return [{ group: null, items: items.filter((item) => item.label.toLowerCase().includes(needle) || item.id.toLowerCase().includes(needle) || item.group.toLowerCase().includes(needle)) }].filter((section) => section.items.length);
    return [
      ...(recentItems.length ? [{ group: 'Recently Used', items: recentItems }] : []),
      ...categories.map((group) => ({ group, items: items.filter((item) => item.group === group) })),
    ].filter((section) => section.items.length);
  });
  const folded = (group: string | null) => !!group && !searching && fxBrowser.folded.includes(group);
  const presetsOpen = $derived(searching || !fxBrowser.folded.includes(PRESETS));
  const showPresets = $derived(presetsAvailable && (!searching || presetMatches.length > 0));
  function toggleFold(group: string): void {
    fxBrowser.folded = folded(group) ? fxBrowser.folded.filter((name) => name !== group) : [...fxBrowser.folded, group];
  }

  const key = (item: Item): string => item.id;

  function applyTrackMatte(layer: any): string {
    const layers: any[] = PM.proj?.layers ?? [];
    const index = layers.findIndex((candidate) => candidate.id === layer.id);
    const valid = (candidate: any) => candidate && candidate.id !== layer.id && validMatteSource(layers, layer, candidate.id);
    const source = [...layers.slice(0, Math.max(0, index))].reverse().find(valid) ?? layers.find(valid);
    if (!source) return 'Track Matte needs another visible layer to use as its matte';
    PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { matteSource: source.id } }, { label: 'Add Track Matte', origin: 'fx-browser' });
    return '';
  }

  function apply(event: MouseEvent | KeyboardEvent, item: Item): void {
    const layer = PM.firstSel?.();
    if (!layer) {
      status = 'Select a layer first';
      PM.toast?.(status);
      return;
    }
    if (item.id === TRACK_MATTE.id) {
      const problem = applyTrackMatte(layer);
      if (problem) { status = problem; PM.toast?.(problem); return; }
    } else PM.Edit.apply({ type: 'add_effect', target: layer.id, effect: item.id }, { label: `Add ${item.label}`, origin: 'fx-browser' });
    PM.Inspector?.refresh?.();
    PM.invalidate();
    recent = pushRecent(item.id);
    status = `Added ${item.label}`;
  }

  function dragStart(event: DragEvent, item: Item): void {
    event.dataTransfer?.setData(
      'application/x-powermove-fx',
      JSON.stringify({ kind: 'effect', id: item.id, label: item.label })
    );
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'copy';
  }

  function applyPresetRow(preset: StoredPreset): void {
    if (renamingId) return;
    selectedId = `preset:${preset.id}`;
    if (applyPreset(PM, preset.id)) status = `Applied ${preset.name}`;
  }

  function presetDragStart(event: DragEvent, preset: StoredPreset): void {
    event.dataTransfer?.setData('application/x-powermove-fx', JSON.stringify({ kind: 'preset', id: preset.id, label: preset.name }));
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'copy';
  }

  async function startRename(preset: StoredPreset): Promise<void> {
    renamingId = preset.id;
    renameValue = preset.name;
    await tick();
    const input = list?.querySelector<HTMLInputElement>('.fxb-rename');
    input?.focus(); input?.select();
  }

  async function finishRename(commit: boolean): Promise<void> {
    const id = renamingId;
    if (!id) return;
    const preset = presets.find((item) => item.id === id);
    renamingId = null;
    if (commit && preset && renameValue.trim() && renameValue.trim() !== preset.name) {
      try {
        if (PM.Presets.rename(id, renameValue)) status = `Renamed to ${renameValue.trim()}`;
      } catch (error) {
        PM.toast?.(error instanceof Error ? error.message : String(error), 3200, { kind: 'alert' });
      }
    }
    await tick();
    list?.querySelector<HTMLElement>(`[data-id="preset:${CSS.escape(id)}"] .fxb-pick`)?.focus();
  }

  function deletePreset(preset: StoredPreset): void {
    void PM.confirm?.({
      message: `Delete “${preset.name}”?`,
      detail: 'The preset is removed from My Presets in every window. Layers it was applied to keep their effects and keyframes.',
      confirmLabel: 'Delete Preset'
    })?.then((ok: boolean) => {
      if (!ok) return;
      if (PM.Presets.remove(preset.id)) status = `Deleted ${preset.name}`;
    });
  }

  function presetMenu(event: MouseEvent | KeyboardEvent, preset: StoredPreset): void {
    event.preventDefault();
    selectedId = `preset:${preset.id}`;
    const layer = PM.firstSel?.();
    const at = event instanceof MouseEvent && event.clientX ? { x: event.clientX, y: event.clientY } : undefined;
    PM.menu?.(event.currentTarget as HTMLElement, [
      { header: preset.name },
      { label: layer ? `Apply to ${layer.name}` : 'Apply to Selected Layer', disabled: !layer, run: () => applyPresetRow(preset) },
      '-',
      { label: 'Rename…', run: () => { void startRename(preset); } },
      { label: 'Delete…', icon: 'trash', run: () => deletePreset(preset) },
    ], at);
  }

  function presetKey(event: KeyboardEvent, preset: StoredPreset): void {
    if (event.key === 'Enter') { event.preventDefault(); applyPresetRow(preset); }
    else if (event.key === 'Backspace' || event.key === 'Delete') { event.preventDefault(); deletePreset(preset); }
    else if (event.key === 'F2') { event.preventDefault(); void startRename(preset); }
    else if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) presetMenu(event, preset);
    else listKey(event);
  }

  function renameKey(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Enter') { event.preventDefault(); void finishRename(true); }
    else if (event.key === 'Escape') { event.preventDefault(); void finishRename(false); }
  }

  function listKey(event: KeyboardEvent): void {
    const rows = [...(list?.querySelectorAll<HTMLElement>('.fxb-pick') ?? [])];
    const index = rows.indexOf(event.currentTarget as HTMLElement);
    if (index < 0) return;
    const map: Record<string, number> = { ArrowUp: index - 1, ArrowDown: index + 1, Home: 0, End: rows.length - 1 };
    const next = map[event.key];
    if (next === undefined) return;
    event.preventDefault();
    rows[Math.min(rows.length - 1, Math.max(0, next))]?.focus();
  }
</script>

<div class="fxb" data-svelte-panel={panelId}>
  <div class="fxb-searchrow">
    <svg class="fxb-searchicon" viewBox="0 0 256 256" aria-hidden="true">{@html PM.ICONS?.search ?? ''}</svg>
    <input
      bind:this={searchEl}
      class="fxb-search"
      type="search"
      placeholder={presetsAvailable ? 'Search effects and presets' : 'Search effects'}
      aria-label={presetsAvailable ? 'Search effects and presets' : 'Search effects'}
      bind:value={fxBrowser.query}
      onkeydown={(event) => { if (event.key === 'Escape') fxBrowser.query = ''; }}
    />
  </div>

  <div class="fxb-list" bind:this={list} role="tree" aria-label="Effects">
    {#if showPresets}
      {#if !searching}
        <button type="button" class="fxb-sec" aria-expanded={presetsOpen} onclick={() => toggleFold(PRESETS)}>
          <svg class="fxb-twirl" class:open={presetsOpen} viewBox="0 0 256 256" aria-hidden="true">{@html PM.ICONS?.chev ?? ''}</svg>
          {PRESETS}
          {#if presets.length}<span class="fxb-count">{presets.length}</span>{/if}
        </button>
      {/if}
      {#if presetsOpen}
        {#each presetMatches as preset, index (preset.id)}
          {@const rowId = `preset:${preset.id}`}
          <div
            role="treeitem"
            aria-selected={selectedId === rowId}
            aria-label={preset.name}
            tabindex="-1"
            class="fxb-row fxb-preset"
            class:on={selectedId === rowId}
            data-kind="preset"
            data-id={rowId}
            oncontextmenu={(event) => presetMenu(event, preset)}
          >
            {#if renamingId === preset.id}
              <span class="fxb-badge fxb-badge-preset" aria-hidden="true"><svg viewBox="0 0 10 10"><path d="M5 1 9 5 5 9 1 5Z"/></svg></span>
              <input class="fxb-rename" type="text" maxlength="80" spellcheck="false" aria-label="Preset name"
                bind:value={renameValue} onkeydown={renameKey} onblur={() => finishRename(true)} />
            {:else}
              <button
                type="button"
                class="fxb-pick fxb-label"
                draggable="true"
                tabindex={index === 0 ? 0 : -1}
                aria-pressed={selectedId === rowId}
                title={`${presetDetail(PM, preset)}\nDouble-click to apply at the playhead, or drag onto a layer`}
                onclick={() => selectedId = rowId}
                ondblclick={() => applyPresetRow(preset)}
                onkeydown={(event) => presetKey(event, preset)}
                ondragstart={(event) => presetDragStart(event, preset)}
              ><span class="fxb-badge fxb-badge-preset" aria-hidden="true"><svg viewBox="0 0 10 10"><path d="M5 1 9 5 5 9 1 5Z"/></svg></span><span class="fxb-name">{preset.name}</span></button>
              <button type="button" class="fxb-add" aria-label={`Apply ${preset.name}`} title={`Apply ${preset.name} to the selected layer`} draggable="false" onclick={() => applyPresetRow(preset)}>
                <svg viewBox="0 0 16 16"><path d="M8 3.5v9M3.5 8h9"/></svg>
              </button>
            {/if}
          </div>
        {:else}
          <div class="fxb-presets-empty" data-presets-empty>
            <p class="fxb-presets-how">Right-click a layer and choose <b>Save Animation Preset…</b> to keep its effects and keyframes here.</p>
            <button type="button" class="chip" disabled={!hasSelection} onclick={() => PM.cmd?.('saveAnimationPreset')}>Save from Selected Layer…</button>
          </div>
        {/each}
      {/if}
    {/if}
    {#each sections as section (section.group ?? '')}
      {#if section.group}
        <button type="button" class="fxb-sec" aria-expanded={!folded(section.group)} onclick={() => toggleFold(section.group!)}>
          <svg class="fxb-twirl" class:open={!folded(section.group)} viewBox="0 0 256 256" aria-hidden="true">{@html PM.ICONS?.chev ?? ''}</svg>
          {section.group}
        </button>
      {/if}
      {#if !folded(section.group)}
        {#each section.items as item, index (key(item))}
          <div
            role="treeitem"
            aria-selected={selectedId === item.id}
            aria-label={item.label}
            class="fxb-row"
            class:on={selectedId === item.id}
            data-kind="effect"
            data-id={item.id}
          >
            <button
              type="button"
              class="fxb-pick fxb-label"
              draggable={item.id !== TRACK_MATTE.id}
              tabindex={index === 0 && section === sections[0] && !(showPresets && presetsOpen && presetMatches.length) ? 0 : -1}
              aria-pressed={selectedId === item.id}
              title="Double-click to apply to the selected layer, or drag onto a layer"
              onclick={() => selectedId = item.id}
              ondblclick={(event) => apply(event, item)}
              onkeydown={(event) => { if (event.key === 'Enter') { event.preventDefault(); apply(event, item); } else listKey(event); }}
              ondragstart={(event) => dragStart(event, item)}
            ><span class="fxb-badge" aria-hidden="true">fx</span><span class="fxb-name">{item.label}</span></button>
            <button type="button" class="fxb-add" aria-label={`Add ${item.label}`} title={`Add ${item.label}`} draggable="false" onclick={(event) => apply(event, item)}>
              <svg viewBox="0 0 16 16"><path d="M8 3.5v9M3.5 8h9"/></svg>
            </button>
          </div>
        {/each}
      {/if}
    {:else}
      {#if !(searching && presetMatches.length)}
        <div class="fxb-empty">{searching ? `No effects ${presetsAvailable ? 'or presets ' : ''}match “${query}”` : 'Nothing here yet'}</div>
      {/if}
    {/each}
  </div>

  <span class="panel-sr-only" role="status">{status}</span>
</div>

<style>
  /* Section count: quiet meta at the right of the heading. */
  .fxb-count {
    margin-left: auto;
    color: var(--tx-4);
    font-size: var(--fs-xs);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
  }

  /* A preset's badge is a keyframe diamond where an effect's says "fx". */
  .fxb-badge-preset { width: 20px; padding: 0; }
  .fxb-badge-preset svg { width: 8px; height: 8px; fill: currentColor; }

  /* Inline rename, Finder-style: the row keeps its height and indent. */
  .fxb-rename {
    flex: 1;
    min-width: 0;
    height: 20px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--r-xs);
    background: var(--bg-field);
    color: var(--tx-value);
    font: inherit;
    font-size: 12px;
    box-shadow: 0 0 0 1px var(--accent);
  }
  .fxb-rename:focus { outline: 0; }

  /* Plain text in the section's indent, as the list's other empty states. */
  .fxb-presets-empty {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 4px 6px 10px 24px;
    color: var(--tx-3);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .fxb-presets-empty p { margin: 0; text-wrap: pretty; }
  .fxb-presets-how b { color: var(--tx-2); font-weight: 500; }
  .fxb-presets-empty .chip { margin-top: 2px; }
</style>
