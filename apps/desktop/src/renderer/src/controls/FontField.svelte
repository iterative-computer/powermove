<script lang="ts">
  import { sel } from '../state/selection.svelte';
  import { tick } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import './controls.css';
  import type { PowermoveAPI } from '../kernel/api';

  let {
    api,
    get,
    edit,
    label,
    weight,
    onChange,
    mixed
  }: {
    api: PowermoveAPI;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
    weight?: () => number;
    onChange?: (value: string) => void;
    mixed?: (edit: EditBinding, value: unknown) => boolean;
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, transport.time, String(get() ?? '')));
  const isMixed=$derived((sel.layers,doc.tick.values,doc.proj,transport.time,mixed?.(edit,value)??false));
  const gesture = $derived(new EditGesture(api, edit));
  let trigger = $state<HTMLButtonElement>();
  let menu = $state<HTMLDivElement>();
  let search = $state<HTMLInputElement>();
  let open = $state(false);
  let query = $state('');
  let menuLeft = $state(6);
  let menuTop = $state(6);
  let menuWidth = $state(230);
  const allFonts = $derived(api.media.fonts.options(value));
  const matches = $derived(allFonts.filter((name) => !query.trim() || name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())));

  const familyStyle = (name: string): string => `"${name.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;

  function show(): void {
    if (open) { close(); return; }
    api.ui.closeMenus();
    open = true;
    query = '';
    void tick().then(() => {
      const rect = trigger?.getBoundingClientRect();
      if (rect) {
        menuWidth = Math.max(230, Math.min(310, rect.width + 120));
        menuLeft = api.util.clamp(rect.right - menuWidth, 6, window.innerWidth - menuWidth - 6);
        menuTop = api.util.clamp(rect.bottom + 5, 6, window.innerHeight - (menu?.offsetHeight ?? 0) - 6);
      }
      search?.focus();
    });
  }

  function close(restore = true): void {
    open = false;
    if (restore) void tick().then(() => trigger?.focus());
  }

  function choose(name: string): void {
    gesture.once(name);
    api.transport.invalidate();
    void api.media.fonts.ensure(name, weight?.() ?? 400);
    onChange?.(name);
    api.ui.closeMenus();
    close();
  }

  function menuKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); close(); }
    if (event.key === 'Enter') {
      const first = matches[0];
      if (first) { event.preventDefault(); choose(first); }
    }
  }

  $effect(() => {
    if (!open) return;
    const outside = (event: PointerEvent): void => {
      if (menu && !menu.contains(event.target as Node) && event.target !== trigger) close(false);
    };
    const id = window.setTimeout(() => document.addEventListener('pointerdown', outside), 0);
    return () => {
      window.clearTimeout(id);
      document.removeEventListener('pointerdown', outside);
    };
  });
</script>

<button
  bind:this={trigger}
  type="button"
  class="sel font-select"
  title={value}
  aria-haspopup="dialog"
  aria-expanded={open}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  style:font-family={familyStyle(value)}
  onpointerdown={(event) => event.stopPropagation()}
  onclick={show}
>{isMixed?'Mixed':value}</button>

{#if open}
  <div bind:this={menu} class="drop font-drop pm-control-font-drop" role="dialog" aria-label={label ?? 'Choose font'} tabindex="-1" style:left={`${menuLeft}px`} style:top={`${menuTop}px`} style:width={`${menuWidth}px`} onkeydown={menuKeydown}>
    <input
      bind:this={search}
      bind:value={query}
      class="font-search pm-control-input"
      placeholder="Search fonts"
      aria-label="Search fonts"
      autocomplete="off"
      spellcheck="false"
    />
    <div class="font-results" role="listbox" aria-label="Fonts">
      {#each matches.slice(0, 180) as name (name)}
        <button
          type="button"
          role="option"
          aria-selected={name === value}
          class="di font-item"
          class:on={name === value}
          title={name}
          style:font-family={familyStyle(name)}
          onclick={() => choose(name)}
        >{name}</button>
      {:else}
        <div class="font-empty">No matching fonts</div>
      {/each}
      {#if matches.length > 180}<div class="font-empty">{matches.length - 180} more · keep typing to narrow</div>{/if}
    </div>
  </div>
{/if}
