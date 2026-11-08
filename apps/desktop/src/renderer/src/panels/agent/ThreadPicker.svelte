<script lang="ts">
  import '../../controls/search-picker.css';
  import { onDestroy, tick } from 'svelte';
  import { agentState } from './agent-state.svelte';
  import { relativeOpened } from './threads';
  import { blink } from '../../controls/menu-blink';

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
  type Thread = NonNullable<typeof agentState.threads>[number];
  const threads = $derived(agentState.threads || []);
  const active = $derived(threads.find(thread => thread.id === agentState.threadId));
  const byId = $derived(new Map(threads.map(thread => [thread.id, thread])));
  const parentOf = (thread: Thread | undefined) => thread?.parentId ? byId.get(thread.parentId) : undefined;
  const activeParent = $derived(parentOf(active));
  const childrenOf = $derived.by(() => {
    const map = new Map<string, Thread[]>();
    for (const thread of threads) if (thread.parentId && byId.has(thread.parentId)) map.set(thread.parentId, [...(map.get(thread.parentId) ?? []), thread]);
    return map;
  });
  /* The family on screen: the current thread, its ancestors and their sub-threads. */
  const family = $derived.by(() => {
    const ids = new Set<string>();
    for (let thread = active; thread; thread = parentOf(thread)) ids.add(thread.id);
    return ids;
  });
  const descendants = (id: string): Thread[] => (childrenOf.get(id) ?? []).flatMap(child => [child, ...descendants(child.id)]);
  /* Sub-threads sit under the thread that started them. They unfold for the
     conversation you are in; elsewhere the parent row counts them. */
  const matches = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    if (needle) return threads.filter(thread => thread.title.toLowerCase().includes(needle)).map(thread => ({ thread, depth: 0 }));
    const out: Array<{ thread: Thread; depth: number }> = [];
    const visit = (thread: Thread, depth: number) => {
      out.push({ thread, depth });
      if (family.has(thread.id)) for (const child of childrenOf.get(thread.id) ?? []) visit(child, Math.min(depth + 1, 3));
    };
    for (const thread of threads) if (!thread.parentId || !byId.has(thread.parentId)) visit(thread, 0);
    return out;
  });
  function threadMeta(thread: Thread, depth: number): string {
    const parent = parentOf(thread);
    const role = thread.kind === 'subagent' ? 'Subagent' : thread.kind === 'thread' ? 'Started by the agent' : '';
    const where = role && !depth && parent ? `${role} in ${parent.title}` : role;
    const state = thread.busy ? 'Working…'
      : thread.kind === 'subagent' && thread.status === 'failed' ? 'Failed'
      : thread.kind === 'subagent' && thread.status === 'cancelled' ? 'Stopped'
      : thread.id === agentState.threadId ? 'Current'
      : thread.kind === 'subagent' && thread.status === 'completed' ? 'Done'
      : relativeOpened(thread.updatedAt ?? 0, openedAt);
    const hidden = depth || family.has(thread.id) ? [] : descendants(thread.id);
    const folded = hidden.length ? ` · ${hidden.length} sub-thread${hidden.length === 1 ? '' : 's'}${hidden.some(item => item.busy) ? ' working' : ''}` : '';
    return [where, state].filter(Boolean).join(' · ') + folded;
  }
  const page = $derived(matches.slice(0, visible));
  const reduce = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

  function finishClose(): void {
    // Reopening can reuse this node before the close animation completes.
    // Its old listener must not consume the next opening animation's end.
    popup?.removeEventListener('animationend', finishClose);
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
  onDestroy(() => {
    if (closeTimer) clearTimeout(closeTimer);
    popup?.removeEventListener('animationend', finishClose);
    detach();
  });

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
    if (picking) return;
    glider?.classList.remove('on');
    gliderOn = false;
  }
  function onRowPointer(event: PointerEvent): void {
    const row = (event.target as HTMLElement).closest<HTMLElement>('.thread-row');
    if (row && !picking) glideTo(row);
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

  // A row picked by click or key blinks before the menu closes (menu-blink.ts);
  // until then the list takes no more input.
  let picking = false;
  function choose(id: string, row?: HTMLElement): void {
    if (picking) return;
    if (row) {
      picking = true;
      glideTo(row);
      blink(row, () => { picking = false; choose(id); });
      return;
    }
    close(false);
    if (id !== agentState.threadId) PM.AgentUI?.switchThread(id);
  }

  async function remove(thread: Thread): Promise<void> {
    const subs = descendants(thread.id).length;
    const busy = thread.busy || descendants(thread.id).some(item => item.busy);
    const also = subs ? ` and its ${subs === 1 ? 'sub-thread' : `${subs} sub-threads`}` : '';
    const ok = await PM.confirm?.({
      message: `Delete “${thread.title}”?`,
      detail: busy
        ? `This stops its work and removes the thread’s history${also}. This cannot be undone.`
        : `This removes the thread’s history${also}. This cannot be undone.`,
      confirmLabel: 'Delete Thread',
      destructive: true
    });
    if (!ok) { if (open) search?.focus(); return; }
    const gone = new Set([thread.id, ...descendants(thread.id).map(item => item.id)]);
    const remaining = matches.filter(item => !gone.has(item.thread.id));
    PM.AgentUI?.deleteThread?.(thread.id);
    if (!open) return;
    if (remaining.length) search?.focus(); else close();
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (picking) { event.preventDefault(); return; }
    const target = event.target as HTMLElement;
    const row = target.closest<HTMLElement>('.thread-row');
    if (event.key === 'Escape') { event.preventDefault(); close(); return; }
    if (event.key === 'Tab') { event.preventDefault(); close(); return; }
    if (event.key === 'ArrowDown') { event.preventDefault(); step(row, 1); return; }
    if (event.key === 'ArrowUp') { event.preventDefault(); step(row, -1); return; }
    if (event.key === 'Enter' && target === search) { event.preventDefault(); if (matches[0]) choose(matches[0].thread.id); }
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
      {#if activeParent}<span class="thread-parent">{activeParent.title}</span><span class="thread-sep" aria-hidden="true">›</span>{/if}
      <span class="thread-title">{active?.title || 'New thread'}</span>
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
      {#each page as { thread, depth } (thread.id)}
        <div
          class="thread-row"
          class:current={thread.id === agentState.threadId}
          class:sub={depth > 0}
          style:--depth={depth}
          data-kind={thread.kind}
          role="option"
          tabindex="-1"
          aria-selected={thread.id === agentState.threadId}
          onclick={(event) => choose(thread.id, event.currentTarget as HTMLElement)}
          onfocus={(event) => glideTo(event.currentTarget as HTMLElement)}
          onkeydown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(thread.id, event.currentTarget as HTMLElement); } }}
        >
          <span class="thread-row-text">
            <span class="thread-row-title">
              {#if thread.busy}<span class="thread-dot" aria-hidden="true"></span>{/if}{thread.title}
            </span>
            <span class="thread-row-meta" class:current={thread.id === agentState.threadId}>
              {threadMeta(thread, depth)}
            </span>
          </span>
          <button
            class="thread-delete"
            type="button"
            aria-label={`Delete thread: ${thread.title}`}
            title={thread.busy ? 'Delete thread and stop its run' : 'Delete thread'}
            onclick={(event) => { event.stopPropagation(); void remove(thread); }}
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
  .thread-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .thread-chevron { width: 14px; height: 14px; flex-shrink: 0; }
  .thread-new { display: grid; place-items: center; width: 28px; height: 28px; flex-shrink: 0; padding: 6px; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-3); cursor: default; }
  .thread-trigger:hover:not(:disabled), .thread-new:hover:not(:disabled) { color: var(--tx-2); }
  .thread-new:hover:not(:disabled) { background: var(--ink-1); }
  :disabled { opacity: var(--disabled); cursor: default; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
  .thread-warning { margin: 0 10px 6px 18px; font-size: var(--fs-xs); color: var(--accent); }
  .thread-parent { flex: 0 1 auto; max-width: 42%; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--tx-3); }
  .thread-sep { flex-shrink: 0; padding: 0 4px; color: var(--tx-3); }

</style>
