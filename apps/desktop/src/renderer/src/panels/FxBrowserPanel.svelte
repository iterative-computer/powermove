<script lang="ts">
  import { kernelSignals } from '../kernel/signals.svelte';
  import { validMatteSource } from '../legacy/core/matte';
  import { fxBrowser, pushRecent, readRecent } from './fx-browser.svelte';
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
    if (needle) return [{ group: null, items: items.filter((item) => item.label.toLowerCase().includes(needle) || item.id.toLowerCase().includes(needle) || item.group.toLowerCase().includes(needle)) }];
    return [
      ...(recentItems.length ? [{ group: 'Recently Used', items: recentItems }] : []),
      ...categories.map((group) => ({ group, items: items.filter((item) => item.group === group) })),
    ].filter((section) => section.items.length);
  });
  const folded = (group: string | null) => !!group && !searching && fxBrowser.folded.includes(group);
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
      placeholder="Search effects"
      aria-label="Search effects"
      bind:value={fxBrowser.query}
      onkeydown={(event) => { if (event.key === 'Escape') fxBrowser.query = ''; }}
    />
  </div>

  <div class="fxb-list" bind:this={list} role="tree" aria-label="Effects">
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
              tabindex={index === 0 && section === sections[0] ? 0 : -1}
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
      <div class="fxb-empty">{searching ? `No effects match “${query}”` : 'Nothing here yet'}</div>
    {/each}
  </div>

  <span class="panel-sr-only" role="status">{status}</span>
</div>
