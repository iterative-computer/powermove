import type { PMRegistry } from '../legacy/registry';

const DOCK_ONLY = new Set(['viewer', 'timeline', 'toolbar']);

type PopoutEntry = {
  window: Window;
  element: HTMLElement;
  poolHost: HTMLElement | null;
  closing: boolean;
};

function copyAppearance(source: Document, target: Document): void {
  for (const node of source.head.querySelectorAll('link[rel="stylesheet"], style')) {
    target.head.appendChild(node.cloneNode(true));
  }
  target.documentElement.dataset.density = source.documentElement.dataset.density;
  target.documentElement.dataset.theme = source.documentElement.dataset.theme;
  target.documentElement.style.cssText = source.documentElement.style.cssText;
}

export function canPopoutPanel(id: string): boolean {
  return !!id && !DOCK_ONLY.has(id);
}

/**
 * Detaches a live panel into an Electron-owned child window. The panel DOM node
 * itself moves between same-origin documents, so Svelte state, inputs, event
 * handlers and undoable editor actions remain authoritative rather than being
 * recreated in a second app instance.
 */
export function installPanelPopouts(PM: PMRegistry): void {
  if (PM.Popout?.__sveltePopouts) return;
  const entries = new Map<string, PopoutEntry>();

  const isOpen = (id: string): boolean => {
    const entry = entries.get(id);
    if (!entry) return false;
    try {
      return !entry.window.closed;
    } catch {
      return false;
    }
  };

  const returnToLayout = (id: string, closeWindow: boolean): boolean => {
    const entry = entries.get(id);
    if (!entry) return false;
    entry.closing = true;
    entries.delete(id);
    entry.element.classList.remove('popped');
    const host = entry.poolHost?.isConnected
      ? entry.poolHost
      : document.querySelector<HTMLElement>(`#pm-panel-pool [data-panel-host="${CSS.escape(id)}"]`);
    host?.appendChild(entry.element);
    PM.Layout?.apply?.(PM.Layout.ws ?? PM.WS?.current);
    if (closeWindow) {
      try {
        if (!entry.window.closed) entry.window.close();
      } catch {
        // The panel has already returned; a closing OS window needs no retry.
      }
    }
    return true;
  };

  const open = (id: string): boolean => {
    PM.closeMenus?.();
    const definition = PM.PANELS?.[id];
    if (!definition) return false;
    if (!canPopoutPanel(id)) {
      PM.toast?.(`${definition.title} stays in the layout`);
      return false;
    }
    const existing = entries.get(id);
    if (existing && isOpen(id)) {
      existing.window.focus();
      return true;
    }
    const element = PM.panelInst?.[id]?.el as HTMLElement | undefined;
    if (!element) {
      PM.toast?.('Panel is not ready yet');
      return false;
    }
    const safeName = id.replace(/[^A-Za-z0-9_-]/g, '-').slice(0, 80);
    const child = window.open('', `pm-panel-${safeName}`, 'popup,width=480,height=620');
    if (!child) {
      PM.toast?.('Could not open the panel window');
      return false;
    }
    const poolHost = document.querySelector<HTMLElement>(
      `#pm-panel-pool [data-panel-host="${CSS.escape(id)}"]`
    );
    const entry: PopoutEntry = { window: child, element, poolHost, closing: false };
    entries.set(id, entry);

    try {
      const childDocument = child.document;
      childDocument.head.replaceChildren();
      childDocument.body.replaceChildren();
      copyAppearance(document, childDocument);
      childDocument.title = `${definition.title} — Powermove`;
      childDocument.body.className = 'panel-popout-body';

      const bar = childDocument.createElement('header');
      bar.className = 'pop-bar';
      const title = childDocument.createElement('span');
      title.className = 'pop-title';
      title.textContent = definition.title;
      const spacer = childDocument.createElement('span');
      spacer.className = 'sp';
      const dock = childDocument.createElement('button');
      dock.type = 'button';
      dock.className = 'btn';
      dock.textContent = 'Return to layout';
      dock.setAttribute('aria-label', `Return ${definition.title} to the Powermove layout`);
      dock.addEventListener('click', () => returnToLayout(id, true));
      bar.append(title, spacer, dock);

      const host = childDocument.createElement('main');
      host.className = 'pop-mirror';
      childDocument.body.append(bar, host);
      element.classList.add('popped');
      host.appendChild(element);
      child.addEventListener('beforeunload', () => {
        const current = entries.get(id);
        if (current && !current.closing) returnToLayout(id, false);
      }, { once: true });
      PM.Layout?.apply?.(PM.Layout.ws ?? PM.WS?.current);
      PM.toast?.(`${definition.title} opened in a window`);
      return true;
    } catch (error) {
      entries.delete(id);
      try { child.close(); } catch { /* already gone */ }
      PM.toast?.('Could not open the panel window');
      console.error('[panel popout]', error);
      return false;
    }
  };

  PM.Popout = {
    __sveltePopouts: true,
    open,
    dock: (id: string) => returnToLayout(id, true),
    reclaim: (id: string) => returnToLayout(id, false),
    isOpen,
    openIds: () => [...entries.keys()].filter(isOpen),
    closeAll: () => [...entries.keys()].forEach((id) => returnToLayout(id, true))
  };
}
