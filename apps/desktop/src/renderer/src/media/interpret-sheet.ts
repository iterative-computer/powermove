/* Opens Interpret Footage for one asset (appendix D, the asset right-click
   menu): a Settings-style sheet whose rows show each setting with its
   origin; Apply writes the asset's `interpret` member, and the display size
   and duration it implies, as one Undo step. The job engine reads the file
   (`window.powermove.nativeEngine.probeMedia`), so the sheet needs the new
   engine. */
import { flushSync, mount, unmount } from 'svelte';

import type { EngineBridge } from '../../../shared/engine-ipc';
import type { MediaFacts } from '../../../shared/engine-protocol';
import { bridge } from '../kernel/bridge';
import InterpretSheet from './InterpretSheet.svelte';
import { applyChange, assetChange, type Draft } from './interpret';

type PMLike = Record<string, any>;

let open: { asset: string; close(): void } | null = null;

function engine(): EngineBridge | null {
  const candidate = (bridge() as { nativeEngine?: EngineBridge } | null)?.nativeEngine;
  return candidate && typeof candidate.probeMedia === 'function' ? candidate : null;
}

/** Whether the asset can be interpreted here: video and still media, with
 *  the new engine's bridge present. */
export function canInterpret(asset: { kind?: unknown } | null | undefined): boolean {
  return !!engine() && (asset?.kind === 'video' || asset?.kind === 'image');
}

export function openInterpretSheet(PM: PMLike, assetId: string): void {
  const asset = PM.proj?.assets?.[assetId];
  const host = engine();
  if (!PM?.modal || !asset || !host) return;
  if (open?.asset === assetId) return;
  open?.close();
  const body = document.createElement('div');
  let component: ReturnType<typeof mount> | null = null;
  const handle = PM.modal({
    body,
    width: 520,
    actions: [],
    onClose: () => {
      if (component) void unmount(component);
      component = null;
      if (open === current) open = null;
    }
  });
  handle.el?.classList?.add('account-modal', 'interp-modal');
  const current = { asset: assetId, close: () => handle.close() };
  open = current;
  component = mount(InterpretSheet, {
    target: body,
    props: {
      name: String(asset.name ?? 'Media'),
      initial: asset.interpret ?? null,
      probe: (interpret: Record<string, unknown> | null) => host.probeMedia({ asset: assetId, interpret }),
      onapply: (draft: Draft, facts: MediaFacts) => {
        const record = PM.proj?.assets?.[assetId];
        if (record) {
          const change = assetChange(draft, facts, record);
          PM.hist.do('Interpret Footage', () => applyChange(record, change));
          PM.bus?.emit?.('assets');
          PM.toast?.(`Interpreted ${record.name ?? 'media'}`, 2200, { kind: 'status' });
        }
        handle.close();
      },
      onclose: () => handle.close()
    }
  });
  flushSync();
}
