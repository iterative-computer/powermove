import { afterEach, expect, it, vi } from 'vitest';
import { createImportProgress } from './import-progress';

function frames() {
  let next = 1;
  const pending = new Map<number, FrameRequestCallback>();
  const request = vi.fn((callback: FrameRequestCallback) => { const id = next++; pending.set(id, callback); return id; });
  const cancel = vi.fn((id: number) => { pending.delete(id); });
  vi.stubGlobal('requestAnimationFrame', request);
  vi.stubGlobal('cancelAnimationFrame', cancel);
  return { request, cancel, pending, flush() {
    const callbacks = [...pending.values()]; pending.clear();
    for (const callback of callbacks) callback(0);
  } };
}

afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it('coalesces transient progress to the latest value once per animation frame', () => {
  const raf = frames();
  const PM = { toast: vi.fn(), dismissToast: vi.fn() };
  const operation = createImportProgress('Importing image sequence', PM as any);
  const key = PM.toast.mock.calls[0]![2].key;
  expect(PM.toast).toHaveBeenCalledOnce();
  operation.update({ label: 'Reading', completed: 1, total: 77 });
  operation.update({ label: 'Creating sequence', completed: 40, total: 77 });
  expect(raf.request).toHaveBeenCalledOnce();
  expect(PM.toast).toHaveBeenCalledOnce();
  raf.flush();
  expect(PM.toast).toHaveBeenCalledTimes(2);
  expect(PM.toast).toHaveBeenLastCalledWith('Importing image sequence', 2200,
    expect.objectContaining({ key, progress: 40 / 77, sticky: true }));
  operation.update({ label: 'Done', completed: 77, total: 77 });
  operation.finish('Imported sequence');
  operation.close();
  raf.flush();
  expect(raf.cancel).toHaveBeenCalledOnce();
  expect(PM.toast).toHaveBeenCalledTimes(3);
  expect(PM.toast).toHaveBeenLastCalledWith('Imported sequence', 3400,
    expect.objectContaining({ key, completed: true, progress: 1 }));
  expect(PM.dismissToast).not.toHaveBeenCalled();
});

it('cancels pending progress and dismisses only an unfinished import', () => {
  const raf = frames();
  const PM = { toast: vi.fn(), dismissToast: vi.fn() };
  const operation = createImportProgress('Importing file', PM as any);
  const key = PM.toast.mock.calls[0]![2].key;
  operation.update({ label: 'Reading', completed: 1, total: 2 });
  operation.close(); operation.close();
  operation.update({ label: 'Too late', completed: 2, total: 2 });
  operation.finish('Too late');
  raf.flush();
  expect(PM.toast).toHaveBeenCalledOnce();
  expect(PM.dismissToast).toHaveBeenCalledExactlyOnceWith(key);
  expect(raf.pending.size).toBe(0);
});
