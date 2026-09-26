/* The values sheet for one extension, opened from Settings › Extensions and
   the Mods panel. Store 1.0 has per-extension values only. */
import { flushSync, mount, unmount } from 'svelte';

import type { ExtensionRecord } from '../../../shared/extensions';
import type { VarsBridge } from '../../../shared/vars-ipc';
import SetupSheet from './SetupSheet.svelte';
import { bridge } from '../kernel/bridge';

export { usefulHint } from './hint';

type PMLike = Record<string, any>;

let sheet: { id: string; close(): void } | null = null;

export function hasVars(record: ExtensionRecord | null | undefined): boolean {
  return !!record && record.scope === 'user' && (record.manifest?.vars?.length ?? 0) > 0;
}

function varsBridge(): VarsBridge | null {
  const candidate = bridge()?.vars;
  return candidate && typeof candidate.status === 'function' ? candidate : null;
}

/** Open the values sheet for `record`. One at a time; another extension replaces it. */
export function openSetup(PM: PMLike, record: ExtensionRecord): void {
  if (!PM?.modal || !hasVars(record)) return;
  if (sheet?.id === record.id) return;
  sheet?.close();
  const body = document.createElement('div');
  let component: ReturnType<typeof mount> | null = null;
  const handle = PM.modal({
    body,
    width: 440,
    actions: [],
    onClose: () => {
      if (component) void unmount(component);
      component = null;
      if (sheet === current) sheet = null;
    }
  });
  handle.el?.classList?.add('account-modal', 'vars-modal');
  const current = { id: record.id, close: () => handle.close() };
  sheet = current;
  component = mount(SetupSheet, {
    target: body,
    props: { PM, record, bridge: varsBridge(), onclose: () => handle.close() }
  });
  flushSync();
}

export function installVars(PM: PMLike): void {
  PM.Vars = {
    openSetup: (record: ExtensionRecord) => openSetup(PM, record)
  };
}
