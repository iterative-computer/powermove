import { mount, unmount } from 'svelte';
import ColorField from './ColorField.svelte';
import FillField from './FillField.svelte';
import type { EditBinding } from './gesture';
import type { PowermoveAPI } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';

type LegacyControl = { connected: boolean; after: number; remove(): void };
const legacyControls = new Map<HTMLElement, LegacyControl>();
let lifecycleObserver: MutationObserver | null = null;
let earlierMutations: MutationRecord[] = [], cleanupQueued = false;

function cleanRemovedControls(records: MutationRecord[]): void {
  const mutations = earlierMutations.concat(records);
  earlierMutations = [];
  const pending: Array<[HTMLElement, LegacyControl]> = [];
  for (const [target, control] of legacyControls) {
    if (target.isConnected) control.connected = true;
    else if (!control.connected) pending.push([target, control]);
  }
  if (pending.length) {
    // Mutation records describe historical parent changes. Current containment
    // cannot prove a field was connected: it may have entered an already
    // detached subtree, which remains observed until this notification.
    const parents = new Map<Node, Node | null>();
    const wasConnected = (target: Node) => {
      const seen = new Set<Node>();
      for (let node: Node | null = target; node && !seen.has(node);) {
        if (node.nodeType === Node.DOCUMENT_NODE) return true;
        seen.add(node);
        node = parents.has(node) ? parents.get(node)! : node.parentNode;
      }
      return false;
    };
    for (let i = mutations.length - 1; i >= 0; i--) {
      const record = mutations[i]!;
      for (const node of record.addedNodes) parents.set(node, null);
      for (const node of record.removedNodes) parents.set(node, record.target);
      for (const [target, control] of pending) {
        if (!control.connected && i >= control.after && wasConnected(target)) control.connected = true;
      }
    }
  }
  for (const [target, control] of legacyControls) {
    control.after = 0;
    if (control.connected && !target.isConnected) {
      legacyControls.delete(target);
      try { control.remove(); }
      catch (error) { console.error('[legacy control] cleanup failed', error); }
    }
  }
  if (!legacyControls.size) { lifecycleObserver?.disconnect(); lifecycleObserver = null; }
}

function watchLegacyControl(target: HTMLElement, remove: () => void): void {
  if (!lifecycleObserver) {
    lifecycleObserver = new MutationObserver(cleanRemovedControls);
    lifecycleObserver.observe(document.body, { childList: true, subtree: true });
  } else {
    // Existing records may describe a detached container before this control
    // was created. Retain them for older fields, but exclude them for this one.
    earlierMutations.push(...lifecycleObserver.takeRecords());
    if (earlierMutations.length && !cleanupQueued) {
      cleanupQueued = true;
      queueMicrotask(() => {
        cleanupQueued = false;
        if (lifecycleObserver) cleanRemovedControls(lifecycleObserver.takeRecords());
      });
    }
  }
  legacyControls.set(target, { connected: target.isConnected, after: earlierMutations.length, remove });
}

/** Keep old DOM-based callers on the same controls as the Svelte inspector. */
export function legacyColorField(PM: PMRegistry, fill: boolean, get: () => unknown, set: (value: unknown) => void, opt: any = {}) {
  const target = document.createElement('div') as HTMLDivElement & { sync(): void };
  target.style.width = '100%';
  let revision = $state(0);
  target.sync = () => { revision++; };
  const label = opt.label || (fill ? 'Fill' : 'Color');
  // Legacy getters can read plain draft state (for example New project's
  // background), so a write must also notify the mounted Svelte control.
  const write = (value: unknown): void => { set(value); revision++; };
  const edit: EditBinding = opt.local ? { mode: 'local', label, set: write }
    : opt.command ? { mode: 'command', label, origin: opt.origin, command: opt.command }
    : { mode: 'set', label, set: write };
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
  watchLegacyControl(target, () => { void unmount(component); });
  return target;
}
