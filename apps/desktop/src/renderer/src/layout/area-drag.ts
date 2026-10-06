import type { PMRegistry } from '../legacy/registry';
import { AREA_GAP, AREA_MIN_HEIGHT, dockArea, joinAreas, resolveAreaAction, splitArea, type Area, type AreaAction, type AreaCorner, type AreaRect } from './area';
import { animatePanelLayout, panelRects } from './drag';
import { findPanel, type Workspace } from './model';

const CORNERS: AreaCorner[] = ['top-left', 'top-right', 'bottom-left', 'bottom-right'];

/* DOMRect fields are prototype getters, so `{ ...rect }` would copy nothing. */
const plainRect = ({ left, right, top, bottom }: DOMRect): AreaRect => ({ left, right, top, bottom });

function liveAreas(): Area[] {
  return [...document.querySelectorAll<HTMLElement>('#body > .dock')].flatMap((dock) =>
    [...dock.querySelectorAll<HTMLElement>('.panel[data-panel]')].map((panel, index) => ({
      id: panel.dataset.panel!,
      dockId: dock.dataset.dock || dock.id.replace('dock-', ''),
      index,
      rect: plainRect(panel.getBoundingClientRect())
    }))
  );
}

const title = (PM: PMRegistry, id: string): string => PM.PANELS[id]?.title || id;

/** Panels that could fill the new half of a split: registered, offered by the
 * Library, and not already open in this window or a popout. */
function splitCandidates(PM: PMRegistry, workspace: Workspace): Array<{ id: string; title: string }> {
  return Object.values(PM.PANELS as Record<string, any>)
    .filter((def) => def.id !== 'toolbar' && def.library !== false && !PM.Layout.hasPanel(workspace, def.id) && PM.Popout?.isOpen?.(def.id) !== true)
    .map((def) => ({ id: def.id as string, title: def.title as string }))
    .sort((a, b) => a.title.localeCompare(b.title));
}

function place(element: HTMLElement, rect: AreaRect | null): void {
  element.classList.toggle('on', !!rect);
  if (!rect) return;
  element.style.left = `${Math.round(rect.left)}px`;
  element.style.top = `${Math.round(rect.top)}px`;
  element.style.width = `${Math.round(rect.right - rect.left)}px`;
  element.style.height = `${Math.round(rect.bottom - rect.top)}px`;
}

/** Drag from a panel corner: split, join or dock it, as in Blender's area corners. */
export function beginAreaDrag(PM: PMRegistry, event: PointerEvent, sourceId: string, corner: AreaCorner): void {
  const element = PM.panelInst[sourceId]?.el as HTMLElement | undefined;
  if (!element) return;
  const shade = document.createElement('div');
  shade.className = 'area-op-shade';
  const arrow = document.createElement('span');
  arrow.className = 'area-op-arrow';
  shade.appendChild(arrow);
  const zone = document.createElement('div');
  zone.className = 'area-op-zone';
  const line = document.createElement('div');
  line.className = 'area-op-line';
  const ghost = document.createElement('div');
  ghost.className = 'panel-ghost area-op-hint';
  const card = document.createElement('div');
  card.className = 'panel-ghost-card';
  const label = document.createElement('span');
  label.className = 'panel-ghost-label';
  const detail = document.createElement('span');
  detail.className = 'panel-ghost-destination';
  card.append(label, detail);
  ghost.appendChild(card);

  let moved = false;
  let frame = 0;
  let pending: PointerEvent | null = null;
  let areas: Area[] = [];
  let gap = AREA_GAP;
  let action: AreaAction = { kind: 'none', source: sourceId, reason: '' };

  const render = (): void => {
    frame = 0;
    const next = pending;
    pending = null;
    if (!next) return;
    action = resolveAreaAction(areas, sourceId, corner, next.clientX, next.clientY, AREA_MIN_HEIGHT, gap);
    const source = areas.find((area) => area.id === sourceId)!;
    const resolved = action;
    const target = 'target' in resolved ? areas.find((area) => area.id === resolved.target) : undefined;
    let shaded: AreaRect | null = null;
    let highlighted: AreaRect | null = null;
    let split: AreaRect | null = null;
    arrow.textContent = '';
    if (action.kind === 'split') {
      const r = source.rect;
      const y = action.edge === 'top' ? r.top + action.height + gap / 2 : r.bottom - action.height - gap / 2;
      split = { left: r.left, right: r.right, top: y - 1, bottom: y + 1 };
      highlighted = action.edge === 'top' ? { ...r, bottom: y } : { ...r, top: y };
      label.textContent = 'Split area';
      detail.textContent = `New panel ${action.edge === 'top' ? 'above' : 'below'}`;
    } else if (action.kind === 'join' && target) {
      // Blender darkens the area that is about to disappear and points into it.
      shaded = target.rect;
      arrow.textContent = target.rect.top > source.rect.top ? '↓' : '↑';
      label.textContent = 'Join areas';
      detail.textContent = `${title(PM, target.id)} closes`;
    } else if (action.kind === 'dock' && target) {
      const r = target.rect;
      const mid = (r.top + r.bottom) / 2;
      highlighted = action.place === 'replace' ? r : action.place === 'above' ? { ...r, bottom: mid } : { ...r, top: mid };
      shaded = action.place === 'replace' ? r : source.rect;
      label.textContent = action.place === 'replace' ? 'Replace this area' : 'Move area here';
      detail.textContent = action.place === 'replace'
        ? `${title(PM, target.id)} closes`
        : `${action.place === 'above' ? 'Above' : 'Below'} ${title(PM, target.id)}`;
    } else {
      label.textContent = 'Split / dock';
      detail.textContent = action.kind === 'none' ? action.reason : '';
    }
    place(shade, shaded);
    place(zone, highlighted);
    place(line, split);
    const x = PM.clamp(next.clientX + 14, 8, window.innerWidth - ghost.offsetWidth - 8);
    const y = PM.clamp(next.clientY + 14, 8, window.innerHeight - ghost.offsetHeight - 8);
    ghost.style.transform = `translate3d(${x}px,${y}px,0)`;
  };

  const height = (id: string): number => {
    const area = areas.find((item) => item.id === id);
    return area ? area.rect.bottom - area.rect.top : 0;
  };
  const close = (workspace: Workspace, id: string): boolean => PM.Layout.closePanel(workspace, id);
  const commit = (mutate: (workspace: Workspace) => boolean, message: string): void => {
    const previous = panelRects();
    let changed = false;
    PM.WS.mutate((workspace: Workspace) => { changed = mutate(workspace); });
    if (!changed) return;
    animatePanelLayout(previous);
    PM.toast(message);
  };

  const apply = (last: PointerEvent): void => {
    const done = action;
    if (done.kind === 'join') {
      commit((ws) => joinAreas(ws, sourceId, done.target, { keep: height(sourceId), close: height(done.target) }, close, gap),
        `Joined ${title(PM, done.target)} into ${title(PM, sourceId)}`);
    } else if (done.kind === 'dock') {
      if (done.place === 'replace') PM.Layout.rememberPanelOpen?.(sourceId);
      commit((ws) => dockArea(ws, sourceId, done.target, done.place, height(done.target), close, gap),
        done.place === 'replace'
          ? `${title(PM, sourceId)} replaced ${title(PM, done.target)}`
          : `Moved ${title(PM, sourceId)} ${done.place} ${title(PM, done.target)}`);
    } else if (done.kind === 'split') {
      const candidates = splitCandidates(PM, PM.WS.current as Workspace);
      if (!candidates.length) {
        PM.toast('Every panel is already open');
        return;
      }
      const sizes = { source: height(sourceId), part: done.height };
      PM.menu(element, [
        { header: `New panel ${done.edge === 'top' ? 'above' : 'below'} ${title(PM, sourceId)}` },
        ...candidates.map((candidate) => ({
          label: candidate.title,
          run: () => {
            PM.Layout.rememberPanelOpen?.(candidate.id);
            commit((ws) => splitArea(ws, sourceId, candidate.id, done.edge, sizes, gap), `Split ${title(PM, sourceId)} · added ${candidate.title}`);
          }
        }))
      ], { x: last.clientX, y: last.clientY });
    }
  };

  const finish = (cancelled: boolean, last?: PointerEvent): void => {
    if (frame) window.cancelAnimationFrame(frame);
    window.removeEventListener('keydown', onKeyDown, true);
    for (const node of [shade, zone, line, ghost]) node.remove();
    document.body.classList.remove('area-dragging');
    if (!moved || cancelled || !last) return;
    apply(last);
  };

  const drag = PM.drag(event, {
    cursor: 'crosshair',
    move: (dx: number, dy: number, next: PointerEvent) => {
      if (!moved && Math.hypot(dx, dy) < 4) return;
      if (!moved) {
        moved = true;
        areas = liveAreas();
        gap = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--gap')) || AREA_GAP;
        document.body.classList.add('area-dragging');
        document.body.append(shade, zone, line, ghost);
        ghost.classList.add('on');
      }
      pending = next;
      if (!frame) frame = window.requestAnimationFrame(render);
    },
    up: (_dx: number, _dy: number, next: PointerEvent) => {
      if (moved) {
        pending = next;
        render();
      }
      finish(false, next);
    },
    cancel: () => finish(true)
  });
  const onKeyDown = (keyEvent: KeyboardEvent): void => {
    if (keyEvent.key !== 'Escape') return;
    keyEvent.preventDefault();
    keyEvent.stopPropagation();
    drag.cancel();
  };
  window.addEventListener('keydown', onKeyDown, true);
}

/** Adds the four corner action zones to a docked panel's frame. */
export function attachAreaCorners(PM: PMRegistry, element: HTMLElement, id: string): void {
  if (element.querySelector(':scope > .panel-corner')) return;
  for (const corner of CORNERS) {
    const handle = document.createElement('span');
    handle.className = 'panel-corner';
    handle.dataset.corner = corner;
    handle.title = 'Drag to split, join or dock';
    handle.addEventListener('pointerdown', (event: PointerEvent) => {
      if (event.button !== 0 || element.classList.contains('popped')) return;
      if (!PM.Layout.ws || !findPanel(PM.Layout.ws as Workspace, id)) return;
      event.stopPropagation();
      beginAreaDrag(PM, event, id, corner);
    });
    element.appendChild(handle);
  }
}
