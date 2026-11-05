import { mount, unmount, flushSync } from 'svelte';
import FigmaConnect from './FigmaConnect.svelte';
import type { FigmaLinkBridge } from '../../../../shared/figma-link';
import type { PMRegistry } from '../../legacy/registry';
const openSheets = new WeakMap<PMRegistry, { el: HTMLElement; close: () => void }>();
export function openFigmaConnect(PM: PMRegistry, link: FigmaLinkBridge): void {
  const existing = openSheets.get(PM);
  if (existing?.el.isConnected) {
    existing.el.focus({ preventScroll: true });
    return;
  }
  const body = document.createElement('div');
  body.style.cssText = 'display:flex;flex-direction:column;align-items:flex-start;gap:10px';
  let handle: any;
  const view = mount(FigmaConnect, {
    target: body,
    props: { link, api: PM.Kernel.api('figma-connect'), onDisconnect: () => handle.close() },
  });
  const stop = link.onState((state) => {
    view.update(state);
    flushSync();
  });
  handle = PM.modal({
    title: 'Connect Figma',
    body,
    width: 460,
    actions: [{ label: 'Done', pri: true }],
    onClose: () => {
      stop();
      void link.closeSetup();
      void unmount(view);
      openSheets.delete(PM);
    },
  });
  openSheets.set(PM, handle);
  flushSync();
  void view.start();
}
