import type { PMRegistry } from '../legacy/registry';
import { SIDE_DOCKS } from './side-scroll';

export type PanelSpec = {
  id: string;
  title?: string;
  size?: number;
  min?: number;
  flex?: boolean;
  collapsed?: boolean;
  /** Consecutive panels in a dock that share a group stack as tabs. */
  group?: string;
  /** The group's shown tab; the first member when none is marked. */
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

export function ensureDockFill(dock: DockSpec | undefined): DockSpec | undefined {
  if (!dock?.panels.length) return dock;
  for (const panel of dock.panels) keepPanelAtSetHeight(panel);
  if (!dock.panels.some((panel) => panel.flex)) {
    const fill = [...dock.panels].reverse().find((panel) => !SET_HEIGHT_PANEL_DEFAULTS[panel.id])
      ?? dock.panels.at(-1)!;
    fill.flex = true;
  }
  return dock;
}

/** A dock's visual slots: a lone panel, or a run of panels stacked as tabs. */
export type DockSlot = { specs: PanelSpec[]; active: PanelSpec };

export function dockSlots(specs: PanelSpec[]): DockSlot[] {
  const slots: DockSlot[] = [];
  for (const spec of specs) {
    const last = slots.at(-1);
    if (spec.group && last && last.specs[0]!.group === spec.group) last.specs.push(spec);
    else slots.push({ specs: [spec], active: spec });
  }
  for (const slot of slots) slot.active = slot.specs.find((spec) => spec.active) ?? slot.specs[0]!;
  return slots;
}

/** Show one tab of a stack. The stack keeps its place and size in the column,
 * so the newly shown panel takes over the previous tab's size. */
export function activateTab(ws: Workspace, id: string): boolean {
  const found = findPanel(ws, id);
  if (!found?.spec.group) return false;
  const slot = dockSlots(found.dock.panels).find((item) => item.specs.includes(found.spec));
  if (!slot || slot.specs.length < 2 || slot.active === found.spec) return false;
  const previous = slot.active;
  for (const spec of slot.specs) delete spec.active;
  found.spec.active = true;
  if (previous.flex) found.spec.flex = true;
  else delete found.spec.flex;
  if (previous.size) found.spec.size = previous.size;
  else delete found.spec.size;
  if (previous.collapsed) found.spec.collapsed = true;
  else delete found.spec.collapsed;
  return true;
}

/* After Effects' docking model, on Powermove's columns. A dragged panel (or a
   whole tab stack) lands one of three ways:
   - group  → it joins another panel's tab stack and becomes the shown tab
   - beside → it docks above or below another panel or stack
   - dock   → it goes into a column at a point, or at the end of it
   Side-by-side splits inside a column are left out on purpose: columns stack
   top to bottom, and splitting a 300px column in two leaves neither half
   usable. New columns still come from the window-edge drop strips. */
export type PanelPlacement =
  | { kind: 'group'; target: string; before?: string | null }
  | { kind: 'beside'; target: string; place: 'above' | 'below' }
  | { kind: 'dock'; dockId: string; before?: string | null };

const SLOT_SIZING = ['size', 'flex', 'collapsed'] as const;

function copySlotSizing(from: PanelSpec, to: PanelSpec): void {
  for (const key of SLOT_SIZING) {
    if (from[key]) (to as Record<string, unknown>)[key] = from[key];
    else delete to[key];
  }
}

function slotOf(dock: DockSpec, spec: PanelSpec): DockSlot | undefined {
  return dockSlots(dock.panels).find((slot) => slot.specs.includes(spec));
}

/** A lone panel left with a group name would merge with any neighbour that
 * later shares it, so a stack of one stops being a stack. */
function tidyGroups(dock: DockSpec): void {
  for (const slot of dockSlots(dock.panels)) {
    if (slot.specs.length > 1) continue;
    delete slot.specs[0]!.group;
    delete slot.specs[0]!.active;
  }
}

/** When a stack's shown tab leaves, the next one that stays is shown and
 * inherits the stack's place and size. */
function handOffStack(dock: DockSpec, leaving: PanelSpec, leavingIds: string[]): void {
  const slot = slotOf(dock, leaving);
  const rest = slot?.specs.filter((spec) => !leavingIds.includes(spec.id)) ?? [];
  if (!slot || slot.active !== leaving || !rest.length) return;
  const heir = rest[0]!;
  for (const spec of rest) delete spec.active;
  heir.active = true;
  copySlotSizing(leaving, heir);
}

/** Takes panels out of the layout. A stack that loses its shown tab shows the
 * next one, which inherits the stack's place and size. */
function detachPanels(ws: Workspace, ids: string[]): Set<DockSpec> {
  const touched = new Set<DockSpec>();
  for (const id of ids) {
    const found = findPanel(ws, id);
    if (!found) continue;
    handOffStack(found.dock, found.spec, ids);
    found.dock.panels.splice(found.dock.panels.indexOf(found.spec), 1);
    touched.add(found.dock);
  }
  return touched;
}

/** Moves panels to a new place, keeping a moved stack together. Returns false
 * when the placement has nowhere to go (its target was one of the moved). */
export function placePanels(ws: Workspace, specs: PanelSpec[], placement: PanelPlacement): boolean {
  const ids = specs.map((spec) => spec.id);
  if (!ids.length) return false;
  if (placement.kind !== 'dock' && ids.includes(placement.target)) return false;
  const live = specs.map((spec) => ({ ...(findPanel(ws, spec.id)?.spec ?? spec) }));
  const shown = live.find((spec) => spec.active) ?? live[0]!;
  /* An anchor that is itself moving stands for the next panel that is not. */
  const anchorAfterMove = (dock: DockSpec, before?: string | null): PanelSpec | undefined => {
    if (!before) return undefined;
    const start = dock.panels.findIndex((spec) => spec.id === before);
    return start < 0 ? undefined : dock.panels.slice(start).find((spec) => !ids.includes(spec.id));
  };
  let anchor: PanelSpec | undefined;
  if (placement.kind === 'dock') {
    const dock = ws.layout.docks.find((item) => item.id === placement.dockId);
    anchor = dock ? anchorAfterMove(dock, placement.before) : undefined;
  } else if (placement.kind === 'group') {
    const target = findPanel(ws, placement.target);
    anchor = target ? anchorAfterMove(target.dock, placement.before) : undefined;
  }

  const touched = detachPanels(ws, ids);
  ws.hiddenPanels = (ws.hiddenPanels ?? []).filter((item) => !ids.includes(item.id));

  let dock: DockSpec;
  let index: number;
  if (placement.kind === 'dock') {
    dock = ensureDock(ws, placement.dockId);
    const slot = anchor && dock.panels.includes(anchor) ? slotOf(dock, anchor) : undefined;
    index = slot ? dock.panels.indexOf(slot.specs[0]!) : dock.panels.length;
  } else {
    const target = findPanel(ws, placement.target);
    if (!target) return false;
    dock = target.dock;
    const slot = slotOf(dock, target.spec)!;
    if (placement.kind === 'beside') {
      index = dock.panels.indexOf(placement.place === 'above' ? slot.specs[0]! : slot.specs.at(-1)!) + (placement.place === 'below' ? 1 : 0);
    } else {
      const group = target.spec.group || uniqueGroup(ws, target.spec.id);
      for (const member of slot.specs) member.group = group;
      for (const member of live) {
        member.group = group;
        delete member.active;
        for (const key of SLOT_SIZING) delete member[key];
      }
      /* The dropped panel comes to the front and takes over the stack's size. */
      const front = live.find((spec) => spec.id === shown.id)!;
      copySlotSizing(slot.active, front);
      for (const member of slot.specs) delete member.active;
      front.active = true;
      const at = anchor && slot.specs.includes(anchor) ? anchor : null;
      index = at ? dock.panels.indexOf(at) : dock.panels.indexOf(slot.specs.at(-1)!) + 1;
    }
  }
  if (placement.kind !== 'group' && live.length === 1) {
    delete live[0]!.group;
    delete live[0]!.active;
  }
  for (const spec of live) keepPanelAtSetHeight(spec);
  dock.panels.splice(index, 0, ...live);
  touched.add(dock);
  for (const item of touched) {
    tidyGroups(item);
    ensureDockFill(item);
  }
  if (dock.hidden) dock.hidden = false;
  return true;
}

function uniqueGroup(ws: Workspace, seed: string): string {
  const used = new Set(ws.layout.docks.flatMap((dock) => dock.panels.map((spec) => spec.group)));
  let name = `stack-${seed}`;
  for (let n = 2; used.has(name); n += 1) name = `stack-${seed}-${n}`;
  return name;
}

/** The members of a panel's tab stack, or the panel alone. */
export function stackOf(ws: Workspace, id: string): PanelSpec[] {
  const found = findPanel(ws, id);
  if (!found) return [];
  return slotOf(found.dock, found.spec)?.specs ?? [found.spec];
}

export function removePanel(ws: Workspace, id: string): void {
  eachDock(ws, (dock) => {
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
  handOffStack(found.dock, found.spec, [id]);
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
  const index = Math.max(0, Math.min(hidden.index, dock.panels.length));
  dock.panels.splice(index, 0, { ...hidden.spec, id });
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
  const insertion = index == null ? dock.panels.length : Math.max(0, Math.min(index, dock.panels.length));
  dock.panels.splice(insertion, 0, clean);
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

export function movePanelBy(ws: Workspace, id: string, delta: number): boolean {
  const found = findPanel(ws, id);
  if (!found || !Number.isInteger(delta) || !delta) return false;
  const from = found.dock.panels.indexOf(found.spec);
  const to = Math.max(0, Math.min(from + delta, found.dock.panels.length - 1));
  if (to === from) return false;
  found.dock.panels.splice(from, 1);
  found.dock.panels.splice(to, 0, found.spec);
  return true;
}

/**
 * Floor a panel may shrink to in a dock column. Side columns scroll only when
 * their panels stop crushing, so this floor is what makes the overflow real —
 * and it must match `clampPanelHeight`'s, or a splitter drag would fight the
 * DOM minimum it is dragging against.
 *
 * In a side column a fluid panel stops at a usable height instead: the column
 * scrolls before Properties or Media crush into a sliver with its own tiny
 * scroller, which made the two scroll levels fight.
 */
export const SIDE_FLUID_MIN_HEIGHT = 200;
const isFluid = (spec: PanelSpec, def: Record<string, any>): boolean => !!(spec.flex || (!spec.size && !def.size));
export function panelMinHeight(
  spec: PanelSpec,
  def: Record<string, any> = {},
  dockId?: string,
  fluid = isFluid(spec, def)
): number {
  const floor = Math.max(72, Number(spec.min ?? def.min) || 88);
  return SIDE_DOCKS.has(dockId ?? '') && fluid ? Math.max(floor, SIDE_FLUID_MIN_HEIGHT) : floor;
}

/* Properties and Layer Effects read top to bottom. In a side column they
   show every row and the column scrolls as one page, as After Effects'
   Effect Controls does. Over a panel of a set height (Effects & Presets,
   Takes) they scroll inside themselves instead, so that panel stays in view,
   as in the prototype. */
const STACKED = new WeakSet<PanelSpec>();

/** Record which panels sit above a panel that keeps its own height. */
export function markStackedPanels(slots: DockSlot[], fitsOwnContent: (spec: PanelSpec) => boolean): DockSlot[] {
  slots.forEach((slot, index) => {
    const below = slots.slice(index + 1).some((other) => !fitsOwnContent(other.active));
    for (const spec of slot.specs) {
      if (below) STACKED.add(spec);
      else STACKED.delete(spec);
    }
  });
  return slots;
}

export const fitsContent = (def: Record<string, any>, dockId?: string, spec?: PanelSpec): boolean =>
  !!def.fitContent && SIDE_DOCKS.has(dockId ?? '') && !(spec && STACKED.has(spec));

/* A side column scrolls, so its fluid panel fills spare height but never
   shrinks below its set height (or the floor): when the column overflows it
   keeps the length it was dragged to and the column scrolls instead. */
export function panelFlex(spec: PanelSpec, def: Record<string, any>, dockId?: string): string {
  if (fitsContent(def, dockId, spec)) return isFluid(spec, def) ? '1 0 auto' : '0 0 auto';
  if (!isFluid(spec, def)) return `0 0 ${spec.size || def.size || 180}px`;
  if (!SIDE_DOCKS.has(dockId ?? '')) return '1 1 auto';
  return `1 0 ${Math.max(spec.size || 0, panelMinHeight(spec, def, dockId, true))}px`;
}

/** The expanded panel's CSS floor. A content-height panel treats a dragged
 * height as extra room below its rows, never as a cap above them. */
export function panelFloor(spec: PanelSpec, def: Record<string, any>, dockId?: string): number {
  // Its rows are its height, so the usable-height floor for fluid panels does not apply.
  if (fitsContent(def, dockId, spec)) return Math.max(panelMinHeight(spec, def, dockId, false), spec.size || 0);
  return panelMinHeight(spec, def, dockId);
}

export function applyPanelSize(el: HTMLElement, spec: PanelSpec, def: Record<string, any>, dockId?: string): void {
  el.style.flex = panelFlex(spec, def, dockId);
  // A panel turning fluid or fixed changes its floor in a side column.
  if (dockId && el.dataset.collapsed !== '1') el.style.minHeight = `${panelFloor(spec, def, dockId)}px`;
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
  inst.el.style.minHeight = collapsed ? 'var(--hdr-h)' : `${panelFloor(current.spec, inst.def, current.dock.id)}px`;
  inst.body.style.display = collapsed ? 'none' : '';
  inst.el.style.flex = collapsed
    ? '0 0 var(--hdr-h)'
    : panelFlex(current.spec, inst.def, current.dock.id);
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
