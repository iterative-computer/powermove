import { findPanel, hidePanel, insertPanel, keepPanelAtSetHeight, removePanel, type PanelSpec, type Workspace } from './model';

/* Blender-style area corners. Dragging from a panel corner resolves to one of
   Blender's area operations, adapted to docks that stack panels vertically:
   - inside the panel itself        → split it, a new panel takes the corner's side
   - into its neighbour in the dock → join: the neighbour closes, this one grows
   - into any other panel           → dock above/below it, or replace it outright
   Closing always goes through hidePanel, so a joined or replaced panel can be
   restored from the Library exactly where it was. */

export type AreaRect = { left: number; right: number; top: number; bottom: number };
export type Area = { id: string; dockId: string; index: number; rect: AreaRect };
export type AreaCorner = 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';
export type DockPlace = 'above' | 'below' | 'replace';

export type AreaAction =
  | { kind: 'split'; source: string; edge: 'top' | 'bottom'; height: number }
  | { kind: 'join'; source: string; target: string }
  | { kind: 'dock'; source: string; target: string; place: DockPlace }
  | { kind: 'none'; source: string; reason: string };

export const AREA_MIN_HEIGHT = 88;
export const AREA_GAP = 8;
/** Fraction of a target's height at its top and bottom that docks beside it;
 * the middle band replaces it. */
const DOCK_EDGE_BAND = 0.3;

const inside = (rect: AreaRect, x: number, y: number): boolean =>
  x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;

/** Panels the layout must never close (the composition viewer). */
export const isPermanentArea = (id: string): boolean => id === 'viewer';

export function resolveAreaAction(
  areas: Area[],
  sourceId: string,
  corner: AreaCorner,
  x: number,
  y: number,
  min = AREA_MIN_HEIGHT,
  gap = AREA_GAP
): AreaAction {
  const source = areas.find((area) => area.id === sourceId);
  if (!source) return { kind: 'none', source: sourceId, reason: 'Panel is not docked' };

  if (inside(source.rect, x, y)) {
    const edge = corner.startsWith('top') ? 'top' : 'bottom';
    const total = source.rect.bottom - source.rect.top;
    const height = Math.round((edge === 'top' ? y - source.rect.top : source.rect.bottom - y) - gap / 2);
    if (height < min || total - height - gap < min) {
      return { kind: 'none', source: sourceId, reason: total < min * 2 + gap ? 'Too small to split' : 'Drag further to split' };
    }
    return { kind: 'split', source: sourceId, edge, height };
  }

  const target = areas.find((area) => area.id !== sourceId && inside(area.rect, x, y));
  if (!target) return { kind: 'none', source: sourceId, reason: 'Drag into a panel' };

  if (target.dockId === source.dockId && Math.abs(target.index - source.index) === 1) {
    if (isPermanentArea(target.id)) return { kind: 'none', source: sourceId, reason: 'The viewer cannot be closed' };
    return { kind: 'join', source: sourceId, target: target.id };
  }

  const relative = (y - target.rect.top) / Math.max(1, target.rect.bottom - target.rect.top);
  let place: DockPlace = relative < DOCK_EDGE_BAND ? 'above' : relative > 1 - DOCK_EDGE_BAND ? 'below' : 'replace';
  if (place === 'replace' && isPermanentArea(target.id)) place = relative < 0.5 ? 'above' : 'below';
  return { kind: 'dock', source: sourceId, target: target.id, place };
}

type Close = (workspace: Workspace, id: string) => boolean;

const growsWithDock = (spec: PanelSpec): boolean => !keepPanelAtSetHeight({ ...spec }) && !!spec.flex;

/** Source absorbs its neighbour: the neighbour closes and its height moves over. */
export function joinAreas(
  workspace: Workspace,
  keepId: string,
  closeId: string,
  heights: { keep: number; close: number },
  close: Close = hidePanel,
  gap = AREA_GAP
): boolean {
  const keep = findPanel(workspace, keepId);
  const gone = findPanel(workspace, closeId);
  if (!keep || !gone || keep.dock !== gone.dock || isPermanentArea(closeId)) return false;
  const fixedHeight = !!keepPanelAtSetHeight({ ...keep.spec });
  /* Claim the fill role before closing, or the dock would hand it to some
     third panel and that panel would change size too. */
  if (!fixedHeight && (keep.spec.flex || gone.spec.flex)) keep.spec.flex = true;
  else keep.spec.size = Math.round((keep.spec.size || heights.keep) + heights.close + gap);
  return close(workspace, closeId);
}

/** Inserts `newId` into the part of `sourceId` between the dragged corner and the split line. */
export function splitArea(
  workspace: Workspace,
  sourceId: string,
  newId: string,
  edge: 'top' | 'bottom',
  heights: { source: number; part: number },
  gap = AREA_GAP
): boolean {
  const found = findPanel(workspace, sourceId);
  if (!found || sourceId === newId || findPanel(workspace, newId)) return false;
  const index = found.dock.panels.indexOf(found.spec);
  if (!growsWithDock(found.spec)) {
    found.spec.size = Math.max(AREA_MIN_HEIGHT, Math.round((found.spec.size || heights.source) - heights.part - gap));
  }
  workspace.hiddenPanels = (workspace.hiddenPanels ?? []).filter((item) => item.id !== newId);
  insertPanel(workspace, { id: newId, size: Math.round(heights.part) }, found.dock.id, edge === 'top' ? index : index + 1);
  return true;
}

/** Moves `sourceId` beside `targetId`, taking half its height, or into its slot. */
export function dockArea(
  workspace: Workspace,
  sourceId: string,
  targetId: string,
  place: DockPlace,
  targetHeight: number,
  close: Close = hidePanel,
  gap = AREA_GAP
): boolean {
  if (sourceId === targetId || !findPanel(workspace, sourceId) || !findPanel(workspace, targetId)) return false;
  if (place === 'replace' && isPermanentArea(targetId)) return false;
  removePanel(workspace, sourceId);
  const target = findPanel(workspace, targetId)!;
  const dockId = target.dock.id;
  const index = target.dock.panels.indexOf(target.spec);
  if (place === 'replace') {
    const slot: PanelSpec = { id: sourceId };
    if (target.spec.size) slot.size = target.spec.size;
    if (target.spec.flex) slot.flex = true;
    if (!close(workspace, targetId)) return false;
    insertPanel(workspace, slot, dockId, index);
    return true;
  }
  const half = Math.max(AREA_MIN_HEIGHT, Math.round((targetHeight - gap) / 2));
  if (!growsWithDock(target.spec)) {
    target.spec.size = Math.max(AREA_MIN_HEIGHT, Math.round((target.spec.size || targetHeight) - half - gap));
  }
  insertPanel(workspace, { id: sourceId, size: half }, dockId, place === 'above' ? index : index + 1);
  return true;
}
