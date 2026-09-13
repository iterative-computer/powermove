import type { PMRegistry } from '../legacy/registry';
import { applyPanelSize, findPanel, panelMinHeight, setPanelCollapsed, type DockSpec, type PanelSpec, type Workspace } from './model';

export type SelectionSnapshot = {
  active: HTMLElement | null;
  start: number | null;
  end: number | null;
  direction: 'forward' | 'backward' | 'none' | null;
};

const pendingDockFocus = new WeakMap<HTMLElement, SelectionSnapshot>();

export function snapshotFocus(): SelectionSnapshot {
  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const selectable = active as HTMLInputElement | HTMLTextAreaElement | null;
  return {
    active,
    start: typeof selectable?.selectionStart === 'number' ? selectable.selectionStart : null,
    end: typeof selectable?.selectionEnd === 'number' ? selectable.selectionEnd : null,
    direction: selectable?.selectionDirection ?? null
  };
}

export function restoreFocus(snapshot: SelectionSnapshot): void {
  const { active } = snapshot;
  if (!active?.isConnected || active.closest('#pm-panel-pool')) return;
  try {
    active.focus({ preventScroll: true });
  } catch {
    active.focus();
  }
  if (snapshot.start == null || snapshot.end == null || !('setSelectionRange' in active)) return;
  try {
    (active as HTMLInputElement | HTMLTextAreaElement).setSelectionRange(
      snapshot.start,
      snapshot.end,
      snapshot.direction ?? 'none'
    );
  } catch {
    // Input types without a text selection API deliberately ignore restoration.
  }
}

export function movePreservingFocus(element: HTMLElement, target: HTMLElement): void {
  if (element.parentElement === target) return;
  if (target.closest('#pm-panel-pool')) {
    pendingDockFocus.delete(element);
    releaseFocusBeforeParking(element);
    target.appendChild(element);
    return;
  }
  const snapshot = pendingDockFocus.get(element) ?? snapshotFocus();
  target.appendChild(element);
  pendingDockFocus.delete(element);
  restoreFocus(snapshot);
}

function releaseFocusBeforeParking(element: HTMLElement): void {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement) || !element.contains(active)) return;
  active.blur();
}

function parkTarget(id: string): HTMLElement | null {
  const hosts = document.querySelectorAll<HTMLElement>('#pm-panel-pool [data-panel-host]');
  for (const host of hosts) if (host.dataset.panelHost === id) return host;
  return document.getElementById('pm-panel-pool');
}

export function stagePanelMove(id: string, element: HTMLElement): void {
  const park = parkTarget(id);
  if (!park) return;
  const snapshot = snapshotFocus();
  if (snapshot.active && element.contains(snapshot.active)) pendingDockFocus.set(element, snapshot);
  releaseFocusBeforeParking(element);
  if (element.parentElement !== park) park.appendChild(element);
}

export type PanelSlotParams = {
  PM: PMRegistry;
  id: string;
  spec: PanelSpec;
  dock: DockSpec;
};

export function panelSlot(node: HTMLElement, initial: PanelSlotParams) {
  let params = initial;

  const attach = (): void => {
    const inst = params.PM.panelInst[params.id];
    if (!inst?.el) return;
    inst.spec = params.spec;
    inst.dock = params.dock;
    applyPanelSize(inst.el, params.spec, inst.def);
    inst.el.style.minHeight = `${panelMinHeight(params.spec, inst.def)}px`;
    inst.el.querySelector('.ptitle')?.replaceChildren(params.spec.title || inst.def.title);
    setPanelCollapsed(params.PM, params.id, !!findPanel(params.PM.Layout.ws as Workspace, params.id)?.spec.collapsed, false);
    movePreservingFocus(inst.el, node);
  };

  attach();
  return {
    update(next: PanelSlotParams) {
      params = next;
      attach();
    },
    destroy() {
      const element = params.PM.panelInst[params.id]?.el as HTMLElement | undefined;
      if (element?.parentElement !== node) return;
      const current = params.PM.Layout.ws && findPanel(params.PM.Layout.ws as Workspace, params.id);
      if (current && current.dock.id !== params.dock.id) stagePanelMove(params.id, element);
      else parkPanel(params.id, element);
    }
  };
}

export function parkPanel(id: string, element: HTMLElement): void {
  const park = parkTarget(id);
  if (!park) return;
  // A parked panel is aria-hidden. Focus must leave it instead of being restored
  // by the live dock-to-dock portal path.
  pendingDockFocus.delete(element);
  releaseFocusBeforeParking(element);
  if (element.parentElement !== park) park.appendChild(element);
}
