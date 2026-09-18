<script lang="ts">
  import { tick } from 'svelte';
  import { agentState } from './agent-state.svelte';
  import { relativeOpened } from './threads';

  let { PM }: { PM: Record<string, any> } = $props();
  let trigger = $state<HTMLButtonElement>();
  let popup = $state<HTMLDivElement>();
  let list = $state<HTMLDivElement>();
  let glider = $state<HTMLDivElement>();
  let search: HTMLInputElement | undefined = $state();
  // `open` is the logical state (aria-expanded flips at once); `shown` keeps
  // the sheet mounted while the close animation plays.
  let open = $state(false);
  let shown = $state(false);
  let side = $state<'bottom' | 'top'>('bottom');
  let dismissByPress = false;
  let query = $state('');
  let left = $state(0);
  let top = $state(0);
  let width = $state(248);
  let maxHeight = $state(360);
  let openedAt = $state(Date.now());
  // Long histories render in pages: the first page paints at once, the rest
  // arrives as the list scrolls, so a thousand threads never block the open.
  const PAGE = 60;
  let visible = $state(PAGE);
  let closeTimer: ReturnType<typeof setTimeout> | undefined;

  const GAP = 6;
  const EDGE = 8;
  const MAX_HEIGHT = 360;

  const blocked = $derived(agentState.threadSwitchBlocked);
  const hint = $derived(blocked ? 'Finish applying the current change to switch threads' : 'Switch thread');
  const running = $derived((agentState.threads || []).filter(thread => thread.busy).length);
  const threads = $derived(agentState.threads || []);
  const active = $derived(threads.find(thread => thread.id === agentState.threadId));
  const matches = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    return needle ? threads.filter(thread => thread.title.toLowerCase().includes(needle)) : threads;
  });
  const page = $derived(matches.slice(0, visible));
  const reduce = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

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
    // The sheet leaves the way it came; if the animation never fires, still clean up.
    popup.dataset.state = 'closed';
    popup.addEventListener('animationend', finishClose, { once: true });
    closeTimer = setTimeout(finishClose, 200);
  }

  function place(): void {
    if (!trigger || !popup) return;
    const a = trigger.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    width = Math.max(Math.ceil(a.width), 248);
    const h = Math.min(popup.offsetHeight || MAX_HEIGHT, MAX_HEIGHT);
    const below = Math.max(0, vh - a.bottom - EDGE - GAP);
    const above = Math.max(0, a.top - EDGE - GAP);
    side = below >= h || below >= above ? 'bottom' : 'top';
    maxHeight = Math.min(MAX_HEIGHT, side === 'bottom' ? below : above);
    const height = Math.min(h, maxHeight);
    top = side === 'bottom' ? a.bottom + GAP : a.top - GAP - height;
    left = Math.max(EDGE, Math.min(a.left, vw - width - EDGE));
  }

  async function toggle(event: MouseEvent): Promise<void> {
    const dismiss = open || (event.detail > 0 && dismissByPress);
    dismissByPress = false;
    if (dismiss) { close(); return; }
    if (closeTimer) finishClose();
    query = '';
    visible = PAGE;
    openedAt = Date.now();
    shown = true;
    open = true;
    await tick();
    if (!popup) return;
    popup.dataset.state = '';
    place();
    popup.dataset.state = 'open';
    attach();
    search?.focus();
  }

  // ── outside dismissal, like the select listbox ──
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

  // ── one hover layer glides between rows ──
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
    const row = (event.target as HTMLElement).closest<HTMLElement>('.thread-row');
    if (row) glideTo(row);
  }
  function onListScroll(): void {
    if (!list || visible >= matches.length) return;
    if (list.scrollTop + list.clientHeight > list.scrollHeight - 120) visible += PAGE;
  }

  function rows(): HTMLElement[] {
    return Array.from(popup?.querySelectorAll<HTMLElement>('.thread-row') || []);
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

  function choose(id: string): void {
    close(false);
    if (id !== agentState.threadId) PM.AgentUI?.switchThread(id);
  }

  function remove(id: string): void {
    const remaining = matches.filter(thread => thread.id !== id);
    PM.AgentUI?.deleteThread?.(id);
    if (remaining.length) search?.focus(); else close();
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    const target = event.target as HTMLElement;
    const row = target.closest<HTMLElement>('.thread-row');
    if (event.key === 'Escape') { event.preventDefault(); close(); return; }
    if (event.key === 'Tab') { event.preventDefault(); close(); return; }
    if (event.key === 'ArrowDown') { event.preventDefault(); step(row, 1); return; }
    if (event.key === 'ArrowUp') { event.preventDefault(); step(row, -1); return; }
    if (event.key === 'Enter' && target === search) { event.preventDefault(); if (matches[0]) choose(matches[0].id); }
  }
</script>

{#if agentState.threadId}
  <div class="thread-bar" role="group" aria-label="Agent threads">
    <button
      class="thread-trigger"
      type="button"
      bind:this={trigger}
      aria-label="Switch thread"
      aria-haspopup="listbox"
      aria-expanded={open}
      title={hint}
      disabled={blocked}
      onpointerdown={() => { dismissByPress = open; }}
      onpointercancel={() => { dismissByPress = false; }}
      onclick={toggle}
    >
      <span>{active?.title || 'New thread'}</span>
      {#if agentState.backgroundRuns}
        <span class="thread-running" title={`${agentState.backgroundRuns} thread${agentState.backgroundRuns === 1 ? '' : 's'} still working`}>
          <span class="thread-dot" aria-hidden="true"></span>{agentState.backgroundRuns}
        </span>
      {/if}
      <svg class="thread-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 6.25 3.5 3.5 3.5-3.5" /></svg>
    </button>
    <button class="thread-new" type="button" aria-label="New thread" title={blocked ? hint : 'New thread'} disabled={blocked}
      onclick={() => PM.AgentUI?.newThread()}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10" /></svg>
    </button>
  </div>
  {#if agentState.threadSaveError}<p class="thread-warning" role="status">Thread history couldn’t be saved. Keep this window open.</p>{/if}
{/if}

{#if shown}
  <div
    class="pm-menu thread-popup"
    bind:this={popup}
    role="dialog"
    tabindex="-1"
    aria-label="Threads"
    data-side={side}
    style:left={`${left}px`}
    style:top={`${top}px`}
    style:width={`${width}px`}
    style:max-height={`${maxHeight}px`}
    onkeydown={onKeydown}
  >
    <input
      class="thread-search"
      type="text"
      bind:this={search}
      bind:value={query}
      oninput={() => { visible = PAGE; }}
      placeholder="Search threads…"
      aria-label="Search threads"
      autocomplete="off"
      spellcheck="false"
    />
    <div
      class="thread-list"
      bind:this={list}
      role="listbox"
      aria-label="Threads"
      tabindex="-1"
      onpointermove={onRowPointer}
      onpointerleave={rest}
      onscroll={onListScroll}
    >
      <div class="pm-menu-glider thread-glider" bind:this={glider} aria-hidden="true"></div>
      {#each page as thread (thread.id)}
        <div
          class="thread-row"
          class:current={thread.id === agentState.threadId}
          role="option"
          tabindex="-1"
          aria-selected={thread.id === agentState.threadId}
          onclick={() => choose(thread.id)}
          onfocus={(event) => glideTo(event.currentTarget as HTMLElement)}
          onkeydown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(thread.id); } }}
        >
          <span class="thread-row-text">
            <span class="thread-row-title">
              {#if thread.busy}<span class="thread-dot" aria-hidden="true"></span>{/if}{thread.title}
            </span>
            <span class="thread-row-meta" class:current={thread.id === agentState.threadId}>
              {thread.busy ? 'Working…' : thread.id === agentState.threadId ? 'Current' : relativeOpened(thread.updatedAt ?? 0, openedAt)}
            </span>
          </span>
          <button
            class="thread-delete"
            type="button"
            aria-label={`Delete thread: ${thread.title}`}
            title={thread.busy ? 'Delete thread and stop its run' : 'Delete thread'}
            onclick={(event) => { event.stopPropagation(); remove(thread.id); }}
          >
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5 5 13h6l.5-8.5M6.75 6.75v3.75M9.25 6.75v3.75" /></svg>
          </button>
        </div>
      {:else}
        <p class="thread-empty">No threads match “{query.trim()}”.</p>
      {/each}
    </div>
  </div>
{/if}

<style>
  .thread-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    /* The trigger's text starts where the panel title does (header padding
       18px = 16px here + the trigger's own 2px). */
    margin: 0 10px 0 16px;
    padding: 0 0 6px;
    flex-shrink: 0;
  }
  .thread-trigger { display: flex; align-items: center; gap: 2px; flex: 1; min-width: 0; height: 28px; padding: 0 2px; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-2); font: inherit; font-size: var(--fs-sm); text-align: left; cursor: default; }
  .thread-trigger span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .thread-chevron { width: 14px; height: 14px; flex-shrink: 0; }
  .thread-new { display: grid; place-items: center; width: 28px; height: 28px; flex-shrink: 0; padding: 6px; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-3); cursor: default; }
  .thread-trigger:hover:not(:disabled), .thread-new:hover:not(:disabled) { color: var(--tx-2); }
  .thread-new:hover:not(:disabled) { background: var(--ink-1); }
  :disabled { opacity: var(--disabled); cursor: default; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
  .thread-warning { margin: 0 10px 6px 18px; font-size: var(--fs-xs); color: var(--accent); }

  /* The sheet is the select listbox (.pm-menu: raised sheet, open/close motion,
     glider) with a fixed search field above a scrolling list. */
  .thread-popup { display: flex; flex-direction: column; overflow: hidden; box-sizing: border-box; font-weight: var(--fw-regular); }
  .thread-search { flex-shrink: 0; margin: 0 0 2px; height: var(--ctl-h); padding: 0 8px; border: 0; border-radius: 6px; background: transparent; color: var(--tx); font: inherit; }
  .thread-search::placeholder { color: var(--tx-3); }
  .thread-search:focus { outline: none; }
  .thread-list { position: relative; flex: 1; min-height: 0; overflow-y: auto; scrollbar-width: thin; }
  .thread-glider { left: 0; right: 0; }
  .thread-row { position: relative; z-index: 1; display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-radius: 6px; }
  .thread-row:focus-visible { outline: none; }
  .thread-row.current { background: var(--ink-1); }
  .thread-row-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .thread-row-title { display: flex; align-items: center; gap: 5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .thread-running { display: flex; align-items: center; gap: 4px; flex-shrink: 0; padding: 0 5px; border-radius: 999px; background: var(--ink-1); color: var(--tx-3); font-size: var(--fs-xs); }
  .thread-dot { width: 6px; height: 6px; flex-shrink: 0; border-radius: 50%; background: var(--blue); animation: thread-pulse 1.6s ease-in-out infinite; }
  @keyframes thread-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
  @media (prefers-reduced-motion: reduce) { .thread-dot { animation: none; } }
  .thread-row-meta { color: var(--tx-3); font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .thread-row-meta.current { color: var(--blue); }
  .thread-delete { display: grid; place-items: center; width: 26px; height: 26px; flex-shrink: 0; padding: 0; border: 0; border-radius: 6px; background: transparent; color: var(--tx-3); opacity: 0; }
  .thread-row:hover .thread-delete, .thread-row.current .thread-delete, .thread-delete:focus-visible { opacity: 1; }
  .thread-delete:hover { color: var(--tx); background: var(--ink-2); }
  .thread-empty { margin: 4px 8px 8px; color: var(--tx-3); font-size: var(--fs-sm); }
</style>
