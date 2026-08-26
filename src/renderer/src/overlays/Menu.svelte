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
    if (event.key === 'ArrowDown') focusAt(onItem ? active + 1 : 0);
    else if (event.key === 'ArrowUp') focusAt(onItem ? active - 1 : buttons.length - 1);
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
        role="menuitem"
        aria-disabled={!!item.disabled}
        tabindex="-1"
        onclick={(event) => {
          event.stopPropagation();
          if (!item.disabled) onrun(item);
        }}
      >
        <span>{item.label}</span>
        {#if item.kb}
          <span class="kb">{item.kb}</span>
        {/if}
      </button>
    {/if}
  {/each}
</div>
