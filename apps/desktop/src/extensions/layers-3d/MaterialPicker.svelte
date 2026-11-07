<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import type { PowermoveAPI } from 'powermove';
  import MaterialThumb from './MaterialThumb.svelte';
  import { filterGroups, sameMap, slotSpec, type MaterialMapValue, type MaterialSlot, type SourceFilter, type SourceGroup, type SourceOption } from './materials';

  /* The Material source menu: an AppKit-style pop-up sheet with a search
     field, an All / Comps / Footage filter (as in After Effects), grouped
     sources, the current one checked at the leading edge, and sources that
     can't be used shown dimmed with the reason underneath. */
  let { api, layerId, rect, slot, materialName, groups, current, onPick, onImage, onClose }: {
    api: PowermoveAPI;
    layerId: string;
    rect: { left: number; top: number; bottom: number; width: number };
    slot: MaterialSlot;
    materialName: string;
    groups: SourceGroup[];
    current: MaterialMapValue | null;
    onPick: (map: MaterialMapValue | null) => void;
    onImage: () => void;
    onClose: (outside?: boolean) => void;
  } = $props();

  const EDGE = 8, GAP = 4, MAX = 440;
  let sheet = $state<HTMLDivElement>();
  let search = $state<HTMLInputElement>();
  let query = $state('');
  let filter = $state<SourceFilter>('all');
  let active = $state<string | null>(null);
  let left = $state(0), top = $state(0), maxHeight = $state(MAX), side = $state<'bottom' | 'top'>('bottom');
  const width = $derived(Math.max(272, Math.round(rect.width)));
  const spec = $derived(slotSpec(slot));
  const visible = $derived(filterGroups(groups, filter, query));
  const enabled = $derived(visible.flatMap(group => group.options.filter(option => !option.disabled)));
  const filters: Array<{ id: SourceFilter; label: string }> = [{ id: 'all', label: 'All' }, { id: 'comps', label: 'Comps' }, { id: 'footage', label: 'Footage' }];

  function place(): void {
    const vh = window.innerHeight, vw = window.innerWidth;
    const below = vh - rect.bottom - GAP - EDGE, above = rect.top - GAP - EDGE;
    const h = Math.min(sheet?.scrollHeight ?? MAX, MAX);
    side = below >= h || below >= above ? 'bottom' : 'top';
    maxHeight = Math.max(160, Math.min(MAX, side === 'bottom' ? below : above));
    top = side === 'bottom' ? rect.bottom + GAP : Math.max(EDGE, rect.top - GAP - Math.min(h, maxHeight));
    left = Math.max(EDGE, Math.min(rect.left, vw - width - EDGE));
  }

  function pick(option: SourceOption): void {
    if (option.disabled) return;
    onClose();
    if (!sameMap(option.map, current)) onPick(option.map);
  }

  function move(delta: number): void {
    if (!enabled.length) return;
    const index = enabled.findIndex(option => option.id === active);
    const next = enabled[index < 0 ? (delta > 0 ? 0 : enabled.length - 1) : Math.max(0, Math.min(enabled.length - 1, index + delta))]!;
    active = next.id;
    sheet?.querySelector<HTMLElement>(`[data-option="${CSS.escape(next.id)}"]`)?.scrollIntoView({ block: 'nearest' });
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape' || event.key === 'Tab') { event.preventDefault(); onClose(); return; }
    if (event.key === 'ArrowDown') { event.preventDefault(); move(1); return; }
    if (event.key === 'ArrowUp') { event.preventDefault(); move(-1); return; }
    if (event.key === 'Enter') {
      event.preventDefault();
      const option = enabled.find(item => item.id === active) ?? (query.trim() ? enabled.find(item => item.id !== 'original') : undefined);
      if (option) pick(option);
    }
  }

  const outside = (event: PointerEvent) => { if (!sheet?.contains(event.target as Node)) onClose(true); };
  const scrolled = (event: Event) => { if (!sheet?.contains(event.target as Node)) onClose(); };
  const dismiss = () => onClose();
  onMount(async () => {
    active = enabled.find(option => sameMap(option.map, current))?.id ?? null;
    await tick();
    place();
    search?.focus({ preventScroll: true });
    if (active) sheet?.querySelector<HTMLElement>(`[data-option="${CSS.escape(active)}"]`)?.scrollIntoView({ block: 'nearest' });
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('scroll', scrolled, true);
    window.addEventListener('resize', dismiss);
    window.addEventListener('blur', dismiss);
  });
  onDestroy(() => {
    document.removeEventListener('pointerdown', outside, true);
    document.removeEventListener('scroll', scrolled, true);
    window.removeEventListener('resize', dismiss);
    window.removeEventListener('blur', dismiss);
  });
</script>

<div
  class="pm-menu material-picker"
  bind:this={sheet}
  role="dialog"
  tabindex="-1"
  aria-label={`${spec.label} source for ${materialName}`}
  data-material-picker={slot}
  data-side={side}
  style:left={`${left}px`}
  style:top={`${top}px`}
  style:width={`${width}px`}
  style:max-height={`${maxHeight}px`}
  onkeydown={keydown}
>
  <div class="mp-head">
    <div class="mp-search">
      <span class="mp-search-icon" aria-hidden="true">{@html api.ui.icon('search')}</span>
      <input bind:this={search} bind:value={query} type="text" placeholder="Search layers, comps, images" aria-label="Search sources"
        autocomplete="off" spellcheck="false" oninput={() => { active = null; }} />
    </div>
    {#if spec.layers}
      <div class="mp-filter" role="radiogroup" aria-label="Show">
        {#each filters as item (item.id)}
          <button type="button" role="radio" aria-checked={filter === item.id} class:on={filter === item.id}
            onclick={() => { filter = item.id; active = null; search?.focus({ preventScroll: true }); }}>{item.label}</button>
        {/each}
      </div>
    {/if}
  </div>
  <div class="mp-list" role="listbox" tabindex="-1" aria-label="Sources" onpointerleave={() => { active = null; }}>
    {#each visible as group (group.id)}
      {#if group.id !== 'original'}<div class="pm-menu-sep" role="separator"></div>{/if}
      {#if group.label}<div class="pm-menu-heading" role="presentation">{group.label}</div>{/if}
      {#each group.options as option (option.id)}
        {@const chosen = sameMap(option.map, current) && !option.disabled}
        <div
          class="pm-menu-item mp-item"
          class:has-detail={!!option.detail}
          role="option"
          tabindex="-1"
          data-option={option.id}
          aria-selected={chosen}
          aria-disabled={option.disabled ? 'true' : undefined}
          data-active={active === option.id ? '' : undefined}
          title={option.disabled ? `${option.label}: ${option.detail}` : undefined}
          onpointermove={() => { if (!option.disabled) active = option.id; }}
          onclick={() => pick(option)}
          onkeydown={(event) => { if (event.key === ' ') { event.preventDefault(); pick(option); } }}
        >
          <svg class="pm-menu-check" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5 6.5 11.5 12.5 4.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
          {#if option.map != null && !option.disabled}
            <!-- The source's picture, once the engine has drawn it. -->
            <span class="pm-menu-icon mp-icon mp-thumb" aria-hidden="true">
              <MaterialThumb {api} {layerId} map={option.map}>
                {#snippet placeholder()}{@html api.ui.icon(option.icon ?? 'image')}{/snippet}
              </MaterialThumb>
            </span>
          {:else if option.icon}<span class="pm-menu-icon mp-icon" aria-hidden="true">{@html api.ui.icon(option.icon)}</span>{:else}<span class="pm-menu-icon mp-icon mp-original" aria-hidden="true"></span>{/if}
          {#if option.detail}
            <span class="pm-menu-text"><span class="pm-menu-label">{option.label}</span><span class="pm-menu-detail">{option.detail}</span></span>
          {:else}
            <span class="pm-menu-label">{option.label}</span>
          {/if}
          {#if option.hidden && !option.disabled}<span class="mp-tag" title="Hidden layers still work as materials">Hidden</span>{/if}
        </div>
      {/each}
      {#if group.id === 'images'}
        <div class="pm-menu-item mp-item" role="option" tabindex="-1" aria-selected="false" data-option="choose-image"
          data-active={active === 'choose-image' ? '' : undefined}
          onpointermove={() => { active = 'choose-image'; }} onclick={() => { onClose(); onImage(); }}
          onkeydown={(event) => { if (event.key === ' ') { event.preventDefault(); onClose(); onImage(); } }}>
          <span class="pm-menu-icon mp-icon" aria-hidden="true">{@html api.ui.icon('plus')}</span><span class="pm-menu-label">Choose Image File…</span>
        </div>
      {/if}
    {:else}
      <p class="mp-empty">Nothing matches “{query.trim()}”.</p>
    {/each}
    {#if !spec.layers && !query.trim()}
      <p class="mp-note">Layers and comps can drive Base Color and Emissive. {spec.label} takes an image.</p>
    {/if}
  </div>
</div>

<style>
  .material-picker { display: flex; flex-direction: column; overflow: hidden; padding: 0; }
  .mp-head { flex: none; display: flex; flex-direction: column; gap: 6px; padding: 6px 6px 4px; }
  .mp-search { display: flex; align-items: center; gap: 6px; height: 26px; padding: 0 8px; border-radius: 7px; background: var(--menu-sep, rgb(var(--ink-rgb) / .06)); }
  .mp-search-icon { display: grid; place-items: center; width: 13px; height: 13px; color: var(--menu-tx-2); flex: none; }
  .mp-search-icon :global(svg) { width: 13px; height: 13px; }
  .mp-search input { flex: 1; min-width: 0; height: 100%; padding: 0; border: 0; background: transparent; color: var(--menu-tx); font: inherit; font-size: 12.5px; outline: none; cursor: text; user-select: text; -webkit-user-select: text; }
  .mp-search input::placeholder { color: var(--menu-tx-2); }
  .mp-search:focus-within { box-shadow: 0 0 0 2px color-mix(in srgb, var(--menu-accent, var(--accent)) 55%, transparent); }
  .mp-filter { display: grid; grid-template-columns: repeat(3, 1fr); gap: 2px; padding: 2px; border-radius: 7px; background: var(--menu-sep, rgb(var(--ink-rgb) / .06)); }
  .mp-filter button { height: 20px; padding: 0; border: 0; border-radius: 5px; background: transparent; color: var(--menu-tx-2); font: inherit; font-size: 11.5px; font-weight: 500; }
  .mp-filter button.on { background: var(--bg-float); color: var(--menu-tx); box-shadow: 0 0 0 .5px rgb(var(--ink-rgb) / .12), 0 1px 2px rgb(var(--ink-rgb) / .1); }
  :global(:root[data-theme="dark"]) .mp-filter button.on { background: rgb(255 255 255 / .16); box-shadow: none; }
  .mp-list { flex: 1; min-height: 0; overflow-y: auto; padding: 0 5px 5px; scrollbar-width: thin; }
  .mp-list > .pm-menu-sep:first-child { display: none; }
  .mp-item[data-active] { background: var(--menu-accent); color: #fff; }
  .mp-item[data-active] :is(.pm-menu-icon, .pm-menu-detail, .pm-menu-check) { color: #fff; }
  .mp-item[aria-disabled='true'] { pointer-events: auto; }
  .mp-item[aria-disabled='true'] .pm-menu-icon { opacity: .5; }
  .mp-tag { flex: none; margin-left: auto; padding: 0 5px; border-radius: 4px; background: var(--menu-sep, rgb(var(--ink-rgb) / .06)); color: var(--menu-tx-2); font-size: 10.5px; line-height: 15px; }
  .mp-item[data-active] .mp-tag { background: rgb(255 255 255 / .2); color: #fff; }
  .mp-icon :global(svg) { width: 14px; height: 14px; }
  .mp-icon { display: grid; place-items: center; }
  /* On the highlighted row the picture keeps a light rim against the accent. */
  .mp-item[data-active] .mp-thumb :global(img) { box-shadow: 0 0 0 1px rgb(255 255 255 / .55); }
  .mp-original::before { content: ''; width: 10px; height: 10px; border-radius: 50%; background: radial-gradient(circle at 35% 30%, rgb(255 255 255 / .9), rgb(var(--ink-rgb) / .35) 70%); box-shadow: inset 0 0 0 .5px rgb(var(--ink-rgb) / .3); }
  .mp-empty, .mp-note { margin: 6px 11px 4px; color: var(--menu-tx-2); font-size: 11.5px; line-height: 1.4; white-space: normal; }
</style>
