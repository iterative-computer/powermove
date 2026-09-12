import type { PMRegistry } from '../legacy/registry';
import { dockLabel, insertPanel, removePanel, type DockSpec, type PanelSpec, type Workspace } from './model';

type RectTarget = { id: string; el?: HTMLElement; virtual?: boolean; rect: DOMRect; panels: Array<{ el: HTMLElement; rect: DOMRect }> };
type PreviewTarget = { key: string };

const PANEL_SELECTOR = '#body > .dock .panel[data-panel]';

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
  const docks: RectTarget[] = [...document.querySelectorAll<HTMLElement>('#body > .dock')].map((element) => ({
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
  if (dock.virtual) {
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

export function beginPanelDrag(PM: PMRegistry, event: PointerEvent, spec: PanelSpec, dock: DockSpec, element: HTMLElement): void {
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

  const fromIndex = Math.max(0, dock.panels.findIndex((panel) => panel.id === spec.id));
  let moved = false;
  let frame = 0;
  let pending: PointerEvent | null = null;
  let dropTarget: { dockId: string; index: number } | null = null;
  const previewTarget: PreviewTarget = { key: '' };

  const render = (): void => {
    frame = 0;
    const next = pending;
    pending = null;
    if (!next) return;
    const docks = liveTargets(PM);
    dropTarget = PM.Layout.hitTestDockPlacement(docks, next.clientX, next.clientY);
    if (!dropTarget) {
      preview.classList.remove('on');
      previewTarget.key = '';
      destination.textContent = 'Not a drop zone';
    } else {
      const targetDock = docks.find((item) => item.id === dropTarget!.dockId);
      const before = targetDock?.panels[dropTarget.index];
      const after = dropTarget.index > 0 ? targetDock?.panels[dropTarget.index - 1] : undefined;
      destination.textContent = before
        ? `Above ${PM.PANELS[before.el.dataset.panel!]?.title || 'panel'}`
        : after
          ? `Below ${PM.PANELS[after.el.dataset.panel!]?.title || 'panel'}`
          : `Place in ${dockLabel(dropTarget.dockId)}`;
      placePreview(PM, preview, dropTarget, docks, previewTarget);
    }
    const x = PM.clamp(next.clientX + 12, 8, window.innerWidth - ghost.offsetWidth - 8);
    const y = PM.clamp(next.clientY + 12, 8, window.innerHeight - ghost.offsetHeight - 8);
    ghost.style.transform = `translate3d(${Math.round(x)}px,${Math.round(y)}px,0)`;
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
    if (!moved) return;
    if (cancelled) {
      PM.toast('Move cancelled');
      return;
    }
    if (!dropTarget) {
      PM.toast(`Drop on a panel or dock to move ${PM.PANELS[spec.id]?.title || spec.id}`);
      return;
    }
    const index = PM.Layout.resolveDropIndex(dock.id, fromIndex, dropTarget.dockId, dropTarget.index);
    if (index == null) {
      PM.toast(`${PM.PANELS[spec.id]?.title || spec.id} is already here`);
      return;
    }
    const previous = panelRects();
    PM.WS.mutate((workspace: Workspace) => {
      removePanel(workspace, spec.id);
      insertPanel(workspace, spec, dropTarget!.dockId, index);
    });
    animatePanelLayout(previous);
    PM.toast(`Moved ${PM.PANELS[spec.id]?.title || spec.id} → ${dockLabel(dropTarget.dockId)}`);
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
      ghost.classList.add('on');
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
