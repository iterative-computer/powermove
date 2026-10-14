import type { PMRegistry } from '../legacy/registry';

export type PanelSpec = {
  id: string;
  title?: string;
  size?: number;
  min?: number;
  flex?: boolean;
  collapsed?: boolean;
  /** Consecutive specs in a dock that share a group key form one slot whose
   * panels show as tabs in a single header. Members share the slot's size,
   * flex and collapsed state; every panel keeps its own spec, so code that
   * looks panels up by id sees tabbed panels like any other. */
  group?: string;
  /** The tab shown in its group. Without one, the first member shows. */
  active?: boolean;
};

export type DockSpec = {
  id: string;
  panels: PanelSpec[];
  size?: number;
  flex?: boolean;
  hidden?: boolean;
};

export type Workspace = {
  density?: string;
  theme?: Record<string, any>;
  hiddenPanels?: Array<Record<string, any>>;
  layout: { docks: DockSpec[] };
};

/* `bottom` is the one row dock: it spans under every column except `right`,
   so its `size` is a height rather than a width. */
const DOCK_ORDER: Record<string, number> = { left: 0, center: 1, bottom: 1.5, right: 2 };
export const defaultDockSize = (id: string): number => id === 'right' || id === 'bottom' ? 300 : 250;
const SET_HEIGHT_PANEL_DEFAULTS: Readonly<Record<string, number>> = Object.freeze({ agent: 350 });

/** Prefer a saved pixel height when another panel can fill the dock.
 * Keep that height even while the panel temporarily fills a column alone. */
export function keepPanelAtSetHeight(spec: PanelSpec): boolean {
  const fallback = SET_HEIGHT_PANEL_DEFAULTS[spec.id];
  if (!fallback) return false;
  spec.size = spec.size || fallback;
  delete spec.flex;
  return true;
}

export const eachDock = (ws: Workspace, fn: (dock: DockSpec) => void): void => {
  ws.layout.docks.forEach(fn);
};

export const hasPanel = (ws: Workspace, id: string): boolean =>
  ws.layout.docks.some((dock) => dock.panels.some((panel) => panel.id === id));

/** Splits a dock's panels into slots: runs of consecutive specs sharing a
 * group key, and single specs. Order and identity are preserved. */
export function dockSlots(panels: PanelSpec[]): PanelSpec[][] {
  const slots: PanelSpec[][] = [];
  for (const spec of panels) {
    const last = slots.at(-1);
    if (last && spec.group && last[0]!.group === spec.group) last.push(spec);
    else slots.push([spec]);
  }
  return slots;
}

/** The tab a slot shows. */
export const activeTab = (slot: PanelSpec[]): PanelSpec => slot.find((spec) => spec.active) ?? slot[0]!;

/** The slot holding `id`, with the dock it is in. */
export function findSlot(ws: Workspace, id: string): { dock: DockSpec; slot: PanelSpec[]; spec: PanelSpec } | null {
  for (const dock of ws.layout.docks) {
    for (const slot of dockSlots(dock.panels)) {
      const spec = slot.find((panel) => panel.id === id);
      if (spec) return { dock, slot, spec };
    }
  }
  return null;
}

const SLOT_FIELDS = ['size', 'flex', 'collapsed'] as const;

function copySlotFields(from: PanelSpec, to: PanelSpec): void {
  if (from === to) return;
  for (const key of SLOT_FIELDS) {
    if (from[key] == null) delete to[key];
    else (to as Record<string, unknown>)[key] = from[key];
  }
}

/** Marks `spec` as its slot's tab and gives every member the shown tab's
 * size, so the slot keeps its height whichever tab is in front. */
function showInSlot(slot: PanelSpec[], spec: PanelSpec): void {
  if (slot.length < 2) return;
  const shown = activeTab(slot);
  copySlotFields(shown, spec);
  for (const member of slot) {
    if (member === spec) member.active = true;
    else delete member.active;
    copySlotFields(spec, member);
  }
}

/** Gives the dock one slot that absorbs spare height. `syncTabs` copies each
 * slot's shown size onto its other tabs; the render path turns it off when it
 * passes a filtered view, so a popped-out tab keeps its place in front. */
export function ensureDockFill(dock: DockSpec | undefined, syncTabs = true): DockSpec | undefined {
  if (!dock?.panels.length) return dock;
  const slots = dockSlots(dock.panels);
  for (const slot of slots) {
    keepPanelAtSetHeight(activeTab(slot));
    if (syncTabs) showInSlot(slot, activeTab(slot));
  }
  if (!slots.some((slot) => activeTab(slot).flex)) {
    const fill = [...slots].reverse().find((slot) => !SET_HEIGHT_PANEL_DEFAULTS[activeTab(slot).id])
      ?? slots.at(-1)!;
    for (const spec of fill) spec.flex = true;
  }
  return dock;
}

/** Before `spec` leaves its slot, hand the front to its neighbouring tab. */
function handOffTab(dock: DockSpec, spec: PanelSpec): void {
  const slot = dockSlots(dock.panels).find((members) => members.includes(spec));
  if (!slot || slot.length < 2 || activeTab(slot) !== spec) return;
  const index = slot.indexOf(spec);
  showInSlot(slot, slot[index + 1] ?? slot[index - 1]!);
}

/** Clamps an insertion index and moves it past any group it would split.
 * Indexes count every tab, so callers that count slots can land inside one;
 * a spec of that same group may go between its members. */
function slotBoundary(panels: PanelSpec[], index: number, group?: string): number {
  let at = Math.max(0, Math.min(Number.isFinite(index) ? index : panels.length, panels.length));
  const splits = (): boolean => {
    const key = panels[at]?.group;
    return !!key && key !== group && key === panels[at - 1]?.group;
  };
  while (at > 0 && at < panels.length && splits()) at += 1;
  return at;
}

export function removePanel(ws: Workspace, id: string): void {
  eachDock(ws, (dock) => {
    const spec = dock.panels.find((panel) => panel.id === id);
    if (spec) handOffTab(dock, spec);
    dock.panels = dock.panels.filter((panel) => panel.id !== id);
    ensureDockFill(dock);
  });
}

export function ensureDock(ws: Workspace, id: string): DockSpec {
  let dock = ws.layout.docks.find((item) => item.id === id);
  if (dock) return dock;
  dock = { id, panels: [] };
  if (id === 'center') dock.flex = true;
  else dock.size = defaultDockSize(id);
  const order = DOCK_ORDER[id] ?? 3;
  const index = ws.layout.docks.findIndex((item) => (DOCK_ORDER[item.id] ?? 3) > order);
  ws.layout.docks.splice(index < 0 ? ws.layout.docks.length : index, 0, dock);
  return dock;
}

export function hidePanel(ws: Workspace, id: string): boolean {
  if (id === 'viewer') return false;
  const found = findPanel(ws, id);
  if (!found) return false;
  const index = found.dock.panels.indexOf(found.spec);
  ws.hiddenPanels = (ws.hiddenPanels ?? []).filter((item) => item.id !== id);
  ws.hiddenPanels.push({
    id,
    dockId: found.dock.id,
    index,
    dockIndex: ws.layout.docks.indexOf(found.dock),
    spec: { ...found.spec },
    dock: { id: found.dock.id, size: found.dock.size, flex: found.dock.flex }
  });
  handOffTab(found.dock, found.spec);
  found.dock.panels.splice(index, 1);
  ensureDockFill(found.dock);
  return true;
}

export function restorePanel(ws: Workspace, id: string): boolean {
  const hidden = (ws.hiddenPanels ?? []).find((item) => item.id === id);
  if (!hidden) return false;
  if (hasPanel(ws, id)) {
    ws.hiddenPanels = (ws.hiddenPanels ?? []).filter((item) => item.id !== id);
    return true;
  }
  let dock = ws.layout.docks.find((item) => item.id === hidden.dockId);
  if (!dock) {
    dock = { id: hidden.dockId, panels: [] };
    if (hidden.dock?.size) dock.size = hidden.dock.size;
    if (hidden.dock?.flex) dock.flex = true;
    const dockIndex = Math.max(0, Math.min(hidden.dockIndex ?? ws.layout.docks.length, ws.layout.docks.length));
    ws.layout.docks.splice(dockIndex, 0, dock);
  }
  const spec: PanelSpec = { ...hidden.spec, id };
  delete spec.active;
  dock.panels.splice(slotBoundary(dock.panels, hidden.index, spec.group), 0, spec);
  /* A closed tab that lands back beside its group rejoins it, in front. */
  const slot = dockSlots(dock.panels).find((members) => members.includes(spec))!;
  if (slot.length > 1) showInSlot(slot, spec);
  else delete spec.group;
  ensureDockFill(dock);
  ws.hiddenPanels = (ws.hiddenPanels ?? []).filter((item) => item.id !== id);
  return true;
}

export function findPanel(ws: Workspace, id: string): { dock: DockSpec; spec: PanelSpec } | null {
  for (const dock of ws.layout.docks) {
    const spec = dock.panels.find((panel) => panel.id === id);
    if (spec) return { dock, spec };
  }
  return null;
}

export function addPanel(ws: Workspace, id: string, dockId?: string): void {
  removePanel(ws, id);
  ws.hiddenPanels = (ws.hiddenPanels ?? []).filter((item) => item.id !== id);
  const dock = ensureDock(ws, dockId || 'right');
  const spec: PanelSpec = { id, flex: dock.panels.length === 0 };
  keepPanelAtSetHeight(spec);
  dock.panels.push(spec);
  ensureDockFill(dock);
  if (dock.hidden) dock.hidden = false;
}

export function insertPanel(ws: Workspace, spec: PanelSpec, dockId?: string, index?: number): void {
  const dock = ensureDock(ws, dockId || 'center');
  const clean: PanelSpec = { id: spec.id };
  if (spec.size) clean.size = spec.size;
  if (spec.flex) clean.flex = true;
  keepPanelAtSetHeight(clean);
  dock.panels.splice(slotBoundary(dock.panels, index ?? dock.panels.length), 0, clean);
  ensureDockFill(dock);
  if (dock.hidden) dock.hidden = false;
}

export function movePanel(ws: Workspace, id: string, dockId: string): boolean {
  const found = findPanel(ws, id);
  if (!found || found.dock.id === dockId) return false;
  const spec = { ...found.spec };
  removePanel(ws, id);
  insertPanel(ws, spec, dockId);
  ws.hiddenPanels = (ws.hiddenPanels ?? []).filter((item) => item.id !== id);
  return true;
}

/** Moves the slot holding `id` (with all its tabs) by `delta` slots. */
export function movePanelBy(ws: Workspace, id: string, delta: number): boolean {
  const found = findPanel(ws, id);
  if (!found || !Number.isInteger(delta) || !delta) return false;
  const slots = dockSlots(found.dock.panels);
  const from = slots.findIndex((slot) => slot.includes(found.spec));
  const to = Math.max(0, Math.min(from + delta, slots.length - 1));
  if (to === from) return false;
  slots.splice(to, 0, ...slots.splice(from, 1));
  found.dock.panels.splice(0, found.dock.panels.length, ...slots.flat());
  return true;
}

/** Group key unused anywhere in the workspace, closed panels included, so a
 * new group never merges with a stale run that kept its key. */
function newGroupKey(ws: Workspace): string {
  const used = new Set<string>();
  for (const dock of ws.layout.docks) for (const spec of dock.panels) if (spec.group) used.add(spec.group);
  for (const hidden of ws.hiddenPanels ?? []) if (hidden.spec?.group) used.add(hidden.spec.group);
  let n = 1;
  while (used.has(`tabs-${n}`)) n += 1;
  return `tabs-${n}`;
}

/** Panels that cannot be a tab: the viewer must stay visible. Headless
 * panels have no header to show tabs in. */
export type CanStack = (id: string) => boolean;
const stackable = (id: string, canStack?: CanStack): boolean => id !== 'viewer' && (canStack?.(id) ?? true);

/**
 * Stacks `id` as a tab in the slot holding `targetId`, before the tab
 * `beforeId` (or last), and brings it to front. Works across docks and for
 * reordering within a group. The slot keeps its size.
 */
export function stackPanel(ws: Workspace, id: string, targetId: string, beforeId: string | null = null, canStack?: CanStack): boolean {
  if (id === targetId && beforeId == null) return false;
  if (!stackable(id, canStack) || !stackable(targetId, canStack)) return false;
  const target = findSlot(ws, targetId);
  if (!target || beforeId === id) return false;
  const order = target.slot.map((spec) => spec.id);
  if (order.includes(id)) {
    const next = order.filter((item) => item !== id);
    const at = beforeId == null ? next.length : next.indexOf(beforeId);
    next.splice(at < 0 ? next.length : at, 0, id);
    if (next.join('\n') === order.join('\n')) return false;
  }
  /* Reordering within a group can name the dragged tab as the target. */
  const keep = order.find((item) => item !== id);
  if (!keep) return false;
  const shown = { ...activeTab(target.slot) };
  const group = target.slot[0]!.group ?? newGroupKey(ws);
  const source = findPanel(ws, id)?.spec;
  const spec: PanelSpec = { id, group };
  if (source?.title) spec.title = source.title;
  if (source?.min) spec.min = source.min;
  removePanel(ws, id);
  ws.hiddenPanels = (ws.hiddenPanels ?? []).filter((item) => item.id !== id);
  /* Removing the source can hand the target slot's fill role elsewhere;
     re-read it and restore the size it had before the move. */
  const slot = findSlot(ws, keep)!;
  for (const member of slot.slot) member.group = group;
  const anchor = beforeId == null ? undefined : slot.slot.find((member) => member.id === beforeId);
  const panels = slot.dock.panels;
  const at = anchor ? panels.indexOf(anchor) : panels.indexOf(slot.slot.at(-1)!) + 1;
  panels.splice(at, 0, spec);
  const members = dockSlots(panels).find((run) => run.includes(spec))!;
  copySlotFields(shown, activeTab(members));
  showInSlot(members, spec);
  ensureDockFill(slot.dock);
  if (slot.dock.hidden) slot.dock.hidden = false;
  return true;
}

/** Brings a tab to front in its group. */
export function activateTab(ws: Workspace, id: string): boolean {
  const found = findSlot(ws, id);
  if (!found || found.slot.length < 2 || activeTab(found.slot) === found.spec) return false;
  showInSlot(found.slot, found.spec);
  return true;
}

/** Shows the next (`delta` 1) or previous (-1) tab of the group holding `id`, wrapping. */
export function cycleTab(ws: Workspace, id: string, delta: number): string | null {
  const found = findSlot(ws, id);
  if (!found || found.slot.length < 2) return null;
  const from = found.slot.indexOf(activeTab(found.slot));
  const next = found.slot[(from + delta + found.slot.length * 8) % found.slot.length]!;
  showInSlot(found.slot, next);
  return next.id;
}

/** Moves a tab by `delta` places within its group. */
export function moveTabBy(ws: Workspace, id: string, delta: number): boolean {
  const found = findSlot(ws, id);
  if (!found || found.slot.length < 2 || !Number.isInteger(delta) || !delta) return false;
  const from = found.slot.indexOf(found.spec);
  const to = Math.max(0, Math.min(from + delta, found.slot.length - 1));
  if (to === from) return false;
  const panels = found.dock.panels;
  const start = panels.indexOf(found.slot[0]!);
  const order = [...found.slot];
  order.splice(to, 0, ...order.splice(from, 1));
  panels.splice(start, order.length, ...order);
  return true;
}

/** Takes a tab out of its group into its own slot right below the group. */
export function unstackPanel(ws: Workspace, id: string): boolean {
  const found = findSlot(ws, id);
  if (!found || found.slot.length < 2) return false;
  const title = found.spec.title;
  const neighbour = found.slot.find((member) => member !== found.spec)!.id;
  removePanel(ws, id);
  const rest = findSlot(ws, neighbour)!;
  insertPanel(ws, { id }, rest.dock.id, rest.dock.panels.indexOf(rest.slot.at(-1)!) + 1);
  const placed = findPanel(ws, id);
  if (placed && title) placed.spec.title = title;
  return true;
}

/**
 * Repairs saved or agent-authored tab groups: a key reused by a later run
 * starts a new group, the viewer and unstackable panels leave their group,
 * and each group shows exactly one tab. Single panels keep a group key so a
 * closed tab can rejoin its group when restored.
 */
export function tidyTabGroups(panels: PanelSpec[], canStack?: CanStack): void {
  for (const spec of panels) if (spec.group && !stackable(spec.id, canStack)) delete spec.group;
  const seen = new Set<string>();
  for (const slot of dockSlots(panels)) {
    const key = slot[0]!.group;
    if (key && seen.has(key)) {
      if (slot.length < 2) delete slot[0]!.group;
      else {
        let n = 2;
        while (seen.has(`${key}-${n}`)) n += 1;
        for (const spec of slot) spec.group = `${key}-${n}`;
      }
    }
    if (slot[0]!.group) seen.add(slot[0]!.group);
    if (slot.length < 2) delete slot[0]!.active;
    else showInSlot(slot, activeTab(slot));
  }
}

/**
 * Floor a panel may shrink to in a dock column. Side columns scroll only when
 * their panels stop crushing, so this floor is what makes the overflow real —
 * and it must match `clampPanelHeight`'s, or a splitter drag would fight the
 * DOM minimum it is dragging against.
 */
export function panelMinHeight(spec: PanelSpec, def: Record<string, any> = {}): number {
  return Math.max(72, Number(spec.min ?? def.min) || 88);
}

export function applyPanelSize(el: HTMLElement, spec: PanelSpec, def: Record<string, any>): void {
  el.style.flex = spec.flex || (!spec.size && !def.size)
    ? '1 1 auto'
    : `0 0 ${spec.size || def.size}px`;
}

export function setPanelCollapsed(PM: PMRegistry, id: string, collapsed: boolean, emit = true): boolean {
  if (id === 'viewer') return false;
  const inst = PM.panelInst[id];
  const current = PM.Layout.ws && findPanel(PM.Layout.ws as Workspace, id);
  if (!inst?.el || !inst.body || !current?.spec) return false;
  // Headless panels keep their controls inside the body. Collapsing it hides
  // every way to reopen the panel; also repair saved states from older shells.
  if ((PM.PANELS[id] ?? inst.def)?.headless) collapsed = false;
  current.spec.collapsed = !!collapsed;
  inst.el.dataset.collapsed = collapsed ? '1' : '0';
  inst.el.style.minHeight = collapsed ? 'var(--hdr-h)' : `${panelMinHeight(current.spec, inst.def)}px`;
  inst.body.style.display = collapsed ? 'none' : '';
  inst.el.style.flex = collapsed
    ? '0 0 var(--hdr-h)'
    : (current.spec.flex ? '1 1 auto' : `0 0 ${current.spec.size || inst.def.size || 180}px`);
  if (emit) {
    PM.WS.save();
    PM.bus.emit('layout:applied');
  }
  return true;
}

export function dockLabel(id: string): string {
  const labels: Record<string, string> = { left: 'Left dock', center: 'Center dock', right: 'Right dock', bottom: 'Bottom dock' };
  return labels[id] || id.replace(/^\w/, (character) => character.toUpperCase()) + ' dock';
}
