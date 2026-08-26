import type { Workspace } from './model';

type Rect = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width?: number;
  height?: number;
};

type DockTarget = {
  id: string;
  rect: Rect;
  panels?: Array<{ rect: Rect }>;
  el?: HTMLElement;
  virtual?: boolean;
};

export function resolveDropIndex(
  fromDockId: string,
  fromIndex: number,
  toDockId: string,
  index: number
): number | null {
  if (!Number.isInteger(index)) return null;
  if (toDockId !== fromDockId) return index;
  const adjusted = index > fromIndex ? index - 1 : index;
  return adjusted === fromIndex ? null : adjusted;
}

export function hitTestDockPlacement(
  docks: DockTarget[] | null | undefined,
  x: number,
  y: number,
  tolerance = 28
): { dockId: string; index: number } | null {
  let best: { dock: DockTarget; distance: number } | null = null;
  for (const dock of docks || []) {
    const rect = dock.rect;
    if (!rect || x < rect.left - tolerance || x > rect.right + tolerance || y < rect.top - tolerance || y > rect.bottom + tolerance) continue;
    const dx = x < rect.left ? rect.left - x : x > rect.right ? x - rect.right : 0;
    const dy = y < rect.top ? rect.top - y : y > rect.bottom ? y - rect.bottom : 0;
    const distance = dx * dx + dy * dy;
    if (!best || distance < best.distance) best = { dock, distance };
  }
  if (!best) return null;
  const panels = best.dock.panels || [];
  let index = panels.length;
  for (let panelIndex = 0; panelIndex < panels.length; panelIndex += 1) {
    const rect = panels[panelIndex]!.rect;
    if (rect && y < rect.top + (rect.height ?? rect.bottom - rect.top) / 2) {
      index = panelIndex;
      break;
    }
  }
  return { dockId: best.dock.id, index };
}

export function buildDockDropTargets<T extends DockTarget>(
  docks: T[] | null | undefined,
  bodyRect: Rect | null | undefined,
  edgeWidth = 96
): Array<T | DockTarget> {
  const targets = [...(docks || [])];
  if (!bodyRect) return targets;
  const make = (id: string, left: number, right: number): DockTarget => ({
    id,
    virtual: true,
    panels: [],
    rect: {
      left,
      right,
      top: bodyRect.top,
      bottom: bodyRect.bottom,
      width: right - left,
      height: bodyRect.bottom - bodyRect.top
    }
  });
  const virtual: DockTarget[] = [];
  if (!targets.some((dock) => dock.id === 'left')) {
    virtual.push(make('left', bodyRect.left, Math.min(bodyRect.right, bodyRect.left + edgeWidth)));
  }
  if (!targets.some((dock) => dock.id === 'right')) {
    virtual.push(make('right', Math.max(bodyRect.left, bodyRect.right - edgeWidth), bodyRect.right));
  }
  return [...virtual, ...targets];
}

export function clampPanelHeight(
  start: number,
  delta: number,
  sign: number,
  pairHeight: number,
  minHeight = 88,
  otherMinHeight = 88,
  gap = 8
): number {
  const min = Math.max(72, Number(minHeight) || 88);
  const max = Math.max(min, (Number(pairHeight) || min * 2 + gap) - Math.max(72, Number(otherMinHeight) || 88) - gap);
  return Math.max(min, Math.min(max, start + sign * delta));
}

/* A horizontal splitter sits between two panels. Which one it resizes depends
   on which of them flex-fill the dock:
   - one sized, one flex  → drag the sized one; the flex one absorbs the delta
   - both sized           → transfer height between them; nothing else moves
   - both flex            → pin the panel after the splitter to a size
   The old model assumed exactly one flex neighbour and silently converted the
   other to flex, which threw away its size and let a third panel absorb the
   drag instead. */
export type PairResize =
  | { mode: 'single'; target: 'before' | 'after'; sign: 1 | -1 }
  | { mode: 'transfer' };

export function resolvePairResize(beforeFlex: boolean, afterFlex: boolean): PairResize {
  if (!beforeFlex && !afterFlex) return { mode: 'transfer' };
  if (!beforeFlex) return { mode: 'single', target: 'before', sign: 1 };
  return { mode: 'single', target: 'after', sign: -1 };
}

/* Moves `delta` px from the panel after the splitter to the one before it
   (dragging down), honoring both minimums. The pair's combined height is
   invariant, so a third panel can never absorb the change. */
export function transferPanelHeights(
  beforeStart: number,
  afterStart: number,
  delta: number,
  beforeMin = 88,
  afterMin = 88
): { before: number; after: number } {
  const total = beforeStart + afterStart;
  const minBefore = Math.max(72, Number(beforeMin) || 88);
  const minAfter = Math.max(72, Number(afterMin) || 88);
  const before = Math.max(minBefore, Math.min(total - minAfter, beforeStart + delta));
  return { before, after: total - before };
}

export function visibleDockPlan(
  workspace: Workspace
): Array<{ dock: Workspace['layout']['docks'][number]; specs: Workspace['layout']['docks'][number]['panels'] }> {
  return (workspace.layout?.docks || [])
    .map((dock) => ({ dock, specs: dock.panels || [] }))
    .filter((item) => !item.dock.hidden && item.specs.length > 0);
}
