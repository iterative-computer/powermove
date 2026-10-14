import type { PMRegistry } from '../legacy/registry';
import { dockLabel, findSlot, insertPanel, removePanel, stackPanel, type DockSpec, type PanelSpec, type Workspace } from './model';
import { canStack, liveTabTargets, resolveTabDrop, type TabDrop, type TabDropTarget } from './tabs';

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
    const animation = node.animate(
      [
        { transform: `translate3d(${Math.round(dx)}px,${Math.round(dy)}px,0)` },
        { transform: 'translate3d(0,0,0)' }
      ],
      { duration: 180, easing: 'cubic-bezier(.22,1,.36,1)' }
    );
    /* The dock clips panels that cross its edge (rounded-scroll), measuring
       them mid-flight; nothing resizes when the slide ends, so ask it to
       measure the settled panel once more. */
    void animation?.finished?.then(() => node.closest('.dock')?.dispatchEvent(new Event('scroll')), () => {});
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
    panels: [...element.querySelectorAll<HTMLElement>('.panel[data-panel]')].map((panel) => ({
      el: panel,
      rect: panel.getBoundingClientRect()
    }))
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
  preview.classList.remove('tab-edge');
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

function placeTabPreview(preview: HTMLElement, target: TabDropTarget, drop: TabDrop, previewTarget: PreviewTarget): void {
  if (drop.zone === 'center') {
    preview.classList.remove('tab-edge');
    preview.classList.add('dock-edge');
    preview.style.left = `${Math.round(target.body.left)}px`;
    preview.style.top = `${Math.round(target.header.top)}px`;
    preview.style.width = `${Math.round(target.body.right - target.body.left)}px`;
    preview.style.height = `${Math.round(target.body.bottom - target.header.top)}px`;
  } else {
    preview.classList.remove('dock-edge');
    preview.classList.add('tab-edge');
    preview.style.left = `${Math.round(drop.x - 1)}px`;
    preview.style.top = `${Math.round(target.header.top + 8)}px`;
    preview.style.width = '';
    preview.style.height = `${Math.max(8, Math.round(target.header.bottom - target.header.top - 16))}px`;
  }
  retargetPreview(preview, previewTarget, `tabs:${target.id}:${drop.zone}:${drop.beforeId ?? ''}`);
}

type FloatingDrag = { move(dx: number, dy: number): void; cancel(): void; dockIds: string[] };

/**
 * Drags a panel to another dock position, or onto a panel header or the
 * middle of a panel to join it as a tab. `onTap` runs for a press released
 * without dragging.
 */
export function beginPanelDrag(
  PM: PMRegistry,
  event: PointerEvent,
  spec: PanelSpec,
  dock: DockSpec,
  element: HTMLElement,
  floating?: FloatingDrag,
  onTap?: () => void
): void {
  const ghost = document.createElement('div');
  ghost.className = 'panel-ghost';
  const card = document.createElement('div');
  card.className = 'panel-ghost-card';
  const label = document.createElement('span');
  label.className = 'panel-ghost-label';
  label.textContent = PM.PANELS[spec.id]?.title || spec.id;
  const destination = document.createElement('span');
  destination.className = 'panel-ghost-destination';
  card.append(label, destination);
  ghost.appendChild(card);
  const preview = document.createElement('div');
  preview.className = 'panel-drop-preview';
  document.body.append(ghost, preview);
  element.classList.add('drag-src');

  /* The source's slot position among the visible slots, which is what drop
     indexes count. A tab leaving its group always moves, even to its own spot. */
  const sourceSlot = PM.Layout.ws ? findSlot(PM.Layout.ws as Workspace, spec.id) : null;
  const tabbed = (sourceSlot?.slot.length ?? 0) > 1;
  const sourceIds = new Set((sourceSlot?.slot ?? [spec]).map((panel) => panel.id));
  const fromIndex = Math.max(0, [...document.querySelectorAll<HTMLElement>(`#dock-${CSS.escape(dock.id)} .panel[data-panel]`)]
    .findIndex((panel) => sourceIds.has(panel.dataset.panel!)));
  const stacks = !floating && canStack(PM, spec.id);
  let moved = false;
  let frame = 0;
  let pending: PointerEvent | null = null;
  let dropTarget: { dockId: string; index: number; beforeId: string | null } | null = null;
  let tabDrop: TabDrop | 'same' | null = null;
  const previewTarget: PreviewTarget = { key: '' };

  const render = (): void => {
    frame = 0;
    const next = pending;
    pending = null;
    if (!next) return;
    const tabTargets = stacks ? liveTabTargets(PM) : [];
    tabDrop = stacks ? resolveTabDrop(tabTargets, spec.id, next.clientX, next.clientY) : null;
    const docks = tabDrop ? [] : liveTargets(PM).filter(target => !floating || floating.dockIds.includes(target.id));
    const hit = tabDrop ? null : PM.Layout.hitTestDockPlacement(docks, next.clientX, next.clientY) as { dockId: string; index: number } | null;
    dropTarget = hit && {
      ...hit,
      beforeId: docks.find((item) => item.id === hit.dockId)?.panels[hit.index]?.el.dataset.panel ?? null
    };
    if (floating) ghost.classList.toggle('on', !!dropTarget);
    if (tabDrop && tabDrop !== 'same') {
      const drop = tabDrop;
      destination.textContent = `Tab with ${PM.PANELS[drop.targetId]?.title || 'panel'}`;
      placeTabPreview(preview, tabTargets.find((target) => target.id === drop.targetId)!, drop, previewTarget);
    } else if (!dropTarget) {
      preview.classList.remove('on');
      previewTarget.key = '';
      destination.textContent = tabDrop === 'same' ? 'Already here' : 'Not a drop zone';
    } else {
      const targetDock = docks.find((item) => item.id === dropTarget!.dockId);
      const before = targetDock?.panels[dropTarget.index];
      const after = dropTarget.index > 0 ? targetDock?.panels[dropTarget.index - 1] : undefined;
      const own = [before, after].some((panel) => panel && tabbed && sourceIds.has(panel.el.dataset.panel!));
      destination.textContent = own
        ? 'Separate tab'
        : before
          ? `Above ${PM.PANELS[before.el.dataset.panel!]?.title || 'panel'}`
          : after
            ? `Below ${PM.PANELS[after.el.dataset.panel!]?.title || 'panel'}`
            : `Place in ${dockLabel(dropTarget.dockId)}`;
      placePreview(PM, preview, dropTarget, docks, previewTarget);
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
    ghost.remove();
    preview.remove();
    element.classList.remove('drag-src');
    document.body.classList.remove('panel-dragging');
    if (!moved) {
      if (!cancelled) onTap?.();
      return;
    }
    if (cancelled) {
      floating?.cancel();
      if (!floating) PM.toast('Move cancelled');
      return;
    }
    const title = PM.PANELS[spec.id]?.title || spec.id;
    if (tabDrop === 'same') return;
    if (tabDrop) {
      const { targetId, beforeId } = tabDrop;
      const previous = panelRects();
      PM.Layout.rememberPanelOpen?.(spec.id);
      PM.WS.mutate((workspace: Workspace) => {
        stackPanel(workspace, spec.id, targetId, beforeId, (id) => canStack(PM, id));
      });
      animatePanelLayout(previous);
      return;
    }
    if (!dropTarget) {
      if (floating) return;
      PM.toast(`Drop on a panel or dock to move ${title}`);
      return;
    }
    const index = PM.Layout.resolveDropIndex(dock.id, fromIndex, dropTarget.dockId, dropTarget.index);
    if (index == null && !tabbed) {
      PM.toast(`${title} is already here`);
      return;
    }
    const { dockId, beforeId } = dropTarget;
    const previous = panelRects();
    PM.Layout.rememberPanelOpen?.(spec.id);
    PM.WS.mutate((workspace: Workspace) => {
      /* Drop indexes count visible slots; insert before the slot the pointer
         was above, found by a panel that stays in it after the move. */
      const anchor = beforeId == null ? null
        : findSlot(workspace, beforeId)?.slot.find((panel) => panel.id !== spec.id) ?? null;
      removePanel(workspace, spec.id);
      const target = anchor && findSlot(workspace, anchor.id);
      insertPanel(workspace, spec, dockId, target ? target.dock.panels.indexOf(target.slot[0]!) : undefined);
    });
    animatePanelLayout(previous);
    PM.toast(`Moved ${title} → ${dockLabel(dockId)}`);
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
      floating?.move(dx, dy);
      if (!floating) ghost.classList.add('on');
      queue(next);
    },
    up: (_dx: number, _dy: number, next: PointerEvent) => {
      if (moved) {
        floating?.move(_dx, _dy);
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
