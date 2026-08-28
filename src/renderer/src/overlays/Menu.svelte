<script lang="ts">
  import { onMount } from 'svelte';

  import type { MenuItem } from './types';

  let {
    items,
    x,
    y,
    label = 'Menu',
    onrun,
    onclose
  }: {
    items: MenuItem[];
    x: number;
    y: number;
    label?: string;
    onrun: (item: Exclude<MenuItem, '-' | { header: string }>) => void;
    onclose: (restoreFocus?: boolean) => void;
  } = $props();

  let menu: HTMLElement;
  let buttons = $state<HTMLButtonElement[]>([]);
  let active = $state(0);
  let left = $state(0);
  let top = $state(0);
  const hasCurves = $derived(items.some(item => typeof item === 'object' && 'curve' in item && !!item.curve));

  export function element(): HTMLElement {
    return menu;
  }

  function focusAt(index: number): void {
    if (!buttons.length) return;
    active = (index + buttons.length) % buttons.length;
    for (const [buttonIndex, button] of buttons.entries()) {
      button.tabIndex = buttonIndex === active ? 0 : -1;
    }
    buttons[active]?.focus({ preventScroll: true });
  }

  function keydown(event: KeyboardEvent): void {
    if (!menu?.isConnected) return;
    const onItem = buttons.includes(document.activeElement as HTMLButtonElement);
    const onCurve = onItem && buttons[active]?.classList.contains('curve-option');
    if (event.key === 'ArrowDown') focusAt(onItem ? (onCurve ? Math.min(buttons.length - 1, active + 4) : active + 1) : 0);
    else if (event.key === 'ArrowUp') focusAt(onItem ? (onCurve ? Math.max(0, active - 4) : active - 1) : buttons.length - 1);
    else if (hasCurves && event.key === 'ArrowRight') focusAt(onItem ? active + 1 : 0);
    else if (hasCurves && event.key === 'ArrowLeft') focusAt(onItem ? active - 1 : 0);
    else if (event.key === 'Home') focusAt(0);
    else if (event.key === 'End') focusAt(buttons.length - 1);
    else if (event.key === 'Escape') onclose(true);
    else if (event.key === 'Tab') onclose(false);
    else if (event.key === 'Enter' || event.key === ' ') {
      buttons.find((button) => button === document.activeElement)?.click();
    }
    else return;
    if (event.key !== 'Tab') event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();
  }

  onMount(() => {
    const width = menu.offsetWidth;
    const height = menu.offsetHeight;
    left = Math.min(Math.max(x, 6), window.innerWidth - width - 6);
    top = Math.min(Math.max(y, 6), window.innerHeight - height - 6);
    buttons = [...menu.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]')];
    for (const button of buttons) button.tabIndex = -1;
    menu.focus({ preventScroll: true });
    window.addEventListener('keydown', keydown, true);
    return () => window.removeEventListener('keydown', keydown, true);
  });
</script>

<div
  bind:this={menu}
  class="drop"
  class:curve-grid={hasCurves}
  role="menu"
  tabindex="-1"
  aria-label={label}
  style:left={`${left}px`}
  style:top={`${top}px`}
  data-svelte-overlay-menu="true"
>
  {#each items as item}
    {#if item === '-'}
      <div class="sep" role="separator"></div>
    {:else if 'header' in item}
      <div class="hd">{item.header}</div>
    {:else}
      <button
        type="button"
        class:di={true}
        class:on={!!item.on}
        class:disabled={!!item.disabled}
        class:curve-option={!!item.curve}
        role="menuitem"
        aria-disabled={!!item.disabled}
        tabindex="-1"
        onclick={(event) => {
          event.stopPropagation();
          if (!item.disabled) onrun(item);
        }}
      >
        {#if item.curve}
          {@const p = item.curve}
          <svg class="curve-preview" viewBox="0 -12 52 56" aria-hidden="true">
            <path class="curve-guide" d="M6 6V34H46" />
            <path class="curve-handles" d={`M6 34L${6 + p[0]! * 40} ${34 - p[1]! * 28} M46 6L${6 + p[2]! * 40} ${34 - p[3]! * 28}`} />
            <path class="curve-line" d={`M6 34C${6 + p[0]! * 40} ${34 - p[1]! * 28} ${6 + p[2]! * 40} ${34 - p[3]! * 28} 46 6`} />
            <circle class="curve-control" cx={6 + p[0]! * 40} cy={34 - p[1]! * 28} r="1.8" />
            <circle class="curve-control" cx={6 + p[2]! * 40} cy={34 - p[3]! * 28} r="1.8" />
            <path class="curve-key" d="M6 31L9 34L6 37L3 34Z M46 3L49 6L46 9L43 6Z" />
          </svg>
        {/if}
        <span>{item.label}</span>
        {#if item.kb}
          <span class="kb">{item.kb}</span>
        {/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .curve-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); width: 344px; gap: 3px; }
  .curve-grid > :global(:not(.curve-option)) { grid-column: 1 / -1; }
  .curve-grid .curve-option { display: flex; flex-direction: column; justify-content: center; min-height: 76px; gap: 2px; padding: 4px 2px; font-size: 10px; text-align: center; }
  .curve-option.on { background: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line); }
  .curve-preview { flex: none; width: 52px; height: 40px; overflow: visible; }
  .curve-guide { fill: none; stroke: var(--tx-4); opacity: .25; stroke-width: 1; }
  .curve-handles { fill: none; stroke: var(--tx-3); opacity: .6; stroke-width: 1; }
  .curve-line { fill: none; stroke: var(--accent); stroke-width: 1.8; }
  .curve-control { fill: var(--bg-float); stroke: var(--tx-3); stroke-width: 1; }
  .curve-key { fill: var(--accent); }
</style>
