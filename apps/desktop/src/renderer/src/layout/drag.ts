import type { PMRegistry } from '../legacy/registry';
import { resolvePanelZone, type PanelZone, type ZonePanel, type ZoneRect } from './dock-zones';
import { dockLabel, placePanels, stackOf, type DockSpec, type PanelPlacement, type PanelSpec, type Workspace } from './model';

type RectTarget = { id: string; el?: HTMLElement; virtual?: boolean; rect: DOMRect; panels: Array<{ el: HTMLElement; rect: DOMRect }> };
type PreviewTarget = { key: string };

const PANEL_SELECTOR = '#body .dock .panel[data-panel]';

export function panelRects(): Map<string, DOMRect> {
  const rects = new Map<string, DOMRect>();
  document.querySelectorAll<HTMLElement>(PANEL_SELECTOR).forEach((node) => {
    if (node.dataset.panel) rects.set(node.dataset.panel, node.getBoundingClientRect());
  });
  return rects;
}

export function animatePanelLayout(previous: Map<string, DOMRect>): void {
  if (!previous.size || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;
  document.querySelectorAll<HTMLElement>(PANEL_SELECTOR).forEach((node) => {
    const before = previous.get(node.dataset.panel || '');
    if (!before || typeof node.animate !== 'function') return;
    const after = node.getBoundingClientRect();
    const dx = before.left - after.left;
    const dy = before.top - after.top;
    if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
    node.animate(
      [
        { transform: `translate3d(${Math.round(dx)}px,${Math.round(dy)}px,0)` },
        { transform: 'translate3d(0,0,0)' }
      ],
      { duration: 180, easing: 'cubic-bezier(.22,1,.36,1)' }
    );
  });
}

export function retargetPreview(preview: HTMLElement, target: PreviewTarget, key: string): void {
  if (target.key === key) {
    preview.classList.add('on');
    return;
  }
  target.key = key;
  preview.classList.remove('on');
  window.requestAnimationFrame(() => {
    if (preview.isConnected && target.key === key) preview.classList.add('on');
  });
}

function liveTargets(PM: PMRegistry): RectTarget[] {
  const docks: RectTarget[] = [...document.querySelectorAll<HTMLElement>('#body .dock')].map((element) => ({
    id: element.id.replace('dock-', ''),
    el: element,
    rect: element.getBoundingClientRect(),
    // A stack's hidden tabs have no box; only the shown one takes a place.
    panels: [...element.querySelectorAll<HTMLElement>('.panel[data-panel]')]
      .map((panel) => ({ el: panel, rect: panel.getBoundingClientRect() }))
      .filter((panel) => panel.rect.height > 0)
  }));
  const bodyRect = document.getElementById('body')?.getBoundingClientRect();
  return PM.Layout.buildDockDropTargets(docks, bodyRect) as RectTarget[];
}

function placePreview(
  PM: PMRegistry,
  preview: HTMLElement,
  target: { dockId: string; index: number },
  docks: RectTarget[],
  previewTarget: PreviewTarget
): void {
  const dock = docks.find((item) => item.id === target.dockId);
  if (!dock) return;
  const rect = dock.rect;
  if (dock.virtual && dock.id === 'bottom') {
    const height = Math.min(300, Math.max(120, Math.round(rect.height * 2.5)));
    preview.classList.add('dock-edge');
    preview.style.left = `${Math.round(rect.left)}px`;
    preview.style.top = `${Math.round(rect.bottom - height)}px`;
    preview.style.width = `${Math.round(rect.width)}px`;
    preview.style.height = `${height}px`;
    retargetPreview(preview, previewTarget, 'virtual:bottom');
  } else if (dock.virtual) {
    const width = Math.min(dock.id === 'right' ? 300 : 250, Math.max(120, Math.round(rect.width * 2.5)));
    preview.classList.add('dock-edge');
    preview.style.left = `${Math.round(dock.id === 'right' ? rect.right - width : rect.left)}px`;
    preview.style.top = `${Math.round(rect.top)}px`;
    preview.style.width = `${width}px`;
    preview.style.height = `${Math.max(80, Math.round(rect.height))}px`;
    retargetPreview(preview, previewTarget, `virtual:${dock.id}`);
  } else {
    preview.classList.remove('dock-edge');
    const before = dock.panels[target.index];
    const after = target.index > 0 ? dock.panels[target.index - 1] : undefined;
    let y = before ? before.rect.top - 3 : after ? after.rect.bottom + 3 : rect.top + 8;
    y = PM.clamp(y, rect.top + 4, rect.bottom - 8);
    preview.style.left = `${Math.round(rect.left + 8)}px`;
    preview.style.top = `${Math.round(y)}px`;
    preview.style.width = `${Math.max(24, Math.round(rect.width - 16))}px`;
    preview.style.height = '';
    retargetPreview(preview, previewTarget, `${dock.el?.id || `dock-${dock.id}`}:${target.index}`);
  }
}

/** The shown panels under the docks, measured for the AE drop zones. */
function zonePanels(PM: PMRegistry): ZonePanel[] {
  const ws = PM.Layout.ws as Workspace | null;
  const box = (rect: DOMRect): ZoneRect => ({ left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom });
  return [...document.querySelectorAll<HTMLElement>(PANEL_SELECTOR)].flatMap((element) => {
    const id = element.dataset.panel!;
    const rect = element.getBoundingClientRect();
    if (!rect.height || element.dataset.collapsed === '1') return [];
    const def = PM.PANELS[id] || {};
    const header = def.headless ? null : element.querySelector<HTMLElement>(':scope > header');
    const tabs = header ? [...header.querySelectorAll<HTMLElement>(':scope > .ptabs > .ptab')] : [];
    const title = header?.querySelector<HTMLElement>(':scope > .ptitle');
    return [{
      id,
      stack: ws ? stackOf(ws, id).map((spec) => spec.id) : [id],
      rect: box(rect),
      header: header ? box(header.getBoundingClientRect()) : null,
      tabs: (tabs.length ? tabs : title ? [title] : []).map((tab) => {
        const tabRect = tab.getBoundingClientRect();
        return { id: tab.dataset.tab || id, left: tabRect.left, right: tabRect.right };
      }),
      groupable: !def.headless && id !== 'viewer'
    }];
  });
}

/** Whether a placement would change anything, tried on a copy first. */
function changesLayout(workspace: Workspace, specs: PanelSpec[], placement: PanelPlacement): boolean {
  const probe = JSON.parse(JSON.stringify(workspace)) as Workspace;
  const shape = (ws: Workspace) => JSON.stringify(ws.layout.docks.map((dock) => [dock.id, dock.panels.map((spec) => [spec.id, spec.group ?? null])]));
  return placePanels(probe, specs, placement) && shape(probe) !== shape(workspace);
}

type FloatingDrag = {
  move(event: PointerEvent): void;
  finish(cancelled: boolean): void;
  dockIds?: string[];
};

/**
 * Drag a panel by its header, or a whole tab stack by passing `stack`. Over a
 * panel the After Effects drop zones apply (see dock-zones.ts); elsewhere in a
 * column, or at a window edge, it docks between panels as before.
 */
export function beginPanelDrag(
  PM: PMRegistry,
  event: PointerEvent,
  spec: PanelSpec,
  _dock: DockSpec,
  element: HTMLElement,
  floating?: FloatingDrag,
  stack?: PanelSpec[]
): void {
  const moving = stack && stack.length > 1 ? stack : [spec];
  const ids = moving.map((item) => item.id);
  const name = (id: string): string => PM.PANELS[id]?.title || id;
  const movingName = moving.map((item) => item.title || name(item.id)).join(' + ');
  const ghost = document.createElement('div');
  ghost.className = 'panel-ghost';
  const card = document.createElement('div');
  card.className = 'panel-ghost-card';
  const label = document.createElement('span');
  label.className = 'panel-ghost-label';
  label.textContent = movingName;
  const destination = document.createElement('span');
  destination.className = 'panel-ghost-destination';
  card.append(label, destination);
  ghost.appendChild(card);
  const preview = document.createElement('div');
  preview.className = 'panel-drop-preview';
  /* AE outlines the frame under the pointer and fills the zone the panel
     will take; a tab marks where it joins a stack. */
  const frameOutline = document.createElement('div');
  frameOutline.className = 'panel-drop-frame';
  const tabMark = document.createElement('div');
  tabMark.className = 'panel-drop-tab';
  tabMark.textContent = moving.length > 1 ? `${moving.length} panels` : movingName;
  document.body.append(ghost, frameOutline, preview, tabMark);
  if (!floating) element.classList.add('drag-src');
  /* Panels that cannot show a tab strip cannot join a stack. */
  const canGroup = !floating && moving.every((item) => !PM.PANELS[item.id]?.headless && item.id !== 'viewer');

  let moved = false;
  let frame = 0;
  let pending: PointerEvent | null = null;
  let dropTarget: { dockId: string; index: number; before: string | null } | null = null;
  let zone: PanelZone | null = null;
  const previewTarget: PreviewTarget = { key: '' };

  const showBox = (node: HTMLElement, rect: ZoneRect | null): void => {
    node.classList.toggle('on', !!rect);
    if (!rect) return;
    node.style.left = `${Math.round(rect.left)}px`;
    node.style.top = `${Math.round(rect.top)}px`;
    node.style.width = `${Math.round(rect.right - rect.left)}px`;
    node.style.height = `${Math.round(rect.bottom - rect.top)}px`;
  };

  const render = (): void => {
    frame = 0;
    const next = pending;
    pending = null;
    if (!next) return;
    zone = null;
    dropTarget = null;
    const panels = floating ? [] : zonePanels(PM);
    zone = resolvePanelZone(panels, next.clientX, next.clientY, ids, canGroup);
    const zonePanel = zone ? panels.find((item) => item.id === zone!.target) : undefined;
    showBox(frameOutline, zonePanel?.rect ?? null);
    tabMark.classList.toggle('on', zone?.kind === 'group');
    if (zone?.kind === 'group' && zone.tab) {
      tabMark.style.left = `${Math.round(zone.tab.left)}px`;
      tabMark.style.top = `${Math.round(zone.tab.top)}px`;
      tabMark.style.height = `${Math.round(zone.tab.height)}px`;
    }
    if (zone) {
      preview.classList.remove('dock-edge');
      if (zone.kind === 'self') {
        preview.classList.remove('on', 'zone');
        previewTarget.key = '';
        destination.textContent = 'Already here';
      } else {
        preview.classList.add('zone');
        showBox(preview, zone.zone);
        retargetPreview(preview, previewTarget, `zone:${zone.kind}:${zone.target}:${JSON.stringify(zone.placement)}`);
        const placement = zone.placement;
        destination.textContent = placement.kind === 'group'
          ? `Tab with ${name(zone.target)}`
          : placement.kind === 'beside' ? `${placement.place === 'above' ? 'Above' : 'Below'} ${name(zone.target)}` : '';
      }
    } else {
      preview.classList.remove('zone');
      preview.style.height = '';
      const docks = liveTargets(PM).filter(dock => !floating?.dockIds || floating.dockIds.includes(dock.id));
      const hit = PM.Layout.hitTestDockPlacement(docks, next.clientX, next.clientY) as { dockId: string; index: number } | null;
      if (hit) {
        const targetDock = docks.find((item) => item.id === hit.dockId);
        dropTarget = { ...hit, before: targetDock?.panels[hit.index]?.el.dataset.panel ?? null };
      }
      if (floating) {
        floating.move(next);
        // Leave the middle of the workspace free for floating. Dock only at
        // column edges or between panels, using the existing placement preview.
        const target = docks.find(item => item.id === dropTarget?.dockId);
        if (target && !target.virtual) {
          const nearEdge = Math.min(Math.abs(next.clientX - target.rect.left), Math.abs(next.clientX - target.rect.right),
            Math.abs(next.clientY - target.rect.top), Math.abs(next.clientY - target.rect.bottom)) <= 32;
          const nearPanel = target.panels.some(panel => Math.min(Math.abs(next.clientY - panel.rect.top), Math.abs(next.clientY - panel.rect.bottom)) <= 24);
          if (!nearEdge && !nearPanel) dropTarget = null;
        }
      }
      if (!dropTarget) {
        preview.classList.remove('on');
        previewTarget.key = '';
        destination.textContent = 'Not a drop zone';
      } else {
        const targetDock = docks.find((item) => item.id === dropTarget!.dockId);
        const before = targetDock?.panels[dropTarget.index];
        const after = dropTarget.index > 0 ? targetDock?.panels[dropTarget.index - 1] : undefined;
        destination.textContent = before
          ? `Above ${name(before.el.dataset.panel!)}`
          : after
            ? `Below ${name(after.el.dataset.panel!)}`
            : `Place in ${dockLabel(dropTarget.dockId)}`;
        placePreview(PM, preview, dropTarget, docks, previewTarget);
      }
    }
    const x = PM.clamp(next.clientX + 12, 8, window.innerWidth - ghost.offsetWidth - 8);
    const y = PM.clamp(next.clientY + 12, 8, window.innerHeight - ghost.offsetHeight - 8);
    ghost.style.transform = `translate3d(${x}px,${y}px,0)`;
  };
  const queue = (next: PointerEvent): void => {
    pending = next;
    if (!frame) frame = window.requestAnimationFrame(render);
  };
  const finish = (cancelled: boolean): void => {
    if (frame) window.cancelAnimationFrame(frame);
    window.removeEventListener('keydown', onKeyDown, true);
    for (const node of [ghost, preview, frameOutline, tabMark]) node.remove();
    element.classList.remove('drag-src');
    document.body.classList.remove('panel-dragging');
    if (!moved) return;
    if (cancelled) {
      floating?.finish(true);
      PM.toast('Move cancelled');
      return;
    }
    if (zone?.kind === 'self') {
      PM.toast(`${movingName} is already here`);
      return;
    }
    const placement: PanelPlacement | null = zone
      ? zone.placement
      : dropTarget ? { kind: 'dock', dockId: dropTarget.dockId, before: dropTarget.before } : null;
    if (!placement) {
      if (floating) { floating.finish(false); return; }
      PM.toast(`Drop on a panel or dock to move ${movingName}`);
      return;
    }
    const current = (PM.WS.current ?? PM.Layout.ws) as Workspace | null;
    if (current && !changesLayout(current, moving, placement)) {
      PM.toast(`${movingName} is already here`);
      return;
    }
    const previous = panelRects();
    PM.WS.mutate((workspace: Workspace) => {
      placePanels(workspace, moving, placement);
      for (const id of ids) PM.Layout.rememberPanelOpen?.(id);
    });
    animatePanelLayout(previous);
    PM.toast(placement.kind === 'group'
      ? `${movingName} tabbed with ${name(placement.target)}`
      : placement.kind === 'beside'
        ? `Moved ${movingName} ${placement.place} ${name(placement.target)}`
        : `Moved ${movingName} → ${dockLabel(placement.dockId)}`);
  };
  const drag = PM.drag(event, {
    cursor: 'grabbing',
    move: (dx: number, dy: number, next: PointerEvent) => {
      if (!moved && Math.hypot(dx, dy) < 5) return;
      if (!moved) {
        moved = true;
        destination.textContent = 'Move panel';
        document.body.classList.add('panel-dragging');
      }
      if (!floating) ghost.classList.add('on');
      queue(next);
    },
    up: (_dx: number, _dy: number, next: PointerEvent) => {
      if (moved) {
        pending = next;
        render();
      }
      finish(false);
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
