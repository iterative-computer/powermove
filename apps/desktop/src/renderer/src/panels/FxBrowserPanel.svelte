<script lang="ts">
  import { kernelSignals } from '../kernel/signals.svelte';
  import { fxBrowser, pushRecent, readRecent } from './fx-browser.svelte';
  import type { PanelProps } from './registerSveltePanel';

  interface Item { id: string; label: string; group: string }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  const query = $derived(fxBrowser.query);
  const category = $derived(fxBrowser.category);
  let recent = $state<string[]>(readRecent());
  let status = $state('');
  let selectedId = $state<string | null>(null);
  let rail: HTMLDivElement | undefined = $state();
  let list: HTMLDivElement | undefined = $state();
  let searchEl: HTMLInputElement | undefined = $state();

  $effect(() => {
    if (fxBrowser.searchOpen) searchEl?.focus();
  });

  const effects = $derived.by<Item[]>(() => {
    kernelSignals.effects;
    return Object.entries((PM.FX ?? {}) as Record<string, { label: string; group: string }>).map(
      ([id, definition]) => ({ id, label: definition.label ?? id, group: definition.group ?? 'Other' })
    );
  });

  const items = $derived(effects);
  const categories = $derived([...new Set(items.map((item) => item.group))]);
  const recentItems = $derived(recent.map((id) => items.find((item) => item.id === id)).filter((item): item is Item => !!item));
  /* The rail: every family, plus Recent once something has been used. A
     category that disappears (extension unloaded) falls back to All. */
  const rails = $derived([
    { id: 'all', label: 'All' },
    ...(recentItems.length ? [{ id: 'recent', label: 'Recent' }] : []),
    ...categories.map((group) => ({ id: group, label: group }))
  ]);
  const activeCategory = $derived(rails.some((entry) => entry.id === category) ? category : 'all');
  const searching = $derived(query.trim().length > 0);
  const shown = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    if (needle) return items.filter((item) => item.label.toLowerCase().includes(needle) || item.id.toLowerCase().includes(needle));
    if (activeCategory === 'recent') return recentItems;
    if (activeCategory === 'all') return items;
    return items.filter((item) => item.group === activeCategory);
  });
  /* All browses by family; a chosen family, Recent, and any search are one flat list. */
  const sections = $derived.by<Array<{ group: string | null; items: Item[] }>>(() => {
    if (searching || activeCategory !== 'all') return [{ group: null, items: shown }];
    return categories
      .map((group) => ({ group, items: shown.filter((item) => item.group === group) }))
      .filter((section) => section.items.length);
  });

  function pick(id: string): void {
    fxBrowser.category = id;
    if (fxBrowser.query) fxBrowser.query = '';
  }

  function railKey(event: KeyboardEvent): void {
    const buttons = [...(rail?.querySelectorAll<HTMLButtonElement>('.fxb-cat') ?? [])];
    const index = buttons.indexOf(event.currentTarget as HTMLButtonElement);
    if (index < 0) return;
    const map: Record<string, number> = { ArrowUp: index - 1, ArrowDown: index + 1, Home: 0, End: buttons.length - 1 };
    const next = map[event.key];
    if (next === undefined) return;
    event.preventDefault();
    const target = buttons[Math.min(buttons.length - 1, Math.max(0, next))];
    target?.focus();
    if (target?.dataset.category) pick(target.dataset.category);
  }

  const key = (item: Item): string => item.id;

  function apply(event: MouseEvent | KeyboardEvent, item: Item): void {
    if ('detail' in event && event.detail > 1) return;
    const layer = PM.firstSel?.();
    if (!layer) {
      status = 'Select a layer first';
      PM.toast?.(status);
      return;
    }
    PM.Edit.apply({ type: 'add_effect', target: layer.id, effect: item.id }, { label: `Add ${item.label}`, origin: 'fx-browser' });
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
  {#if fxBrowser.searchOpen}
    <div class="fxb-searchrow">
      <input
        bind:this={searchEl}
        class="fxb-search"
        type="search"
        placeholder="Search effects"
        aria-label="Search effects"
        bind:value={fxBrowser.query}
        onkeydown={(event) => { if (event.key === 'Escape') { fxBrowser.searchOpen = false; fxBrowser.query = ''; } }}
      />
    </div>
  {/if}

  <div class="fxb-body">
    <div class="fxb-rail" bind:this={rail} role="tablist" aria-label="Effect categories" aria-orientation="vertical">
      {#each rails as entry (entry.id)}
        <button
          type="button"
          class="fxb-cat"
          class:on={entry.id === activeCategory && !searching}
          role="tab"
          aria-selected={entry.id === activeCategory && !searching}
          tabindex={entry.id === activeCategory ? 0 : -1}
          data-category={entry.id}
          onclick={() => pick(entry.id)}
          onkeydown={railKey}
        >{entry.label}</button>
      {/each}
    </div>

  <div class="fxb-list" bind:this={list} role="group" aria-label="Effects">
    {#each sections as section (section.group ?? '')}
      {#if section.group}<div class="fxb-sec">{section.group}</div>{/if}
      {#each section.items as item, index (key(item))}
        <div
          role="group"
          aria-label={item.label}
          class="fxb-row"
          class:on={selectedId === item.id}
          data-kind="effect"
          data-id={item.id}
        >
          <button
            type="button"
            class="fxb-pick fxb-label"
            draggable="true"
            tabindex={index === 0 && section === sections[0] ? 0 : -1}
            aria-pressed={selectedId === item.id}
            title="Drag onto a layer or use the plus button to add"
            onclick={() => selectedId = item.id}
            onkeydown={listKey}
            ondragstart={(event) => dragStart(event, item)}
          >{item.label}</button>
          <button type="button" class="fxb-add" aria-label={`Add ${item.label}`} title={`Add ${item.label}`} draggable="false" onclick={(event) => apply(event, item)}>
            <svg viewBox="0 0 16 16"><path d="M8 3.5v9M3.5 8h9"/></svg>
          </button>
        </div>
      {/each}
    {:else}
      <div class="fxb-empty">{searching ? `No effects match “${query}”` : 'Nothing here yet'}</div>
    {/each}
  </div>
  </div>

  <span class="panel-sr-only" role="status">{status}</span>
</div>
