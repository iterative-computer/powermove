<script lang="ts">
  import { shortcutLabel } from '../platform';
  import { onMount, tick, untrack } from 'svelte';

  import { paletteEntries, type PaletteEntry } from './palette-model';
  import type { OverlayPM } from './types';

  let { PM, onclose }: { PM: OverlayPM; onclose: () => void } = $props();
  let palette: HTMLElement;
  let input: HTMLInputElement;
  let list: HTMLElement;
  let glider: HTMLElement;
  let gliderOn = $state(false);
  let query = $state('');
  /* The selection is a row, not a position: rows that land late can move
     it, and Enter must run the row that was highlighted. None means the top;
     a highlighted row that leaves the list leaves nothing highlighted. */
  let selection = $state.raw<{ id: string; index: number } | null>(null);
  /* Asynchronous answers (sandboxed providers and `when`s) arrive after the
     rows for a query; they count only while that query is the one asked. */
  let latest: object | null = null;
  let late = $state.raw<{ asked: object; items: PaletteEntry[] } | null>(null);
  let asked = $derived.by(() => {
    const request = { items: [] as PaletteEntry[] };
    latest = request;
    request.items = paletteEntries(PM, query, (next) => {
      if (latest !== request) return;
      // The top row the person sees stays the one Enter runs.
      if (!selection && items[0]) selection = { id: items[0].id, index: 0 };
      late = { asked: request, items: next };
    });
    return request;
  });
  let items = $derived(late?.asked === asked ? late.items : asked.items);
  let selected = $derived.by(() => {
    if (!selection) return 0;
    if (items[selection.index]?.id === selection.id) return selection.index;
    return items.findIndex(item => item.id === selection!.id);
  });
  // Group consecutive entries by category while keeping the flat index that
  // keyboard navigation and aria-activedescendant rely on.
  let groups = $derived.by(() => {
    const out: { cat: string; rows: { item: PaletteEntry; index: number }[] }[] = [];
    items.forEach((item, index) => {
      const last = out[out.length - 1];
      if (last && last.cat === item.cat) last.rows.push({ item, index });
      else out.push({ cat: item.cat, rows: [{ item, index }] });
    });
    return out;
  });
  const listId = 'pm-command-palette-list';

  export function element(): HTMLElement {
    return palette;
  }

  function inputChanged(event: Event): void {
    query = (event.currentTarget as HTMLInputElement).value;
    selection = null;
  }

  function select(index: number): void {
    const item = items[index];
    if (item && selection?.id === item.id && selection.index === index) return;
    selection = item ? { id: item.id, index } : null;
  }

  function run(item: PaletteEntry | undefined): void {
    onclose();
    item?.run();
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'ArrowDown') {
      select(Math.min(items.length - 1, selected + 1));
      event.preventDefault();
    } else if (event.key === 'ArrowUp') {
      select(Math.max(0, selected - 1));
      event.preventDefault();
    } else if (event.key === 'Home') {
      select(0);
      event.preventDefault();
    } else if (event.key === 'End') {
      select(Math.max(0, items.length - 1));
      event.preventDefault();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (selected >= 0) run(items[selected]);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      onclose();
    } else if (event.key === 'Tab') {
      event.preventDefault();
    }
  }

  onMount(() => input.focus({ preventScroll: true }));

  // One highlight surface glides to the selected row; hover and keyboard both
  // write `selection`, so a single fill is ever visible.
  $effect(() => {
    const index = selected;
    void items;
    tick().then(() => {
      const row = list?.querySelector<HTMLElement>(`#pm-palette-item-${index}`);
      if (!row || !glider) { gliderOn = false; return; }
      const first = !untrack(() => gliderOn);
      if (first) glider.style.transition = 'none';
      glider.style.transform = `translateY(${row.offsetTop}px)`;
      glider.style.height = `${row.offsetHeight}px`;
      if (first) { void glider.offsetHeight; glider.style.transition = ''; }
      gliderOn = true;
      row.scrollIntoView({ block: 'nearest' });
    });
  });
</script>

<div
  bind:this={palette}
  class="modal"
  id="palette"
  role="dialog"
  aria-modal="true"
  aria-label="Command palette"
  data-svelte-overlay-palette="true"
>
  <div class="field">
    <svg class="psearch" viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5 14 14" />
    </svg>
    <input
      bind:this={input}
      value={query}
      role="combobox"
      aria-label="Search commands, layers, workspaces, and effects"
      aria-controls={listId}
      aria-expanded="true"
      aria-autocomplete="list"
      aria-activedescendant={items[selected] ? `pm-palette-item-${selected}` : undefined}
      placeholder="Search commands, layers, workspaces…"
      oninput={inputChanged}
      onkeydown={keydown}
    />
  </div>
  <div class="plist" id={listId} role="listbox" aria-label="Command results" bind:this={list}>
    <div class="pglider" class:on={gliderOn} bind:this={glider} aria-hidden="true"></div>
    {#each groups as group (group.cat + group.rows[0]?.index)}
      <div class="pgroup" role="presentation">{group.cat}</div>
      {#each group.rows as { item, index } (index)}
        <div
          class:pitem={true}
          class:on={index === selected}
          id={`pm-palette-item-${index}`}
          role="option"
          tabindex="-1"
          aria-selected={index === selected}
          onclick={() => run(item)}
          onkeydown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') run(item);
          }}
          onpointermove={() => select(index)}
        >
          <span>{item.label}</span>
          {#if item.kb}<span class="kb">{shortcutLabel(item.kb)}</span>{/if}
        </div>
      {/each}
    {:else}
      <div class="empty">No matches</div>
    {/each}
  </div>
</div>
