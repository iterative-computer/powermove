/*
 * View iframes: the body of a sandboxed extension's panel.
 *
 * Message topology (the kernel only brokers):
 *
 *   kernel ──(view port)──────── view iframe      data, theme, size, keys, events
 *   kernel ──(runtime RPC)────── runtime iframe   mountPanel / unmountPanel
 *   runtime iframe ──(brokered)── view iframe      panel definition lookup
 *
 * On every `load` of the view document (first insert, `refresh`, and every
 * dock move, because Chromium reloads an iframe that is re-parented) the
 * kernel opens two fresh MessageChannels, hands one end of the brokered
 * channel to the runtime iframe and posts `init` with the other ends to the
 * view. Tearing a view down closes the kernel's port and tells the runtime to
 * close its end.
 */
import { createRpc, type Rpc, type RpcBudget } from '../../../shared/sandbox-rpc';
import type { SandboxInit, SandboxKey, SandboxKeyEvent, SandboxPanelInfo, SandboxViewInit } from '../../sandbox/shim-api';
import type { Disposable } from './api';

export interface ViewLink {
  rpc: Rpc;
  /** Event subscriptions this view made; released with the view. */
  registrations: Map<string, Disposable>;
}

export interface ViewHost {
  /** Document URL for a panel's view, or null to leave `src` unset (tests). */
  src(panelId: string): string | null;
  /** A fresh init snapshot (project mirror, vars, catalog, bundle URL). */
  snapshot(): SandboxInit;
  /** Theme as the host document currently shows it. */
  theme(): SandboxInit['theme'];
  /** The host's keybindings, reduced to what a view needs to filter keydowns. */
  keys(): SandboxKey[];
  budget: RpcBudget;
  forwardKey(payload: SandboxKeyEvent): void;
  outsideClick(): void;
  /** Live views, for mirror/theme/keys broadcasts. */
  links: Set<ViewLink>;
  connectRuntime(panelId: string, token: string, port: MessagePort): void;
  disconnectRuntime(token: string): void;
  /** Kernel handlers shared with the runtime iframe (invoke, events, log, errors). */
  handlers(link: ViewLink): Record<string, (...args: any[]) => unknown>;
  report(error: Error): void;
  /** A sandbox check listens for each view's outcome instead of the error policy. */
  state?(panelId: string, state: 'ready' | 'error', message?: string): void;
  /** Test seam: deliver `init` without a real document. */
  post?(frame: HTMLIFrameElement, message: SandboxViewInit & { t: 'init' }, ports: MessagePort[]): void;
}

function sizeOf(body: HTMLElement): { width: number; height: number } {
  return { width: body.clientWidth, height: body.clientHeight };
}

export function mountSandboxView(host: ViewHost, panel: SandboxPanelInfo, body: HTMLElement, inst: { spec: Record<string, unknown> }): { dispose(): void } {
  if (host.links.size >= 50) {
    const message = document.createElement('p');
    message.textContent = 'This extension has reached its open panel limit.';
    body.append(message);
    return { dispose: () => message.remove() };
  }
  const frame = document.createElement('iframe');
  frame.className = 'ext-panel-frame';
  frame.setAttribute('sandbox', 'allow-scripts');
  frame.title = panel.title;
  frame.dataset.view = panel.id;
  let live: { link: ViewLink; token: string } | null = null;

  const disconnect = (): void => {
    if (!live) return;
    const { link, token } = live;
    live = null;
    host.links.delete(link);
    try { link.rpc.notify('dispose'); } catch { /* already closed */ }
    // Let the queued disposal notification cross the port before closing it.
    queueMicrotask(() => link.rpc.close());
    for (const registration of link.registrations.values()) registration.dispose();
    link.registrations.clear();
    host.disconnectRuntime(token);
  };

  const connect = (): void => {
    disconnect();
    delete frame.dataset.state;
    frame.dataset.loads = String(Number(frame.dataset.loads ?? 0) + 1); // each load is a fresh view document
    const toView = new MessageChannel();
    const brokered = new MessageChannel();
    const token = crypto.randomUUID();
    const link = { registrations: new Map<string, Disposable>() } as ViewLink;
    link.rpc = createRpc(toView.port1, {
      ...host.handlers(link),
      /* The kernel accepts only this extension's bindings. Port messages are
         never converted into DOM keyboard events. */
      key(payload: SandboxKeyEvent) {
        if (frame.ownerDocument.activeElement !== frame || typeof payload?.key !== 'string') return;
        host.forwardKey(payload);
      },
      /* Outside-click dismissal for menus and popovers open in the app. */
      pointer(payload: { button?: unknown; x?: unknown; y?: unknown }) {
        if (frame.ownerDocument.activeElement === frame && payload && payload.button === 0) host.outsideClick();
      },
      mounted() { frame.dataset.state = 'ready'; host.state?.(panel.id, 'ready'); },
      'view-error'(error: { message?: unknown }) {
        frame.dataset.state = 'error';
        const message = typeof error?.message === 'string' ? error.message : 'failed to mount';
        if (host.state) host.state(panel.id, 'error', message);
        else host.report(new Error(`Panel "${panel.id}": ${message}`));
      }
    }, 10_000, { budget: host.budget, onSustainedLimit: () => host.report(new Error('Sandbox RPC rate exceeded 200 messages per second for 3 seconds')) });
    host.links.add(link);
    live = { link, token };
    host.connectRuntime(panel.id, token, brokered.port2);
    const message: SandboxViewInit & { t: 'init' } = {
      t: 'init', ...host.snapshot(), theme: host.theme(), mode: 'view',
      panelId: panel.id, spec: JSON.parse(JSON.stringify(inst?.spec ?? {})) as Record<string, unknown>,
      keys: host.keys(), size: sizeOf(body), ...(panel.noscroll ? { noscroll: true } : {})
    };
    const ports = [toView.port2, brokered.port1];
    if (host.post) host.post(frame, message, ports);
    else frame.contentWindow?.postMessage(message, '*', ports);
  };

  frame.addEventListener('load', connect);
  const observer = typeof ResizeObserver === 'function'
    ? new ResizeObserver(() => { try { live?.link.rpc.notify('size', sizeOf(body)); } catch { /* closing */ } })
    : null;
  observer?.observe(body);
  const removalObserver = typeof MutationObserver === 'function' ? new MutationObserver(() => {
    if (!frame.isConnected) disconnect();
  }) : null;
  removalObserver?.observe(body, { childList: true });
  const src = host.src(panel.id);
  if (src) frame.src = src;
  body.append(frame);

  return {
    dispose() {
      observer?.disconnect();
      removalObserver?.disconnect();
      frame.removeEventListener('load', connect);
      disconnect();
      frame.remove();
    }
  };
}
