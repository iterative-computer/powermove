<script lang="ts">
  /* Powermove's menu, drawn to the macOS menu's measure: 24pt rows on a
     translucent panel, section titles, a checkmark column and an icon column
     only when a row needs them, the system accent under the highlighted row,
     submenus that cascade to the side, type-to-select, and press-drag-release
     picking for menus that open on a press. */
  import { flushSync, onMount, tick } from 'svelte';
  import Icon from '../panels/Icon.svelte';

  import { positionMenuAtCursor } from './position';
  import type { MenuAction, MenuItem } from './types';

  let {
    PM,
    items,
    x,
    y,
    cursorOrigin = false,
    label = 'Menu',
    onrun,
    onclose
  }: {
    PM: Record<string, any>;
    items: MenuItem[];
    x: number;
    y: number;
    cursorOrigin?: boolean;
    label?: string;
    onrun: (item: MenuAction) => void;
    onclose: (restoreFocus?: boolean) => void;
  } = $props();

  type Panel = {
    items: MenuItem[];
    left: number;
    top: number;
    placed: boolean;
    /** Index into `items` of the highlighted row, or -1. */
    active: number;
    /** Row in the parent panel this submenu hangs from. */
    parentRow: number;
    side?: 'above' | 'below';
  };

  const isAction = (item: MenuItem): item is MenuAction => item !== '-' && !('header' in item);
  /** Submenus open, and switch, after the pointer rests this long on a row. */
  const SUBMENU_DELAY = 120;
  /** Type-to-select forgets the typed prefix after this pause. */
  const TYPE_SELECT_MS = 750;

  // svelte-ignore state_referenced_locally
  let panels = $state<Panel[]>([{ items, left: x, top: y, placed: false, active: -1, parentRow: -1 }]);
  let host: HTMLElement;
  const panelEls: HTMLElement[] = [];
  let submenuTimer = 0;
  let typed = '';
  let typedTimer = 0;
  /* A press that began inside the menu ends in `click`; a press that opened
     the menu (right-button down) picks the row it is released on. */
  let pressedInside = false;
  let picked = false;

  export function element(): HTMLElement {
    return panelEls[0]!;
  }

  export function contains(node: Node | null): boolean {
    return !!node && !!host?.contains(node);
  }

  const hasCurves = (panel: Panel): boolean => panel.items.some((item) => isAction(item) && !!item.curve);
  const hasChecks = (panel: Panel): boolean => panel.items.some((item) => isAction(item) && item.on === true && !item.curve);
  const hasIcons = (panel: Panel): boolean => panel.items.some((item) => isAction(item) && !!item.icon && !item.curve);
  const selectable = (item: MenuItem | undefined): item is MenuAction => !!item && isAction(item) && !item.disabled;

  function rowEl(level: number, index: number): HTMLElement | null {
    return panelEls[level]?.querySelector<HTMLElement>(`[data-index="${index}"]`) ?? null;
  }

  function setActive(level: number, index: number, focus = true): void {
    const panel = panels[level];
    if (!panel) return;
    panel.active = index;
    if (!focus) return;
    const row = rowEl(level, index);
    if (row) {
      row.focus({ preventScroll: true });
      row.scrollIntoView({ block: 'nearest' });
    } else panelEls[level]?.focus({ preventScroll: true });
  }

  function place(level: number): void {
    const panel = panels[level];
    const el = panelEls[level];
    if (!panel || !el) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    if (level === 0) {
      if (cursorOrigin) {
        const position = positionMenuAtCursor(x, y, width, height, vw, vh, 2, 6);
        panel.left = position.left;
        panel.top = position.top;
        panel.side = position.side;
      } else {
        panel.left = Math.min(Math.max(x, 6), vw - width - 6);
        panel.top = Math.min(Math.max(y, 6), vh - height - 6);
      }
    } else {
      const row = rowEl(level - 1, panel.parentRow)?.getBoundingClientRect();
      const parent = panelEls[level - 1]?.getBoundingClientRect();
      if (!row || !parent) return;
      /* The first row lines up with the row that opened it; the panel
         overlaps its parent's edge slightly, as AppKit's cascade does. */
      const right = parent.right - 3;
      const left = parent.left - width + 3;
      panel.left = right + width <= vw - 4 || left < 4 ? Math.min(right, vw - width - 4) : left;
      panel.top = Math.min(Math.max(row.top - 5, 6), vh - height - 6);
    }
    panel.placed = true;
  }

  function openSubmenu(level: number, index: number, focusFirst = false): void {
    window.clearTimeout(submenuTimer);
    const item = panels[level]?.items[index];
    if (!item || !isAction(item) || !item.submenu || item.disabled) return;
    const open = panels[level + 1];
    if (!open || open.parentRow !== index) {
      panels = [...panels.slice(0, level + 1), { items: item.submenu, left: 0, top: 0, placed: false, active: -1, parentRow: index }];
      void tick().then(() => {
        place(level + 1);
        if (focusFirst) moveBy(level + 1, 1, -1);
      });
    } else if (focusFirst) moveBy(level + 1, 1, -1);
  }

  function closeBelow(level: number): void {
    window.clearTimeout(submenuTimer);
    if (panels.length > level + 1) panels = panels.slice(0, level + 1);
  }

  /** Next selectable row from `from` in `step` direction; stays put at the ends, as AppKit does. */
  function moveBy(level: number, step: 1 | -1, from = panels[level]?.active ?? -1): void {
    const panel = panels[level];
    if (!panel) return;
    const start = from < 0 ? (step > 0 ? -1 : panel.items.length) : from;
    for (let index = start + step; index >= 0 && index < panel.items.length; index += step) {
      if (selectable(panel.items[index])) { setActive(level, index); return; }
    }
  }

  function pick(level: number, index: number): void {
    const item = panels[level]?.items[index];
    if (picked || !selectable(item)) return;
    if (item.submenu) { openSubmenu(level, index, true); return; }
    picked = true;
    onrun(item);
  }

  function hover(level: number, index: number): void {
    const item = panels[level]?.items[index];
    window.clearTimeout(submenuTimer);
    if (!selectable(item)) {
      setActive(level, -1, false);
      panelEls[level]?.focus({ preventScroll: true });
      submenuTimer = window.setTimeout(() => closeBelow(level), SUBMENU_DELAY);
      return;
    }
    setActive(level, index);
    if (panels[level + 1]?.parentRow === index) return;
    submenuTimer = window.setTimeout(() => (item.submenu ? openSubmenu(level, index) : closeBelow(level)), SUBMENU_DELAY);
  }

  function enterPanel(level: number): void {
    /* Reaching a submenu keeps its parent row lit and cancels any switch. */
    window.clearTimeout(submenuTimer);
    const panel = panels[level];
    if (level > 0 && panel) panels[level - 1]!.active = panel.parentRow;
  }

  function leavePanel(level: number): void {
    if (panels.length > level + 1) return;
    window.clearTimeout(submenuTimer);
    setActive(level, -1, false);
    panelEls[level]?.focus({ preventScroll: true });
  }

  function typeSelect(key: string): boolean {
    const level = panels.length - 1;
    const panel = panels[level]!;
    window.clearTimeout(typedTimer);
    typed += key.toLocaleLowerCase();
    typedTimer = window.setTimeout(() => { typed = ''; }, TYPE_SELECT_MS);
    const index = panel.items.findIndex((item) => selectable(item) && item.label.toLocaleLowerCase().startsWith(typed));
    if (index < 0) return false;
    setActive(level, index);
    return true;
  }

  function keydown(event: KeyboardEvent): void {
    if (!host?.isConnected) return;
    const level = panels.length - 1;
    const panel = panels[level]!;
    const current = panel.items[panel.active];
    const curves = hasCurves(panel);
    if (picked) { /* swallow input until the menu has gone */ }
    else if (curves && (event.key === 'ArrowDown' || event.key === 'ArrowUp') && selectable(current) && current.curve) {
      /* The easing grid is four tiles wide: up and down move by a row. */
      const target = panel.active + (event.key === 'ArrowDown' ? 4 : -4);
      const tile = panel.items[target];
      if (selectable(tile) && tile.curve) setActive(level, target);
      else {
        const tiles = panel.items.flatMap((item, index) => (isAction(item) && item.curve ? [index] : []));
        if (event.key === 'ArrowDown') moveBy(level, 1, Math.max(...tiles));
        else moveBy(level, -1, Math.min(...tiles));
      }
    }
    else if (event.key === 'ArrowDown') moveBy(level, 1);
    else if (event.key === 'ArrowUp') moveBy(level, -1);
    else if (curves && event.key === 'ArrowRight') moveBy(level, 1);
    else if (curves && event.key === 'ArrowLeft') moveBy(level, -1);
    else if (event.key === 'ArrowRight') {
      if (selectable(current) && current.submenu) openSubmenu(level, panel.active, true);
    }
    else if (event.key === 'ArrowLeft') {
      if (level > 0) {
        const parentRow = panel.parentRow;
        closeBelow(level - 1);
        setActive(level - 1, parentRow);
      }
    }
    else if (event.key === 'Home') moveBy(level, 1, -1);
    else if (event.key === 'End') moveBy(level, -1, panel.items.length);
    else if (event.key === 'Escape') onclose(true);
    else if (event.key === 'Tab') { onclose(false); return; }
    else if (event.key === 'Enter' || event.key === ' ') {
      if (event.key === ' ' && typed) typeSelect(' ');
      else if (panel.active >= 0) pick(level, panel.active);
    }
    else if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) typeSelect(event.key);
    else return;
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();
  }

  function pointerdown(event: PointerEvent): void {
    pressedInside = contains(event.target as Node);
  }

  function rowPointerUp(event: PointerEvent, level: number, index: number): void {
    if (pressedInside || event.button > 2) return;
    /* Released over a row after the press that opened the menu. */
    event.preventDefault();
    pick(level, index);
  }

  onMount(() => {
    place(0);
    flushSync();
    panelEls[0]?.focus({ preventScroll: true });
    window.addEventListener('keydown', keydown, true);
    window.addEventListener('pointerdown', pointerdown, true);
    return () => {
      window.clearTimeout(submenuTimer);
      window.clearTimeout(typedTimer);
      window.removeEventListener('keydown', keydown, true);
      window.removeEventListener('pointerdown', pointerdown, true);
    };
  });
</script>

<div class="menu-host" bind:this={host} data-svelte-overlay-menu="true">
  {#each panels as panel, level (level)}
    <div
      bind:this={panelEls[level]}
      class="drop menu-panel"
      class:curve-grid={hasCurves(panel)}
      class:has-checks={hasChecks(panel)}
      class:has-icons={hasIcons(panel)}
      role="menu"
      tabindex="-1"
      aria-label={level === 0 ? label : (panels[level - 1]?.items[panel.parentRow] as MenuAction | undefined)?.label}
      data-level={level}
      data-side={panel.side}
      style:left={`${panel.left}px`}
      style:top={`${panel.top}px`}
      style:visibility={panel.placed ? 'visible' : 'hidden'}
      onpointerenter={() => enterPanel(level)}
      onpointerleave={() => leavePanel(level)}
    >
      {#each panel.items as item, index}
        {#if item === '-'}
          <div class="sep" role="separator"></div>
        {:else if 'header' in item}
          <div class="hd" role="presentation">{item.header}</div>
        {:else}
          <button
            type="button"
            class="di"
            class:on={item.on === true}
            class:active={panel.active === index}
            class:disabled={!!item.disabled}
            class:curve-option={!!item.curve}
            class:open={panels[level + 1]?.parentRow === index}
            role={item.on != null ? 'menuitemcheckbox' : 'menuitem'}
            aria-checked={item.on != null ? item.on : undefined}
            aria-haspopup={item.submenu ? 'menu' : undefined}
            aria-expanded={item.submenu ? panels[level + 1]?.parentRow === index : undefined}
            aria-disabled={!!item.disabled}
            data-index={index}
            tabindex="-1"
            onpointerenter={() => hover(level, index)}
            onpointerup={(event) => rowPointerUp(event, level, index)}
            onclick={(event) => {
              event.stopPropagation();
              pick(level, index);
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
              <span class="label">{item.label}</span>
            {:else}
              {#if hasChecks(panel)}
                <span class="check" aria-hidden="true">
                  {#if item.on === true}
                    <svg viewBox="0 0 12 12"><path d="M2.2 6.4 4.9 9.1 9.9 2.9" /></svg>
                  {/if}
                </span>
              {/if}
              {#if hasIcons(panel)}
                <span class="menu-icon" aria-hidden="true">{#if item.icon}<Icon {PM} name={item.icon} />{/if}</span>
              {/if}
              <span class="label">{item.label}</span>
              {#if item.submenu}
                <svg class="chev" viewBox="0 0 8 12" aria-hidden="true"><path d="M2 1.5 6.5 6 2 10.5" /></svg>
              {:else if item.kb}
                <span class="kb">{item.kb}</span>
              {/if}
            {/if}
          </button>
        {/if}
      {/each}
    </div>
  {/each}
</div>

<style>
  .curve-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); width: 344px; gap: 3px; }
  .curve-grid > :global(:not(.curve-option)) { grid-column: 1 / -1; }
  .curve-grid .curve-option { display: flex; flex-direction: column; justify-content: center; height: auto; min-height: 76px; gap: 2px; padding: 4px 2px; font-size: 10px; text-align: center; }
  .curve-grid .curve-option .label { flex: none; }
  .curve-option.on { background: var(--menu-hover-quiet); }
  .curve-preview { flex: none; width: 52px; height: 40px; overflow: visible; }
  .curve-guide { fill: none; stroke: currentColor; opacity: .2; stroke-width: 1; }
  .curve-handles { fill: none; stroke: currentColor; opacity: .45; stroke-width: 1; }
  .curve-line { fill: none; stroke: var(--accent); stroke-width: 1.8; }
  .curve-control { fill: var(--bg-float); stroke: currentColor; opacity: .7; stroke-width: 1; }
  .curve-key { fill: var(--accent); }
  .curve-option.active .curve-line, .curve-option.active .curve-key { stroke: currentColor; fill: none; }
  .curve-option.active .curve-key { fill: currentColor; }
</style>
