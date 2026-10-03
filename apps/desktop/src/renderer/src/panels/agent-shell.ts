import { beginPanelDrag } from '../layout/drag';
import { ensurePanel } from '../layout/panel';
import { movePreservingFocus, parkPanel } from '../layout/portal';
import './agent-shell.css';

type PM = Record<string, any>;

const EDGE = 12;
const GAP = 6;
const WIDTH = 440;
const HEIGHT = 680;
const COLLAPSE_MS = 280;
const LAUNCHER = '#agent-launcher';
let activeShell: { destroy(): void } | null = null;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max));
}

function reducedMotion(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** One host for the registered agent panel. The panel's Svelte instance is
 * moved, never rebuilt, so drafts, threads and active runs survive closing the
 * popover, switching projects and layout changes. Dragging the launcher onto
 * the workspace docks the same panel; the popover then stands aside. */
export function installAgentShell(PM: PM) {
  activeShell?.destroy();
  let open = false;
  let collapseTimer: ReturnType<typeof setTimeout> | undefined;

  const root = document.createElement('div');
  root.id = 'agent-popover-root';
  root.dataset.open = 'false';
  const surface = document.createElement('section');
  surface.id = 'agent-popover';
  surface.className = 'agent-popover';
  surface.setAttribute('role', 'dialog');
  surface.setAttribute('aria-label', 'Powermove agent');
  surface.inert = true;
  surface.setAttribute('aria-hidden', 'true');
  const panelHost = document.createElement('div');
  panelHost.className = 'agent-popover-panel-host';
  surface.append(panelHost);
  root.append(surface);
  document.body.appendChild(root);
  const listeners = new AbortController();

  const launcher = (): HTMLElement | null => document.querySelector<HTMLElement>(LAUNCHER);

  function viewport() {
    return { width: window.visualViewport?.width ?? window.innerWidth, height: window.visualViewport?.height ?? window.innerHeight };
  }

  /** Centred under the launcher, kept inside the window. */
  function position(): void {
    const view = viewport();
    const anchor = launcher()?.getBoundingClientRect();
    const width = Math.max(1, Math.min(WIDTH, view.width - EDGE * 2));
    const top = (anchor ? anchor.bottom : 44) + GAP;
    const height = Math.max(1, Math.min(HEIGHT, view.height - top - EDGE));
    const centre = anchor ? anchor.left + anchor.width / 2 : view.width / 2;
    const left = clamp(centre - width / 2, EDGE, view.width - width - EDGE);
    Object.assign(surface.style, { left: `${left}px`, top: `${top}px`, width: `${width}px`, height: `${height}px` });
    // Open and collapse grow from, and fold into, the launcher.
    const originX = anchor ? clamp(anchor.left + anchor.width / 2 - left, 0, width) : width / 2;
    surface.style.transformOrigin = `${originX}px 0px`;
    if (anchor) {
      surface.style.setProperty('--agent-collapse-x', `${anchor.left + anchor.width / 2 - (left + width / 2)}px`);
      surface.style.setProperty('--agent-collapse-y', `${anchor.top + anchor.height / 2 - (top + height / 2)}px`);
      surface.style.setProperty('--agent-collapse-sx', String(anchor.width / width));
      surface.style.setProperty('--agent-collapse-sy', String(anchor.height / height));
    }
  }

  function currentPanel(): HTMLElement | null {
    return (PM.panelInst?.agent?.el as HTMLElement | undefined) ?? null;
  }

  function ensureAgentPanel(): HTMLElement | null {
    const existing = currentPanel();
    if (existing) return existing;
    if (!PM.PANELS?.agent || !PM.panelInst) return null;
    return ensurePanel(PM, { id: 'agent' }, { id: 'right', panels: [] });
  }

  /** Placed in a visible dock of the current workspace. */
  function docked(): boolean {
    const workspace = PM.WS?.current;
    if (!workspace || PM.ProjectsScreen?.isOpen) return false;
    const found = PM.Layout?.findPanel?.(workspace, 'agent');
    return !!found && !found.dock?.hidden;
  }

  /** The pool re-parks panels on every layout pass; the popover reclaims it
   * unless the panel is docked, where the layout owns it. */
  function attach(): void {
    if (listeners.signal.aborted) return;
    if (docked()) {
      const panel = currentPanel();
      if (panel) delete panel.dataset.agentPopover;
      if (open) { open = false; sync(); }
      return;
    }
    const panel = ensureAgentPanel();
    if (!panel || panel.parentElement === panelHost) return;
    panel.dataset.agentPopover = '1';
    movePreservingFocus(panel, panelHost);
  }

  function sync(): void {
    root.dataset.open = String(open);
    surface.inert = !open;
    surface.setAttribute('aria-hidden', String(!open));
    launcher()?.setAttribute('aria-expanded', String(open));
    if (open) position();
    PM.bus?.emit?.('agent:popover', open);
  }

  function focusComposer(): void {
    queueMicrotask(() => {
      const field = panelHost.querySelector<HTMLElement>('.agent-inline-prompt');
      (field ?? surface).focus({ preventScroll: true });
    });
  }

  /** Brings the docked conversation forward instead of opening the popover. */
  function reveal(): void {
    const panel = currentPanel();
    if (!panel) return;
    panel.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    panel.querySelector<HTMLElement>('.agent-inline-prompt')?.focus({ preventScroll: true });
    if (!reducedMotion()) panel.animate?.([{ filter: 'brightness(1.12)' }, { filter: 'none' }], { duration: 600, easing: 'ease-out' });
  }

  function show(): void {
    clearTimeout(collapseTimer);
    delete root.dataset.collapsing;
    if (docked()) { reveal(); return; }
    attach();
    if (open) { focusComposer(); return; }
    open = true;
    sync();
    focusComposer();
  }

  function close(options: { restoreFocus?: boolean } = {}): void {
    if (!open) return;
    const hadFocus = surface.contains(document.activeElement);
    open = false;
    sync();
    if (hadFocus && options.restoreFocus !== false) launcher()?.focus({ preventScroll: true });
    else if (hadFocus) (document.activeElement as HTMLElement | null)?.blur?.();
  }

  /** A sent message folds the conversation into the launcher, which then
   * carries the run's live status. Pulses the launcher when already closed. */
  function collapse(): void {
    PM.bus?.emit?.('agent:collapse');
    if (!open) return;
    if (reducedMotion()) { close({ restoreFocus: false }); return; }
    position();
    root.dataset.collapsing = 'true';
    close({ restoreFocus: false });
    clearTimeout(collapseTimer);
    collapseTimer = setTimeout(() => { delete root.dataset.collapsing; }, COLLAPSE_MS);
  }

  function toggle(): void {
    if (open) close();
    else show();
  }

  /** Dragging from the launcher places the agent in a dock, like any panel.
   * A press that never moves stays a click. */
  let dragged = false;
  function dragToDock(event: PointerEvent, source: HTMLElement): void {
    dragged = false;
    if (event.button !== 0 || docked() || PM.ProjectsScreen?.isOpen || !PM.WS?.current) return;
    const startX = event.clientX, startY = event.clientY;
    const watch = new AbortController();
    window.addEventListener('pointermove', (move) => {
      if (dragged || Math.hypot(move.clientX - startX, move.clientY - startY) < 5) return;
      dragged = true;
      if (open) close({ restoreFocus: false });
    }, { signal: watch.signal, capture: true });
    const stop = () => watch.abort();
    window.addEventListener('pointerup', stop, { signal: watch.signal, capture: true });
    window.addEventListener('pointercancel', stop, { signal: watch.signal, capture: true });
    beginPanelDrag(PM as never, event, { id: 'agent' }, { id: 'titlebar', panels: [] }, source);
  }

  /** True once after a launcher press that became a drag. */
  function consumeDrag(): boolean {
    const was = dragged;
    dragged = false;
    return was;
  }

  document.addEventListener('pointerdown', (event) => {
    if (!open) return;
    const target = event.target as Node | null;
    if (!target || surface.contains(target) || launcher()?.contains(target)) return;
    // Menus, pickers and dialogs opened from the conversation float outside it.
    if (target instanceof Element && target.closest('.pm-menu, .modal, #scrim, .fill-picker-layer, [role="listbox"], [role="menu"], [role="dialog"]')) return;
    close({ restoreFocus: false });
  }, { signal: listeners.signal, capture: true });
  /* Other keys flow on to the app keymap, which already leaves text fields and
     panel text alone (select all, cut and paste stay scoped to the agent). */
  root.addEventListener('keydown', (event) => {
    // Controls that use Escape themselves (the slash menu) prevent it first.
    if (event.key !== 'Escape' || !open || event.defaultPrevented) return;
    event.stopPropagation();
    event.preventDefault();
    close();
  }, { signal: listeners.signal });
  const onResize = () => { if (open) position(); };
  window.addEventListener('resize', onResize, { signal: listeners.signal });
  window.visualViewport?.addEventListener('resize', onResize, { signal: listeners.signal });
  const offLayout = PM.bus?.on?.('layout:applied', attach);
  const offProjects = PM.bus?.on?.('projects:screen', () => { if (open) position(); });
  sync();
  queueMicrotask(attach);

  const api = {
    open: show,
    /** App-wide conversations use the same popover. */
    openGlobal: show,
    close,
    /** Kept for older callers: closing the popover. */
    minimize: () => close(),
    collapse,
    toggle,
    attach,
    reveal,
    dragToDock,
    consumeDrag,
    isDocked: docked,
    isOpen: () => open,
    destroy: () => {
      listeners.abort();
      clearTimeout(collapseTimer);
      offLayout?.();
      offProjects?.();
      const panel = currentPanel();
      if (panel?.parentElement === panelHost) { delete panel.dataset.agentPopover; parkPanel('agent', panel); }
      root.remove();
      if (activeShell === api) activeShell = null;
      if (PM.AgentShell === api) delete PM.AgentShell;
    }
  };
  PM.AgentShell = api;
  activeShell = api;
  return api;
}
