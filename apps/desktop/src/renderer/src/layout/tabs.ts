import type { PMRegistry } from '../legacy/registry';
import { beginPanelDrag } from './drag';
import { openPanelMenu } from './menu';
import { activateTab, cycleTab, findPanel, findSlot, type Workspace } from './model';

/* Panel tab groups. A slot holding several panels shows their titles as tabs
   in the shown panel's header, where its title would be. The header keeps its
   actions, menu and drag; each tab is also a drag source of its own. */

type Rect = { left: number; right: number; top: number; bottom: number };

/** One visible slot, measured for a drop: its header, its body and its tabs
 * in order (an untabbed panel's title counts as its one tab). */
export type TabDropTarget = {
  id: string;
  header: Rect;
  body: Rect;
  tabs: Array<{ id: string; rect: Rect }>;
};

/** Where a dragged panel joins a group: before `beforeId`, or last. */
export type TabDrop = { targetId: string; beforeId: string | null; zone: 'header' | 'center'; x: number };

const inside = (rect: Rect, x: number, y: number): boolean =>
  x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;

/** Fraction of a panel body kept for docking above or below it at each edge;
 * the middle stacks onto the panel. Matches the area corners' edge band. */
const CENTER_BAND = 0.3;

/**
 * Resolves a drop onto a panel header (at a tab position) or the middle of a
 * panel body (as the last tab). `'same'` is a tab dropped back where it is.
 * A panel over its own untabbed header is not a tab drop.
 */
export function resolveTabDrop(targets: TabDropTarget[], sourceId: string, x: number, y: number): TabDrop | 'same' | null {
  for (const target of targets) {
    const ids = target.tabs.map((tab) => tab.id);
    if (ids.length === 1 && ids[0] === sourceId) continue;
    if (inside(target.header, x, y)) {
      const before = target.tabs.find((tab) => x < (tab.rect.left + tab.rect.right) / 2);
      const last = target.tabs.at(-1);
      const beforeId = before?.id ?? null;
      const from = ids.indexOf(sourceId);
      if (from >= 0 && (beforeId === sourceId || beforeId === (ids[from + 1] ?? null))) return 'same';
      return { targetId: target.id, beforeId, zone: 'header', x: before ? before.rect.left : (last?.rect.right ?? target.header.left) };
    }
    const { body } = target;
    const width = body.right - body.left;
    const height = body.bottom - body.top;
    const center = {
      left: body.left + width * CENTER_BAND,
      right: body.right - width * CENTER_BAND,
      top: body.top + height * CENTER_BAND,
      bottom: body.bottom - height * CENTER_BAND
    };
    if (inside(center, x, y)) {
      return ids.includes(sourceId) ? 'same' : { targetId: target.id, beforeId: null, zone: 'center', x: body.left };
    }
  }
  return null;
}

const plainRect = ({ left, right, top, bottom }: DOMRect): Rect => ({ left, right, top, bottom });

/** Measures every docked panel that can take tabs. */
export function liveTabTargets(PM: PMRegistry): TabDropTarget[] {
  return [...document.querySelectorAll<HTMLElement>('#body .dock .panel[data-panel]:not(.headless)')]
    .filter((element) => canStack(PM, element.dataset.panel!))
    .flatMap((element) => {
      const header = element.querySelector<HTMLElement>(':scope > header');
      if (!header) return [];
      const tabs = [...header.querySelectorAll<HTMLElement>(':scope > .ptabs > .ptab')];
      const title = header.querySelector<HTMLElement>(':scope > .ptitle');
      const headerRect = plainRect(header.getBoundingClientRect());
      const panelRect = plainRect(element.getBoundingClientRect());
      return [{
        id: element.dataset.panel!,
        header: headerRect,
        body: { ...panelRect, top: headerRect.bottom },
        tabs: tabs.length
          ? tabs.map((tab) => ({ id: tab.dataset.tab!, rect: plainRect(tab.getBoundingClientRect()) }))
          : [{ id: element.dataset.panel!, rect: plainRect((title ?? header).getBoundingClientRect()) }]
      }];
    });
}

/** Panels that may be a tab: not the viewer, and with a header to show tabs in. */
export function canStack(PM: PMRegistry, id: string): boolean {
  return id !== 'viewer' && !!PM.PANELS?.[id] && !PM.PANELS[id].headless;
}

const titleOf = (PM: PMRegistry, id: string): string =>
  (PM.Layout?.ws && findPanel(PM.Layout.ws as Workspace, id)?.spec.title) || PM.PANELS?.[id]?.title || id;

/** Brings a tab to front. Tab choice is view state, so a built-in workspace
 * changes in place instead of forking an edited copy. */
export function showTab(PM: PMRegistry, id: string): void {
  const ws = PM.Layout?.ws as Workspace | null;
  const found = ws && findSlot(ws, id);
  if (!found || found.slot.length < 2 || found.spec.active) return;
  PM.WS.mutate((workspace: Workspace) => activateTab(workspace, id), { inPlace: true });
}

function focusTab(PM: PMRegistry, id: string): void {
  const tab = (PM.panelInst[id]?.header as HTMLElement | undefined)
    ?.querySelector<HTMLElement>(`:scope > .ptabs > .ptab[data-tab="${CSS.escape(id)}"]`);
  tab?.focus({ preventScroll: true });
}

/** Shows the next or previous tab of the group holding `id` and keeps focus on the tab strip. */
export function stepTab(PM: PMRegistry, id: string, delta: number, focus = false): void {
  let shown: string | null = null;
  PM.WS.mutate((workspace: Workspace) => { shown = cycleTab(workspace, id, delta); }, { inPlace: true });
  if (shown && focus) focusTab(PM, shown);
}

function makeTab(PM: PMRegistry, id: string, active: boolean): HTMLButtonElement {
  const tab = document.createElement('button');
  tab.type = 'button';
  tab.className = 'ptab';
  tab.dataset.tab = id;
  tab.setAttribute('role', 'tab');
  tab.setAttribute('aria-selected', String(active));
  tab.tabIndex = active ? 0 : -1;
  tab.textContent = titleOf(PM, id);
  const location = () => PM.Layout?.ws && findPanel(PM.Layout.ws as Workspace, id);
  tab.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) return;
    const current = location();
    if (!current) return;
    event.stopPropagation();
    beginPanelDrag(PM, event, current.spec, current.dock, tab, undefined, () => showTab(PM, id));
  });
  // Pointer taps arrive through the drag above; this is Enter or Space.
  tab.addEventListener('click', (event) => {
    if (event.detail === 0) showTab(PM, id);
  });
  tab.addEventListener('contextmenu', (event) => {
    event.preventDefault();
    event.stopPropagation();
    const current = location();
    if (current) openPanelMenu(PM, current.spec, current.dock, tab, { x: event.clientX, y: event.clientY });
  });
  return tab;
}

/** Renders `tabs` into the header of `id`, the panel the slot shows, or
 * restores its plain title when the slot holds only that panel. */
export function syncPanelTabs(PM: PMRegistry, id: string, tabs: string[]): void {
  const header = PM.panelInst[id]?.header as HTMLElement | undefined;
  if (!header) return;
  const title = header.querySelector<HTMLElement>(':scope > .ptitle');
  let strip = header.querySelector<HTMLElement>(':scope > .ptabs');
  if (tabs.length < 2) {
    strip?.remove();
    if (title) title.hidden = false;
    return;
  }
  if (!strip) {
    strip = document.createElement('div');
    strip.className = 'ptabs';
    strip.setAttribute('role', 'tablist');
    strip.addEventListener('keydown', (event) => {
      const delta = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
      if (!delta) return;
      event.preventDefault();
      event.stopPropagation();
      stepTab(PM, id, delta, true);
    });
    if (title) title.after(strip);
    else header.insertBefore(strip, header.querySelector(':scope > .sp'));
  }
  if (title) title.hidden = true;
  strip.setAttribute('aria-label', `${titleOf(PM, id)} tabs`);
  strip.replaceChildren(...tabs.map((tab) => makeTab(PM, tab, tab === id)));
}
