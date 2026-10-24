import type { IpcMain, IpcMainEvent } from 'electron';
import { ENGINE_IPC } from '../../shared/engine-ipc';
import type { NativeEngine } from './native-engine';
import type { Log } from './connection';

/** User export UI owns warm-job leases, independent of accepted render jobs.
 *  Sheet closure must not release a still-open save panel's lease. */
export class ExportJobWarmth {
  private sheets = new Map<number, Map<string, () => void>>();
  private holds = new Map<number, Set<() => void>>();

  constructor(private engine: Pick<NativeEngine, 'ensureReady' | 'holdJobUtility'>, private log: Log) {}

  setSheetOpen(owner: number, sheet: string, open: boolean): void {
    if (open) {
      const sheets = this.sheets.get(owner) ?? new Map<string, () => void>();
      this.sheets.set(owner, sheets);
      if (!sheets.has(sheet)) sheets.set(sheet, this.hold(owner));
    } else {
      const sheets = this.sheets.get(owner);
      sheets?.get(sheet)?.();
      sheets?.delete(sheet);
      if (!sheets?.size) this.sheets.delete(owner);
    }
  }

  /** Also used around the offline prompt/save panel through job acceptance. */
  hold(owner: number): () => void {
    const releaseEngine = this.engine.holdJobUtility();
    const holds = this.holds.get(owner) ?? new Set<() => void>();
    this.holds.set(owner, holds);
    const release = (): void => {
      if (!holds.delete(release)) return;
      releaseEngine();
      if (!holds.size && this.holds.get(owner) === holds) this.holds.delete(owner);
    };
    holds.add(release);
    void this.engine.ensureReady('job').catch((error: unknown) => {
      this.log('warn', `the job engine did not start ahead of an export: ${error instanceof Error ? error.message : String(error)}`);
    });
    return release;
  }

  async during<T>(owner: number, action: () => Promise<T>): Promise<T> {
    const release = this.hold(owner);
    try { return await action(); }
    finally { release(); }
  }

  forget(owner: number): void {
    this.sheets.delete(owner);
    for (const release of this.holds.get(owner) ?? []) release();
  }

  dispose(): void {
    for (const owner of this.holds.keys()) this.forget(owner);
  }
}

export function installExportJobWarmthIpc(ipc: IpcMain, warmth: ExportJobWarmth, trusted: (event: IpcMainEvent) => boolean): () => void {
  ipc.on(ENGINE_IPC.exportSheet, (event, sheet: unknown, open: unknown) => {
    if (trusted(event) && typeof sheet === 'string' && sheet.length > 0 && sheet.length <= 128 && typeof open === 'boolean') {
      warmth.setSheetOpen(event.sender.id, sheet, open);
    }
  });
  return () => {
    ipc.removeAllListeners(ENGINE_IPC.exportSheet);
    warmth.dispose();
  };
}
