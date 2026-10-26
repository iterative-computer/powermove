import type { PMRegistry } from '../legacy/registry';
import { attachAreaCorners } from './area-drag';
import { beginPanelDrag } from './drag';
import { openPanelMenu } from './menu';
import { dismissedMenu } from '../overlays/dismissal';
import { applyPanelSize, findPanel, setPanelCollapsed, type DockSpec, type PanelSpec, type Workspace } from './model';
import { movePreservingFocus, parkPanel, stagePanelMove } from './portal';
import { panelFrameOf } from '../kernel/panel-frame';

function makeElement<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  if (className) element.className = className;
  return element;
}

function appendIcon(PM: PMRegistry, element: HTMLElement, name: string): void {
  const icon = PM.icon(name);
  if (icon instanceof Node) element.appendChild(icon);
}

/** Keep the layout-owned drag affordance in sync with a headless panel's live
 * definition. Extension HMR can change this option after the shell was built,
 * so this cannot be a create-once decision inside ensurePanel. */
export function syncPanelMoveHandle(PM: PMRegistry, id: string): HTMLButtonElement | null {
  const inst = PM.panelInst[id];
  if (!inst?.el || !inst.body || !inst.def) return null;
  let handle = inst.moveHandle instanceof HTMLButtonElement
    ? inst.moveHandle
    : inst.el.querySelector('.panel-move-handle') as HTMLButtonElement | null;
  const enabled = !!inst.def.headless && !inst.def.hideMoveHandle;
  if (!enabled) {
    handle?.remove();
    inst.moveHandle = null;
    return null;
  }

  if (!handle) {
    handle = makeElement('button', 'panel-move-handle');
    handle.type = 'button';
    appendIcon(PM, handle, 'grip');
    handle.addEventListener('pointerdown', (event: PointerEvent) => {
      if (event.button !== 0) return;
      const current = (PM.Layout.ws && findPanel(PM.Layout.ws as Workspace, id))
        || { spec: inst.spec, dock: inst.dock };
      if (!current?.dock || !current.spec) return;
      event.stopPropagation();
      beginPanelDrag(PM, event, current.spec, current.dock, inst.el);
    });
    handle.addEventListener('contextmenu', (event: MouseEvent) => {
      event.preventDefault();
      event.stopPropagation();
      const current = (PM.Layout.ws && findPanel(PM.Layout.ws as Workspace, id))
        || { spec: inst.spec, dock: inst.dock };
      if (!current?.dock || !current.spec) return;
      openPanelMenu(PM, current.spec, current.dock, handle!, {
        x: event.clientX,
        y: event.clientY
      });
    });
    inst.moveHandle = handle;
  }

  handle.title = `Move ${inst.def.title} · right-click for options`;
  handle.setAttribute('aria-label', `Move ${inst.def.title} panel`);
  const slot = inst.def.moveSlot ? inst.body.querySelector(inst.def.moveSlot) : null;
  handle.classList.toggle('inline', !!slot);
  if (slot) slot.insertBefore(handle, slot.firstChild);
  else inst.el.appendChild(handle);
  return handle;
}

/** Give a tab stack's shown panel a tab for each member in its header. The
 * other members stay mounted, hidden, so switching keeps their state. */
export function syncPanelTabs(PM: PMRegistry, specs: PanelSpec[], active: PanelSpec): void {
  for (const spec of specs) {
    const header = PM.panelInst[spec.id]?.header as HTMLElement | undefined;
    if (!header) continue;
    const tabbed = specs.length > 1 && spec === active;
    header.classList.toggle('tabbed', tabbed);
    let strip = header.querySelector(':scope > .ptabs');
    if (!tabbed) { strip?.remove(); continue; }
    if (!strip) {
      strip = makeElement('div', 'ptabs');
      strip.setAttribute('role', 'tablist');
      const title = header.querySelector(':scope > .ptitle');
      if (title) title.after(strip);
      else header.prepend(strip);
    }
    strip.replaceChildren(...specs.map((member) => {
      const tab = makeElement('button', member === active ? 'ptab on' : 'ptab');
      tab.type = 'button';
      tab.setAttribute('role', 'tab');
      tab.setAttribute('aria-selected', String(member === active));
      tab.dataset.tab = member.id;
      tab.textContent = member.title || PM.PANELS[member.id]?.title || member.id;
      // A panel may name what it shows (--panel-subtitle), beside its title.
      tab.append(makeElement('span', 'ptab-sub'));
      tab.addEventListener('click', (event) => {
        event.stopPropagation();
        if (member !== active) PM.Layout.activateTab?.(member.id);
      });
      return tab;
    }));
  }
}

export function syncPanelManifest(PM: PMRegistry, id: string, spec: PanelSpec, dock: DockSpec): HTMLElement | null {
  const inst = PM.panelInst[id];
  if (!inst?.el) return null;
  inst.spec = spec;
  inst.dock = dock;
  inst.def = PM.PANELS[id] ?? inst.def;
  applyPanelSize(inst.el, spec, inst.def, dock.id);
  inst.el.querySelector('.ptitle')?.replaceChildren(spec.title || inst.def.title);
  setPanelCollapsed(PM, id, !!findPanel(PM.Layout.ws as Workspace, id)?.spec.collapsed, false);
  return inst.el;
}

export function ensurePanel(PM: PMRegistry, spec: PanelSpec, dock: DockSpec): HTMLElement | null {
  const def = PM.PANELS[spec.id];
  if (!def) return null;
  const existing = PM.panelInst[spec.id];
  if (existing?.el && existing.built) return syncPanelManifest(PM, spec.id, spec, dock);

  const inst = existing || (PM.panelInst[spec.id] = { def, cache: null });
  // Workspace normalization strips unsupported presentation fields, so panel
  // chrome is controlled only by the registered definition.
  const headless = !!def.headless;
  const element = makeElement('div', [
    'panel',
    def.flush ? 'flush' : '',
    def.noscroll ? 'noscroll' : '',
    headless ? 'headless' : '',
    // A sandboxed extension's view iframe fills the body; it scrolls itself.
    panelFrameOf(def) ? 'frame' : ''
  ].filter(Boolean).join(' '));
  element.id = `panel-${spec.id}`;
  element.dataset.panel = spec.id;

  const header = makeElement('header');
  const grip = makeElement('span', 'grip');
  grip.title = 'Drag to move · right-click for options';
  appendIcon(PM, grip, 'grip');
  const title = makeElement('span', 'ptitle');
  title.textContent = spec.title || def.title;
  const spacer = makeElement('span', 'sp');
  const options = makeElement('button', 'panel-options');
  options.type = 'button';
  options.title = `${def.title} options`;
  options.setAttribute('aria-label', `${def.title} panel options`);
  appendIcon(PM, options, 'more');
  header.append(grip, title, spacer, options);
  element.appendChild(header);

  const body = makeElement('div', 'body');
  element.appendChild(body);

  Object.assign(inst, { el: element, body, header, def, spec, dock, moveHandle: null, cache: body });
  applyPanelSize(element, spec, def, dock.id);

  body.textContent = '';
  try {
    def.build?.(body, inst);
  } catch (error) {
    console.error(`[panel]${spec.id}`, error);
    const empty = makeElement('div', 'empty');
    empty.textContent = `Panel error: ${error instanceof Error ? error.message : String(error)}`;
    body.appendChild(empty);
  }
  inst.built = true;

  try {
    def.header?.(header, inst);
  } catch {
    // Header decoration is optional and must not prevent the panel from mounting.
  }
  if (!header.querySelector('.grip')) header.insertBefore(grip, header.firstChild);

  const liveLocation = () =>
    (PM.Layout.ws && findPanel(PM.Layout.ws as Workspace, spec.id)) || { spec: inst.spec || spec, dock: inst.dock || dock };
  const beginMove = (event: PointerEvent): void => {
    if (event.button !== 0) return;
    const current = liveLocation();
    if (!current?.dock || !current.spec) return;
    event.stopPropagation();
    beginPanelDrag(PM, event, current.spec, current.dock, element);
  };
  header.addEventListener('pointerdown', (event) => {
    if (dismissedMenu(event)) return;
    if ((event.target as Element | null)?.closest('button')) return;
    beginMove(event);
  });
  header.addEventListener('dblclick', (event) => {
    if ((event.target as Element | null)?.closest('button')) return;
    setPanelCollapsed(PM, spec.id, element.dataset.collapsed !== '1');
  });
  const showMenu = (event: MouseEvent): void => {
    event.preventDefault();
    event.stopPropagation();
    const current = liveLocation();
    if (!current?.dock || !current.spec) return;
    openPanelMenu(PM, current.spec, current.dock, header, {
      x: event.clientX,
      y: event.clientY
    });
  };
  options.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    const current = liveLocation();
    if (!current?.dock || !current.spec) return;
    const rect = options.getBoundingClientRect();
    openPanelMenu(PM, current.spec, current.dock, options, {
      x: rect.left,
      y: rect.bottom
    });
  });
  header.addEventListener('contextmenu', showMenu);
  syncPanelMoveHandle(PM, spec.id);
  attachAreaCorners(PM, element, spec.id);
  if (spec.collapsed) setPanelCollapsed(PM, spec.id, true, false);
  return element;
}

export type PoolHostParams = { PM: PMRegistry; spec: PanelSpec; dock: DockSpec };

export function panelPoolHost(node: HTMLElement, initial: PoolHostParams) {
  let params = initial;
  const mount = (): void => {
    const element = ensurePanel(params.PM, params.spec, params.dock);
    if (element && (!element.isConnected || element.parentElement?.id === 'pm-panel-pool')) {
      movePreservingFocus(element, node);
    }
  };
  mount();
  return {
    update(next: PoolHostParams) {
      const element = params.PM.panelInst[params.spec.id]?.el as HTMLElement | undefined;
      // A new slot may already own the live panel by the time the pool updates.
      // Only stage it while it still belongs to the old dock; parking it after
      // the destination attached leaves that slot blank until another layout.
      if (element && params.dock.id !== next.dock.id
        && element.closest('.dock')?.id === `dock-${params.dock.id}`) {
        stagePanelMove(params.spec.id, element);
      }
      params = next;
      syncPanelManifest(params.PM, params.spec.id, params.spec, params.dock);
      mount();
    },
    destroy() {
      const element = params.PM.panelInst[params.spec.id]?.el as HTMLElement | undefined;
      if (element?.parentElement === node) parkPanel(params.spec.id, element);
    }
  };
}
