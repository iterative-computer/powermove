<script lang="ts">
  import { kernelSignals } from '../kernel/signals.svelte';
  import { fxBrowser } from './fx-browser.svelte';
  import type { PanelProps } from './registerSveltePanel';

  type Kind = 'effect' | 'transition';
  interface Item { kind: Kind; id: string; label: string; group: string }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  const kind = $derived(fxBrowser.kind);
  const query = $derived(fxBrowser.query);
  let status = $state('');
  let list: HTMLDivElement | undefined = $state();
  let searchEl: HTMLInputElement | undefined = $state();

  $effect(() => {
    if (fxBrowser.searchOpen) searchEl?.focus();
  });

  const effects = $derived.by<Item[]>(() => {
    kernelSignals.effects;
    return Object.entries((PM.FX ?? {}) as Record<string, { label: string; group: string }>).map(
      ([id, definition]) => ({ kind: 'effect', id, label: definition.label ?? id, group: definition.group ?? 'Other' })
    );
  });

  const transitions = $derived.by<Item[]>(() => {
    kernelSignals.transitions;
    const found = new Map<string, any>();
    try {
      for (const definition of PM.Kernel?.transitions?.list?.() ?? []) {
        if (definition?.id && !found.has(definition.id)) found.set(definition.id, definition);
      }
    } catch {
      // An extension registry must not take the browser down with it.
    }
    for (const [id, definition] of Object.entries(PM.TRANSITIONS ?? {}) as Array<[string, any]>) {
      if (!found.has(id)) found.set(id, { id, ...definition });
    }
    return [...found.values()].map((definition) => ({
      kind: 'transition',
      id: definition.id,
      label: definition.label ?? definition.id,
      group: definition.group ?? 'Transitions'
    }));
  });

  const items = $derived(kind === 'effect' ? effects : transitions);
  const categories = $derived([...new Set(items.map((item) => item.group))]);
  const shown = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    return items.filter(
      (item) => !needle || item.label.toLowerCase().includes(needle) || item.id.toLowerCase().includes(needle)
    );
  });
  /* Grouped when browsing; a search is one flat list. */
  const sections = $derived.by<Array<{ group: string | null; items: Item[] }>>(() => {
    if (query.trim()) return [{ group: null, items: shown }];
    return categories
      .map((group) => ({ group, items: shown.filter((item) => item.group === group) }))
      .filter((section) => section.items.length);
  });

  const key = (item: Item): string => `${item.kind}:${item.id}`;

  function apply(event: MouseEvent | KeyboardEvent, item: Item): void {
    if ('detail' in event && event.detail > 1) return;
    const layer = PM.firstSel?.();
    if (!layer) {
      status = 'Select a layer first';
      PM.toast?.(status);
      return;
    }
    const command =
      item.kind === 'effect'
        ? { type: 'add_effect', target: layer.id, effect: item.id }
        : { type: 'set_transition', layer: layer.id, edge: 'in', transition: { type: item.id } };
    PM.Edit.apply(command, { label: `Add ${item.label}`, origin: 'fx-browser' });
    PM.Inspector?.refresh?.();
    PM.invalidate();
    status = `Added ${item.label}`;
  }

  function dragStart(event: DragEvent, item: Item): void {
    event.dataTransfer?.setData(
      'application/x-powermove-fx',
      JSON.stringify({ kind: item.kind, id: item.id, label: item.label })
    );
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'copy';
  }

  function listKey(event: KeyboardEvent): void {
    const rows = [...(list?.querySelectorAll<HTMLButtonElement>('.fxb-row') ?? [])];
    const index = rows.indexOf(event.currentTarget as HTMLButtonElement);
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
        placeholder="Search {kind === 'effect' ? 'effects' : 'transitions'}"
        aria-label="Search {kind === 'effect' ? 'effects' : 'transitions'}"
        bind:value={fxBrowser.query}
        onkeydown={(event) => { if (event.key === 'Escape') { fxBrowser.searchOpen = false; fxBrowser.query = ''; } }}
      />
    </div>
  {/if}

  <div class="fxb-list" bind:this={list} role="group" aria-label={kind === 'effect' ? 'Effects' : 'Transitions'}>
    {#each sections as section (section.group ?? '')}
      {#if section.group}<div class="sec fxb-sec">{section.group}</div>{/if}
      {#each section.items as item, index (key(item))}
        <button
          type="button"
          class="fxb-row"
          draggable="true"
          tabindex={index === 0 && section === sections[0] ? 0 : -1}
          data-kind={item.kind}
          data-id={item.id}
          title="Click to add to the selected layer · drag onto a layer"
          onclick={(event) => apply(event, item)}
          onkeydown={listKey}
          ondragstart={(event) => dragStart(event, item)}
        >
          <span class="fxb-label">{item.label}</span>
          <span class="fxb-add" aria-hidden="true">
            <svg viewBox="0 0 16 16"><path d="M8 3.5v9M3.5 8h9"/></svg>
          </span>
        </button>
      {/each}
    {:else}
      <div class="fxb-empty">No {kind === 'effect' ? 'effects' : 'transitions'} match “{query}”</div>
    {/each}
  </div>

  <span class="panel-sr-only" role="status">{status}</span>
</div>
