<script lang="ts">
  import { cssFontStack } from '../typography/font-stack';
  import { sel } from '../state/selection.svelte';
  import { tick } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import './controls.css';
  import type { PowermoveAPI } from '../kernel/api';

  /* The font picker is the app's dropdown sheet (.pm-menu, shared with the
     select listbox and the agent's thread picker): a quiet search field over
     a scrolling list, one hover layer gliding between rows, the current row
     checked. Each row previews its own face at the UI size. */

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
  const isMixed = $derived((sel.layers, doc.tick.values, doc.proj, transport.time, mixed?.(edit, value) ?? false));
  const gesture = $derived(new EditGesture(api, edit));

  let trigger = $state<HTMLButtonElement>();
  let popup = $state<HTMLDivElement>();
  let list = $state<HTMLDivElement>();
  let glider = $state<HTMLDivElement>();
  let search = $state<HTMLInputElement>();
  let open = $state(false);
  let shown = $state(false);
  let side = $state<'bottom' | 'top'>('bottom');
  let query = $state('');
  let left = $state(0);
  let top = $state(0);
  let width = $state(248);
  let maxHeight = $state(360);
  let dismissByPress = false;
  let closeTimer: ReturnType<typeof setTimeout> | undefined;

  const GAP = 6;
  const EDGE = 8;
  const MAX_HEIGHT = 360;
  const PAGE = 80;
  let visible = $state(PAGE);

  const allFonts = $derived(api.media.fonts.options(value));
  const matches = $derived.by(() => {
    const needle = query.trim().toLocaleLowerCase();
    return needle ? allFonts.filter((name) => name.toLocaleLowerCase().includes(needle)) : allFonts;
  });
  const page = $derived(matches.slice(0, visible));
  const reduce = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

  function finishClose(): void {
    shown = false;
    if (closeTimer) { clearTimeout(closeTimer); closeTimer = undefined; }
    detach();
  }

  function close(focusTrigger = true): void {
    if (!open) return;
    open = false;
    rest();
    if (focusTrigger) trigger?.focus();
    if (reduce() || !popup) { finishClose(); return; }
    popup.dataset.state = 'closed';
    popup.addEventListener('animationend', finishClose, { once: true });
    closeTimer = setTimeout(finishClose, 200);
  }

  function place(): void {
    if (!trigger || !popup) return;
    const a = trigger.getBoundingClientRect();
    const vw = window.innerWidth, vh = window.innerHeight;
    width = Math.max(Math.ceil(a.width), 248);
    const h = Math.min(popup.offsetHeight || MAX_HEIGHT, MAX_HEIGHT);
    const below = Math.max(0, vh - a.bottom - EDGE - GAP);
    const above = Math.max(0, a.top - EDGE - GAP);
    side = below >= h || below >= above ? 'bottom' : 'top';
    maxHeight = Math.min(MAX_HEIGHT, side === 'bottom' ? below : above);
    const height = Math.min(h, maxHeight);
    top = side === 'bottom' ? a.bottom + GAP : a.top - GAP - height;
    // Right-aligned to the trigger, like the select listbox in the inspector.
    left = Math.max(EDGE, Math.min(a.right - width, vw - width - EDGE));
  }

  async function toggle(event: MouseEvent): Promise<void> {
    const dismiss = open || (event.detail > 0 && dismissByPress);
    dismissByPress = false;
    if (dismiss) { close(); return; }
    if (closeTimer) finishClose();
    api.ui.closeMenus();
    query = '';
    visible = PAGE;
    shown = true;
    open = true;
    await tick();
    if (!popup) return;
    popup.dataset.state = '';
    place();
    popup.dataset.state = 'open';
    attach();
    search?.focus();
    // Open on the current face so it is in view.
    const current = popup.querySelector<HTMLElement>('.font-menu-row[aria-selected="true"]');
    current?.scrollIntoView({ block: 'center' });
  }

  function onPointerDown(event: PointerEvent): void {
    const t = event.target as Node;
    if (popup?.contains(t) || trigger?.contains(t)) return;
    close(false);
  }
  function onScroll(event: Event): void {
    if (popup?.contains(event.target as Node)) return;
    close(false);
  }
  const onWindowClose = (): void => close(false);
  function attach(): void {
    document.addEventListener('pointerdown', onPointerDown, true);
    document.addEventListener('scroll', onScroll, true);
    window.addEventListener('resize', onWindowClose);
    window.addEventListener('blur', onWindowClose);
  }
  function detach(): void {
    document.removeEventListener('pointerdown', onPointerDown, true);
    document.removeEventListener('scroll', onScroll, true);
    window.removeEventListener('resize', onWindowClose);
    window.removeEventListener('blur', onWindowClose);
  }

  let gliderOn = false;
  function glideTo(row: HTMLElement): void {
    if (!glider) return;
    if (!gliderOn) glider.style.transition = 'none';
    glider.style.transform = `translateY(${row.offsetTop}px)`;
    glider.style.height = `${row.offsetHeight}px`;
    if (!gliderOn) { void glider.offsetHeight; glider.style.transition = ''; }
    gliderOn = true;
    glider.classList.add('on');
  }
  function rest(): void {
    glider?.classList.remove('on');
    gliderOn = false;
  }
  function onRowPointer(event: PointerEvent): void {
    const row = (event.target as HTMLElement).closest<HTMLElement>('.font-menu-row');
    if (row) glideTo(row);
  }
  function onListScroll(): void {
    if (!list || visible >= matches.length) return;
    if (list.scrollTop + list.clientHeight > list.scrollHeight - 120) visible += PAGE;
  }

  function rows(): HTMLElement[] {
    return Array.from(popup?.querySelectorAll<HTMLElement>('.font-menu-row') || []);
  }
  function step(from: HTMLElement | null, delta: number): void {
    const all = rows();
    if (!all.length) return;
    const index = from ? all.indexOf(from) : -1;
    const next = index < 0 ? (delta > 0 ? 0 : all.length - 1) : index + delta;
    if (next < 0) { search?.focus(); rest(); return; }
    const row = all[Math.min(next, all.length - 1)];
    if (!row) return;
    row.focus({ preventScroll: true });
    row.scrollIntoView({ block: 'nearest' });
    glideTo(row);
  }

  function choose(name: string): void {
    close();
    if (name === value && !isMixed) return;
    gesture.once(name);
    api.transport.invalidate();
    void api.media.fonts.ensure(name, weight?.() ?? 400);
    onChange?.(name);
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    const target = event.target as HTMLElement;
    const row = target.closest<HTMLElement>('.font-menu-row');
    if (event.key === 'Escape') { event.preventDefault(); close(); return; }
    if (event.key === 'Tab') { event.preventDefault(); close(); return; }
    if (event.key === 'ArrowDown') { event.preventDefault(); step(row, 1); return; }
    if (event.key === 'ArrowUp') { event.preventDefault(); step(row, -1); return; }
    if (event.key === 'Enter' && target === search) { event.preventDefault(); if (matches[0]) choose(matches[0]); }
  }
</script>

<button
  bind:this={trigger}
  type="button"
  class="sel font-select"
  title={value}
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  style:font-family={cssFontStack(value)}
  style:font-weight={weight ? String(Math.min(700, Math.max(400, weight()))) : undefined}
  onpointerdown={(event) => { event.stopPropagation(); dismissByPress = open; }}
  onpointercancel={() => { dismissByPress = false; }}
  onclick={toggle}
>{isMixed ? 'Mixed' : value}</button>

{#if shown}
  <div
    class="pm-menu font-menu"
    bind:this={popup}
    role="dialog"
    tabindex="-1"
    aria-label={label ?? 'Choose font'}
    data-side={side}
    style:left={`${left}px`}
    style:top={`${top}px`}
    style:width={`${width}px`}
    style:max-height={`${maxHeight}px`}
    onkeydown={onKeydown}
  >
    <input
      class="font-menu-search"
      type="text"
      bind:this={search}
      bind:value={query}
      oninput={() => { visible = PAGE; }}
      placeholder="Search fonts…"
      aria-label="Search fonts"
      autocomplete="off"
      spellcheck="false"
    />
    <div
      class="font-menu-list"
      bind:this={list}
      role="listbox"
      aria-label="Fonts"
      tabindex="-1"
      onpointermove={onRowPointer}
      onpointerleave={rest}
      onscroll={onListScroll}
    >
      <div class="pm-menu-glider font-menu-glider" bind:this={glider} aria-hidden="true"></div>
      {#each page as name (name)}
        <div
          class="pm-menu-item font-menu-row"
          role="option"
          tabindex="-1"
          aria-selected={!isMixed && name === value}
          title={name}
          onclick={() => choose(name)}
          onfocus={(event) => glideTo(event.currentTarget as HTMLElement)}
          onkeydown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(name); } }}
        >
          <span class="pm-menu-label font-menu-name" style:font-family={cssFontStack(name)}>{name}</span>
          <svg class="pm-menu-check" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </div>
      {:else}
        <p class="font-menu-empty">No fonts match “{query.trim()}”.</p>
      {/each}
    </div>
  </div>
{/if}

<style>
  .font-select { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .font-menu { display: flex; flex-direction: column; overflow: hidden; box-sizing: border-box; font-weight: var(--fw-regular); }
  .font-menu-search { flex-shrink: 0; margin: 0 0 2px; height: var(--ctl-h); padding: 0 8px; border: 0; border-radius: 6px; background: transparent; color: var(--tx); font: inherit; }
  .font-menu-search::placeholder { color: var(--tx-3); }
  .font-menu-search:focus { outline: none; }
  .font-menu-list { position: relative; flex: 1; min-height: 0; overflow-y: auto; scrollbar-width: thin; }
  .font-menu-glider { left: 0; right: 0; }
  .font-menu-row:focus-visible { outline: none; }
  /* Each row shows its own face; keep the size and weight of the sheet so
     display faces with tall metrics do not change the rhythm. */
  .font-menu-name { font-size: 13px; line-height: 1; }
  .font-menu-empty { margin: 4px 8px 8px; color: var(--tx-3); font-size: var(--fs-sm); }
</style>
