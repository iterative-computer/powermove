import type { PMRegistry } from '../legacy/registry';
import { beginPanelDrag } from './drag';
import { openPanelMenu } from './menu';
import { dismissedMenu } from '../overlays/dismissal';
import { applyPanelSize, findPanel, panelMinHeight, setPanelCollapsed, type DockSpec, type PanelSpec, type Workspace } from './model';
import { movePreservingFocus, parkPanel, stagePanelMove } from './portal';

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

export function syncPanelManifest(PM: PMRegistry, id: string, spec: PanelSpec, dock: DockSpec): HTMLElement | null {
  const inst = PM.panelInst[id];
  if (!inst?.el) return null;
  inst.spec = spec;
  inst.dock = dock;
  inst.def = PM.PANELS[id] ?? inst.def;
  applyPanelSize(inst.el, spec, inst.def);
  inst.el.style.minHeight = `${panelMinHeight(spec, inst.def)}px`;
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
    headless ? 'headless' : ''
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
  applyPanelSize(element, spec, def);
  element.style.minHeight = `${panelMinHeight(spec, def)}px`;

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
      if (element && params.dock.id !== next.dock.id) stagePanelMove(params.spec.id, element);
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
