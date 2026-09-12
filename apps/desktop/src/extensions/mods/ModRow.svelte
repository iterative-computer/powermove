<script lang="ts">
  import type { ExtensionRecord } from 'powermove';

  let {
    record,
    quiet = false,
    onToggle,
    onFix,
    onMenu
  }: {
    record: ExtensionRecord;
    quiet?: boolean;
    onToggle: (record: ExtensionRecord, next: boolean) => void;
    onFix: (record: ExtensionRecord) => void;
    onMenu: (record: ExtensionRecord, anchor: HTMLElement) => void;
  } = $props();

  const ERROR_STATES = ['build-error', 'manifest-error', 'activation-error', 'runtime-error', 'needs-update'];
  const MAX_ERROR_CHARS = 120;

  let rowElement: HTMLDivElement | undefined = $state();
  let menuElement: HTMLButtonElement | undefined = $state();

  const name = $derived(record.manifest?.name ?? record.id);
  const description = $derived(record.manifest?.description ?? record.id);
  const health = $derived(record.health);
  const broken = $derived(ERROR_STATES.includes(health.state));
  const fullError = $derived('error' in health ? health.error : '');
  const shortError = $derived(broken ? summarise(fullError) : '');
  const notes = $derived(notesFor(record));

  /** First line, clipped — the row stays one glance; the title carries the rest. */
  function summarise(text: string): string {
    const line = (text.split('\n', 1)[0] ?? '').trim() || 'Something went wrong.';
    return line.length > MAX_ERROR_CHARS ? `${line.slice(0, MAX_ERROR_CHARS - 1)}…` : line;
  }

  function notesFor(current: ExtensionRecord): string[] {
    const out: string[] = [];
    if (current.health.state === 'replaced') out.push(`Replaced by ${current.health.by}`);
    const replaces = current.manifest?.replaces ?? [];
    if (replaces.length > 0) out.push(`Replaces built-in ${replaces.join(', ')}`);
    const forked = current.manifest?.forkedFrom;
    if (forked) {
      const at = forked.lastIndexOf('@');
      const base = at > 0 ? forked.slice(0, at) : forked;
      const version = at > 0 ? forked.slice(at + 1) : '';
      out.push(version ? `Forked from ${base} v${version}` : `Forked from ${base}`);
    }
    return out;
  }

  function toggle(event: Event): void {
    event.stopPropagation();
    onToggle(record, !record.enabled);
  }

  function openMenu(): void {
    if (menuElement) onMenu(record, menuElement);
  }

  function handleMenuClick(event: MouseEvent): void {
    event.stopPropagation();
    openMenu();
  }

  function handleContextMenu(event: MouseEvent): void {
    event.preventDefault();
    openMenu();
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.target !== rowElement) return;
    if (event.key === ' ' || event.key === 'Enter') {
      event.preventDefault();
      onToggle(record, !record.enabled);
    } else if (event.key === 'F10' && event.shiftKey) {
      event.preventDefault();
      openMenu();
    }
  }
</script>

<!-- The row is a focus stop so Space toggles and Shift+F10 opens the menu without
     tabbing through every control. It stays a listitem, not a fake control. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="row"
  class:quiet
  class:off={!record.enabled}
  role="listitem"
  tabindex="0"
  aria-label={name}
  bind:this={rowElement}
  onkeydown={handleKeydown}
  oncontextmenu={handleContextMenu}
>
  <div class="text">
    <div class="name">{name}</div>
    <div class="desc" title={description}>{description}</div>
    {#each notes as note (note)}
      <div class="note">{note}</div>
    {/each}
    {#if shortError}
      <div class="error" title={fullError}>{shortError}</div>
      <div class="repair">
        <button type="button" class="link" onclick={() => onFix(record)}>Fix it</button>
        <button type="button" class="link" onclick={() => onToggle(record, false)}>Turn off</button>
      </div>
    {/if}
  </div>

  <button
    type="button"
    class="toggle"
    class:on={record.enabled}
    aria-pressed={record.enabled}
    aria-label={record.enabled ? `Turn off ${name}` : `Turn on ${name}`}
    onclick={toggle}><i aria-hidden="true"></i></button>

  <button type="button" class="more" aria-label={`More for ${name}`} bind:this={menuElement} onclick={handleMenuClick}
    >⋯</button
  >
</div>

<style>
  .row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    min-height: var(--row-h);
    padding: 6px;
    border-radius: var(--r-sm);
    cursor: default;
    transition: background var(--dur-1) var(--ease);
  }

  .row:hover,
  .row:focus-visible {
    background: var(--bg-row);
    outline: 0;
  }

  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .name {
    color: var(--tx);
    font-size: var(--fs-md);
    line-height: var(--lh-tight);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .quiet .name {
    color: var(--tx-2);
  }

  .off .name {
    color: var(--tx-3);
  }

  .desc {
    color: var(--tx-3);
    font-size: var(--fs-xs);
    line-height: var(--lh-tight);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .note {
    color: var(--tx-4);
    font-size: var(--fs-xs);
    line-height: var(--lh-tight);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .error {
    margin-top: 2px;
    color: var(--danger);
    font-size: var(--fs-xs);
    line-height: var(--lh-tight);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .repair {
    display: flex;
    gap: 10px;
    margin-top: 3px;
  }

  .link {
    padding: 0;
    background: none;
    color: var(--accent-tx);
    font-size: var(--fs-xs);
    line-height: var(--lh-tight);
    cursor: default;
  }

  .link:hover {
    color: var(--accent-hover);
  }

  .toggle {
    position: relative;
    flex: none;
    width: 32px;
    height: 18px;
    margin-top: 1px;
    border-radius: var(--r-pill);
    background: var(--ink-3);
    box-shadow: var(--ctl-edge-inset);
    transition: background var(--dur-2) var(--ease-io);
  }

  .toggle i {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--bg-float);
    box-shadow: var(--shadow-raise);
    transition: transform var(--dur-2) var(--ease-io);
  }

  .toggle.on {
    background: var(--accent);
  }

  .toggle.on i {
    transform: translateX(14px);
  }

  .more {
    flex: none;
    width: 18px;
    height: 18px;
    margin-top: 1px;
    border-radius: var(--r-xs);
    background: none;
    color: var(--tx-3);
    font-size: var(--fs-md);
    line-height: 1;
    opacity: 0;
    cursor: default;
    transition: opacity var(--dur-1) var(--ease);
  }

  .row:hover .more,
  .row:focus-within .more,
  .more:focus-visible {
    opacity: 1;
  }

  .more:hover {
    background: var(--ink-1);
    color: var(--tx);
  }

  @media (prefers-reduced-motion: reduce) {
    .row,
    .toggle,
    .toggle i,
    .more {
      transition: none;
    }
  }
</style>
