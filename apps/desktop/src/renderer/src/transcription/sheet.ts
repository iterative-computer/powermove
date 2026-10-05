/* Opens the download sheet: the model picker in a Settings-style sheet,
   shown when something needs transcription and no model is ready. One at a
   time; a second request while it is up joins it (and updates the reason). */
import { flushSync, mount, unmount } from 'svelte';

import type { TranscriptionStatus } from '../../../shared/transcription';
import { transcriptionBridge } from './host';
import ModelSheet from './ModelSheet.svelte';

type PMLike = Record<string, any>;

let host: PMLike | null = null;
let open: { promise: Promise<boolean>; setReason(reason: string): void } | null = null;

export function setSheetHost(PM: PMLike): void {
  host = PM;
}

export function isModelSheetOpen(): boolean {
  return !!open;
}

export const DEFAULT_REASON = 'Transcription turns speech into text on this Mac. Choose a model to download.';

export function openModelSheet(reason: string, status: TranscriptionStatus | null = null): Promise<boolean> {
  const text = String(reason || '').trim().slice(0, 300) || DEFAULT_REASON;
  if (open) {
    open.setReason(text);
    return open.promise;
  }
  const PM = host ?? (globalThis as { PM?: PMLike }).PM ?? null;
  const bridge = transcriptionBridge();
  if (!PM?.modal || !bridge) return Promise.resolve(false);
  let settle!: (ready: boolean) => void;
  const promise = new Promise<boolean>((resolve) => { settle = resolve; });
  const body = document.createElement('div');
  type SheetInstance = { setReason(reason: string): void };
  let component: SheetInstance | null = null;
  let result = false;
  let handle: { el?: HTMLElement; close(): void } | null = null;
  const current = {
    promise,
    setReason: (next: string) => component?.setReason(next)
  };
  open = current;
  handle = PM.modal({
    body,
    width: 520,
    actions: [],
    onClose: () => {
      if (component) void unmount(component as unknown as Record<string, unknown>);
      component = null;
      if (open === current) open = null;
      settle(result);
    }
  });
  handle?.el?.classList?.add('account-modal', 'transcription-modal');
  component = mount(ModelSheet, {
    target: body,
    props: {
      host: bridge,
      initial: status,
      reason: text,
      onfinish: (ready: boolean) => {
        result = ready;
        handle?.close();
      }
    }
  }) as unknown as SheetInstance;
  flushSync();
  return promise;
}
