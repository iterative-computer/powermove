import { describe, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainEvent } from 'electron';
import { ENGINE_IPC } from '../../shared/engine-ipc';
import { ExportJobWarmth, installExportJobWarmthIpc } from './export-job-warmth';

function setup() {
  const releases: Array<ReturnType<typeof vi.fn>> = [];
  const engine = {
    holdJobUtility: vi.fn(() => { const release = vi.fn(); releases.push(release); return release; }),
    ensureReady: vi.fn(async (_role: 'job') => {})
  };
  const log = vi.fn();
  return { warmth: new ExportJobWarmth(engine, log), engine, releases, log };
}

describe('export job utility warmth', () => {
  it('keeps stacked settings sheets independent within the same window', () => {
    const { warmth, releases } = setup();
    warmth.setSheetOpen(1, 'first', true);
    warmth.setSheetOpen(1, 'second', true);
    warmth.setSheetOpen(1, 'first', false);
    expect(releases[0]).toHaveBeenCalledOnce();
    expect(releases[1]).not.toHaveBeenCalled();
    warmth.setSheetOpen(1, 'first', false);
    warmth.setSheetOpen(1, 'second', false);
    expect(releases[1]).toHaveBeenCalledOnce();
  });

  it('prewarms once per sheet, independently holds a save panel and releases on all outcomes', async () => {
    const { warmth, engine, releases } = setup();
    warmth.setSheetOpen(1, 'sheet', true);
    warmth.setSheetOpen(1, 'sheet', true);
    expect(engine.ensureReady).toHaveBeenCalledExactlyOnceWith('job');
    let finish!: () => void;
    const save = warmth.during(1, () => new Promise<void>(resolve => { finish = resolve; }));
    expect(engine.ensureReady).toHaveBeenCalledTimes(2);
    warmth.setSheetOpen(1, 'sheet', false);
    expect(releases[0]).toHaveBeenCalledOnce();
    expect(releases[1]).not.toHaveBeenCalled();
    finish();
    await save;
    expect(releases[1]).toHaveBeenCalledOnce();
    await expect(warmth.during(1, async () => { throw new Error('panel failed'); })).rejects.toThrow('panel failed');
    expect(releases[2]).toHaveBeenCalledOnce();
    expect(await warmth.during(1, async () => ({ status: 'cancelled' }))).toEqual({ status: 'cancelled' });
    expect(releases[3]).toHaveBeenCalledOnce();
  });

  it('drops only the lost window holds and makes late panel completion harmless', () => {
    const { warmth, releases } = setup();
    warmth.setSheetOpen(1, 'sheet', true);
    const closePanel = warmth.hold(1);
    warmth.setSheetOpen(2, 'sheet', true);
    warmth.forget(1);
    expect(releases[0]).toHaveBeenCalledOnce();
    expect(releases[1]).toHaveBeenCalledOnce();
    expect(releases[2]).not.toHaveBeenCalled();
    closePanel();
    warmth.setSheetOpen(1, 'sheet', false);
    warmth.dispose();
    warmth.dispose();
    expect(releases.every(release => release.mock.calls.length === 1)).toBe(true);
  });

  it('contains prewarm failure while retaining the UI hold until close', async () => {
    const { warmth, engine, releases, log } = setup();
    engine.ensureReady.mockRejectedValueOnce(new Error('addon unavailable'));
    warmth.setSheetOpen(1, 'sheet', true);
    await vi.waitFor(() => expect(log).toHaveBeenCalledWith('warn', expect.stringContaining('addon unavailable')));
    expect(releases[0]).not.toHaveBeenCalled();
    warmth.setSheetOpen(1, 'sheet', false);
    expect(releases[0]).toHaveBeenCalledOnce();
  });

  it('accepts boolean sheet state only from trusted senders and unregisters on disposal', () => {
    const { warmth, engine, releases } = setup();
    const handlers = new Map<string, (event: IpcMainEvent, sheet: unknown, open: unknown) => void>();
    const ipc = {
      on: (channel: string, fn: (event: IpcMainEvent, sheet: unknown, open: unknown) => void) => handlers.set(channel, fn),
      removeAllListeners: (channel: string) => handlers.delete(channel)
    } as unknown as IpcMain;
    const trusted = { sender: { id: 1 } } as IpcMainEvent;
    const remove = installExportJobWarmthIpc(ipc, warmth, event => event === trusted);
    const receive = handlers.get(ENGINE_IPC.exportSheet)!;
    receive({ sender: { id: 2 } } as IpcMainEvent, 'sheet', true);
    receive(trusted, 'sheet', 'true');
    receive(trusted, '', true);
    receive(trusted, 'x'.repeat(129), true);
    expect(engine.ensureReady).not.toHaveBeenCalled();
    receive(trusted, 'sheet', true);
    expect(engine.ensureReady).toHaveBeenCalledOnce();
    remove();
    expect(releases[0]).toHaveBeenCalledOnce();
    expect(handlers.has(ENGINE_IPC.exportSheet)).toBe(false);
  });
});
