import { mount, unmount } from 'svelte';
import ColorField from './ColorField.svelte';
import FillField from './FillField.svelte';
import type { EditBinding } from './gesture';
import type { PowermoveAPI } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';

/** Keep old DOM-based callers on the same controls as the Svelte inspector. */
export function legacyColorField(PM: PMRegistry, fill: boolean, get: () => unknown, set: (value: unknown) => void, opt: any = {}) {
  const target = document.createElement('div') as HTMLDivElement & { sync(): void };
  target.style.width = '100%';
  let revision = $state(0);
  target.sync = () => { revision++; };
  const label = opt.label || (fill ? 'Fill' : 'Color');
  const edit: EditBinding = opt.local ? { mode: 'local', label, set }
    : opt.command ? { mode: 'command', label, origin: opt.origin, command: opt.command }
    : { mode: 'set', label, set };
  const api = {
    edit: { begin: (...args: any[]) => PM.Edit.begin(...args), dispatch: (command: any) => PM.Edit.dispatch(command), commit: (name: string) => PM.Edit.commit(name), cancel: () => PM.Edit.cancel(), apply: (...args: any[]) => PM.Edit.apply(...args) },
    history: { begin: (name: string) => PM.hist.begin(name), commit: (name: string) => PM.hist.commit(name), cancel: () => PM.hist.cancel(), do: (...args: any[]) => PM.hist.do(...args) },
    transport: { invalidate: (reason: string) => PM.invalidate(reason) },
    ui: { drag: (...args: any[]) => PM.drag(...args), toast: (text: string) => PM.toast(text) },
    util: { uid: (prefix: string) => PM.uid(prefix) },
    model: { normalizeFill: (...args: any[]) => PM.normalizeFill(...args) }
  } as unknown as PowermoveAPI;
  const component = mount(fill ? FillField : ColorField, { target, props: {
    api, get: () => { void revision; return get(); }, edit, label, ...(fill ? { fallback: opt.fallback } : {})
  } });
  // Legacy surfaces detach their DOM rather than calling Svelte unmount.
  let connected = false;
  const observer = new MutationObserver(() => {
    if (target.isConnected) connected = true;
    else if (connected) { observer.disconnect(); void unmount(component); }
  });
  observer.observe(document.body, { childList: true, subtree: true });
  return target;
}
