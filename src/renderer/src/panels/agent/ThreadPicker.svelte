<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import { relativeOpened } from './threads';

  let { PM }: { PM: Record<string, any> } = $props();
  let trigger = $state<HTMLButtonElement>();
  let popup = $state<HTMLDivElement>();
  let search: HTMLInputElement | undefined = $state();
  let open = $state(false);
  let dismissByPress = false;
  let query = $state('');
  let left = $state(0);
  let top = $state(0);
  let width = $state(240);
  let openedAt = $state(Date.now());

  const blocked = $derived(agentState.threadSwitchBlocked);
  const hint = $derived(blocked ? 'Finish or stop the current run to switch threads' : 'Switch thread');
  const threads = $derived(agentState.threads || []);
  const active = $derived(threads.find(thread => thread.id === agentState.threadId));
  const matches = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    return needle ? threads.filter(thread => thread.title.toLowerCase().includes(needle)) : threads;
  });

  // The popover API drives light dismiss and top-layer stacking; `open` mirrors
  // it so the list only exists while shown (and so tests without popover work).
  function close(focusTrigger = true): void {
    open = false;
    popup?.hidePopover?.();
    if (focusTrigger) trigger?.focus();
  }

  function toggle(event: MouseEvent): void {
    const dismiss = open || (event.detail > 0 && dismissByPress);
    dismissByPress = false;
    if (dismiss) { close(); return; }
    const rect = trigger!.getBoundingClientRect();
    // The rows carry two lines each, so the popup is measured from the list it will hold.
    const height = Math.min(360, 92 + Math.max(1, threads.length) * 46);
    width = Math.max(rect.width, 248);
    left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    top = rect.bottom + 6 + height > window.innerHeight - 8 && rect.top >= height + 8
      ? rect.top - height - 6
      : Math.min(rect.bottom + 6, Math.max(8, window.innerHeight - height - 8));
    query = '';
    openedAt = Date.now();
    open = true;
    popup?.showPopover?.();
  }

  $effect(() => { if (open) search?.focus(); });

  function rows(): HTMLElement[] {
    return Array.from(popup?.querySelectorAll<HTMLElement>('.thread-row') || []);
  }

  function step(from: HTMLElement | null, delta: number): void {
    const all = rows();
    if (!all.length) return;
    const index = from ? all.indexOf(from) : -1;
    const next = index < 0 ? (delta > 0 ? 0 : all.length - 1) : index + delta;
    if (next < 0) { search?.focus(); return; }
    all[Math.min(next, all.length - 1)]?.focus();
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
      <svg class="thread-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 6.25 3.5 3.5 3.5-3.5" /></svg>
    </button>
    <button class="thread-new" type="button" aria-label="New thread" title={blocked ? hint : 'New thread'} disabled={blocked}
      onclick={() => PM.AgentUI?.newThread()}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10" /></svg>
    </button>
  </div>
  {#if agentState.threadSaveError}<p class="thread-warning" role="status">Thread history couldn’t be saved. Keep this window open.</p>{/if}
{/if}

<div
  class="thread-popup"
  bind:this={popup}
  popover="auto"
  role="dialog"
  tabindex="-1"
  aria-label="Threads"
  style:left={`${left}px`}
  style:top={`${top}px`}
  style:width={`${width}px`}
  onbeforetoggle={(event) => { open = (event as ToggleEvent).newState === 'open'; }}
  onkeydown={onKeydown}
>
  {#if open}
    <input
      class="thread-search"
      type="text"
      bind:this={search}
      bind:value={query}
      placeholder="Search threads…"
      aria-label="Search threads"
      autocomplete="off"
      spellcheck="false"
    />
    <div class="thread-list" role="listbox" aria-label="Threads">
      {#each matches as thread (thread.id)}
        <div
          class="thread-row"
          class:current={thread.id === agentState.threadId}
          role="option"
          tabindex="-1"
          aria-selected={thread.id === agentState.threadId}
          onclick={() => choose(thread.id)}
          onkeydown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(thread.id); } }}
        >
          <span class="thread-row-text">
            <span class="thread-row-title">{thread.title}</span>
            <span class="thread-row-meta" class:current={thread.id === agentState.threadId}>
              {thread.id === agentState.threadId ? 'Current' : relativeOpened(thread.updatedAt ?? 0, openedAt)}
            </span>
          </span>
          <button
            class="thread-delete"
            type="button"
            aria-label={`Delete thread: ${thread.title}`}
            title="Delete thread"
            onclick={(event) => { event.stopPropagation(); remove(thread.id); }}
          >
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5 5 13h6l.5-8.5M6.75 6.75v3.75M9.25 6.75v3.75" /></svg>
          </button>
        </div>
      {:else}
        <p class="thread-empty">No threads match “{query.trim()}”.</p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .thread-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    margin: 0 10px;
    padding: 0 0 6px;
    flex-shrink: 0;
  }
  .thread-trigger { display: flex; align-items: center; gap: 2px; flex: 1; min-width: 0; height: 28px; padding: 0 2px; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-2); font: inherit; font-size: var(--fs-sm); text-align: left; cursor: pointer; }
  .thread-trigger span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .thread-chevron { width: 14px; height: 14px; flex-shrink: 0; }
  .thread-new { display: grid; place-items: center; width: 28px; height: 28px; flex-shrink: 0; padding: 6px; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-3); cursor: pointer; }
  .thread-trigger:hover:not(:disabled), .thread-new:hover:not(:disabled) { color: var(--tx-2); }
  .thread-new:hover:not(:disabled) { background: var(--ink-1); }
  .thread-new:focus-visible, .thread-trigger:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  :disabled { opacity: var(--disabled); cursor: default; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
  .thread-warning { margin: 0 10px 6px 18px; font-size: var(--fs-xs); color: var(--accent); }

  .thread-popup {
    position: fixed; inset: auto; margin: 0; box-sizing: border-box;
    max-height: min(360px, calc(100vh - 16px));
    /* A styled `display` would defeat the UA rule that hides a closed popover. */
    display: none; flex-direction: column;
    padding: 6px; border: 0; border-radius: var(--r-lg);
    background: color-mix(in srgb, var(--bg-float) 88%, transparent);
    backdrop-filter: blur(24px) saturate(140%);
    color: var(--tx); box-shadow: var(--shadow-float);
    font: var(--fs-md)/var(--lh) var(--f-ui);
    overflow: hidden;
  }
  .thread-popup:popover-open { display: flex; }
  .thread-popup::backdrop { background: transparent; }
  .thread-search { flex-shrink: 0; margin-bottom: 2px; height: var(--ctl-h); padding: 0 8px; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx); font: inherit; }
  .thread-search::placeholder { color: var(--tx-3); }
  .thread-search:focus { outline: none; }
  .thread-list { flex: 1; min-height: 0; overflow-y: auto; }
  .thread-row { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-radius: var(--r-md); cursor: pointer; }
  .thread-row:hover, .thread-row:focus-visible { background: var(--ink-1); outline: none; }
  .thread-row.current { background: var(--ink-2); }
  .thread-row-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .thread-row-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .thread-row-meta { color: var(--tx-3); font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .thread-row-meta.current { color: var(--blue); }
  .thread-delete { display: grid; place-items: center; width: 26px; height: 26px; flex-shrink: 0; padding: 0; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-3); opacity: 0; cursor: pointer; }
  .thread-row:hover .thread-delete, .thread-row.current .thread-delete, .thread-delete:focus-visible { opacity: 1; }
  .thread-delete:hover { color: var(--tx); background: var(--ink-2); }
  .thread-empty { margin: 4px 8px 8px; color: var(--tx-3); font-size: var(--fs-sm); }
</style>
