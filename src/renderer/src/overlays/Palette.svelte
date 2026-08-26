<script lang="ts">
  import { onMount } from 'svelte';

  import { paletteEntries, type PaletteEntry } from './palette-model';
  import type { OverlayPM } from './types';

  let { PM, onclose }: { PM: OverlayPM; onclose: () => void } = $props();
  let palette: HTMLElement;
  let input: HTMLInputElement;
  let query = $state('');
  let selected = $state(0);
  let items = $derived(paletteEntries(PM, query));
  const listId = 'pm-command-palette-list';

  export function element(): HTMLElement {
    return palette;
  }

  function inputChanged(event: Event): void {
    query = (event.currentTarget as HTMLInputElement).value;
    selected = 0;
  }

  function run(item: PaletteEntry | undefined): void {
    onclose();
    item?.run();
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'ArrowDown') {
      selected = Math.min(items.length - 1, selected + 1);
      event.preventDefault();
    } else if (event.key === 'ArrowUp') {
      selected = Math.max(0, selected - 1);
      event.preventDefault();
    } else if (event.key === 'Home') {
      selected = 0;
      event.preventDefault();
    } else if (event.key === 'End') {
      selected = Math.max(0, items.length - 1);
      event.preventDefault();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      run(items[selected]);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      onclose();
    } else if (event.key === 'Tab') {
      event.preventDefault();
    }
  }

  onMount(() => input.focus({ preventScroll: true }));
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
  <div class="plist" id={listId} role="listbox" aria-label="Command results">
    {#each items as item, index (index)}
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
        onpointermove={() => selected = index}
      >
        <span class="cat">{item.cat}</span>
        <span>{item.label}</span>
        {#if item.kb}<span class="kb">{item.kb}</span>{/if}
      </div>
    {:else}
      <div class="empty">No matches</div>
    {/each}
  </div>
</div>
