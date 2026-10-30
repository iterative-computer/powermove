import type { PMRegistry } from '../legacy/registry';
import { findPanel, stackOf, type Workspace } from './model';

/* After Effects' "Maximize frame" (the ` key): the panel under the pointer
   fills the whole workspace area until the key is pressed again. It is a view
   state only. Nothing is saved, and the rest of the layout stays mounted
   underneath, so every other panel keeps its scroll and size. */

let maximized: string | null = null;
let pointer = { x: -1, y: -1 };
let installed = false;

const body = (): HTMLElement | null => document.getElementById('body');

export const maximizedPanel = (): string | null => maximized;

/** The shown panel of the maximized stack: switching tabs keeps it maximized. */
function shownElement(PM: PMRegistry): HTMLElement | null {
  if (!maximized) return null;
  const ws = PM.Layout?.ws as Workspace | null;
  if (!ws || !findPanel(ws, maximized)) return null;
  const stack = stackOf(ws, maximized);
  const shown = stack.find((spec) => spec.active) ?? stack[0];
  const element = shown && PM.panelInst[shown.id]?.el as HTMLElement | undefined;
  return element?.isConnected && element.closest('#body .dock') ? element : null;
}

function paint(PM: PMRegistry): void {
  const host = body();
  document.querySelectorAll<HTMLElement>('.panel.maximized').forEach((node) => {
    node.classList.remove('maximized');
    for (const key of ['left', 'top', 'width', 'height'] as const) node.style.removeProperty(key);
  });
  const element = shownElement(PM);
  if (!element || !host) {
    maximized = null;
    host?.classList.remove('has-maximized');
    return;
  }
  const rect = host.getBoundingClientRect();
  const inset = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--gap')) || 8;
  element.classList.add('maximized');
  element.style.left = `${rect.left + inset}px`;
  element.style.top = `${rect.top + inset}px`;
  element.style.width = `${Math.max(0, rect.width - inset * 2)}px`;
  element.style.height = `${Math.max(0, rect.height - inset * 2)}px`;
  host.classList.add('has-maximized');
}

function settle(PM: PMRegistry): void {
  paint(PM);
  // Canvases (viewer, timeline) measure on this event, as after a splitter drag.
  PM.bus.emit('layout:applied');
  window.requestAnimationFrame(() => PM.bus.emit('layout:applied'));
}

/** Maximize a panel, or restore one that is maximized. Without an id, uses the
 * panel under the pointer. */
export function toggleMaximizedPanel(PM: PMRegistry, id?: string): boolean {
  if (maximized && (!id || id === maximized || stackOf(PM.Layout.ws as Workspace, maximized).some((spec) => spec.id === id))) {
    maximized = null;
    settle(PM);
    return true;
  }
  const target = id ?? document.elementFromPoint(pointer.x, pointer.y)?.closest<HTMLElement>('#body .dock .panel[data-panel]')?.dataset.panel;
  if (!target || !PM.Layout.ws || !findPanel(PM.Layout.ws as Workspace, target)) return false;
  maximized = target;
  settle(PM);
  return !!maximized;
}

export function installMaximize(PM: PMRegistry): void {
  if (installed) return;
  installed = true;
  window.addEventListener('pointermove', (event) => { pointer = { x: event.clientX, y: event.clientY }; }, { capture: true, passive: true });
  window.addEventListener('resize', () => { if (maximized) paint(PM); });
  // A new layout may have moved, tabbed or closed the panel.
  PM.bus.on('layout', () => { if (maximized) window.requestAnimationFrame(() => paint(PM)); });
}
