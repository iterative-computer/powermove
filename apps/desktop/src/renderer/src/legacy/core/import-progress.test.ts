import { expect, it, vi } from 'vitest';
import { createImportProgress } from './import-progress';
it('uses one keyed progress toast and keeps its completed state after cleanup', () => {
  const PM = { toast: vi.fn(), dismissToast: vi.fn() };
  const operation = createImportProgress('Importing image sequence', PM as any);
  const key = PM.toast.mock.calls[0]![2].key;
  operation.update({ label: 'Creating sequence', completed: 1, total: 77 });
  expect(PM.toast).toHaveBeenLastCalledWith('Creating sequence', 2200, expect.objectContaining({ key, progress: 1 / 77, sticky: true }));
  operation.finish('Imported sequence');
  operation.close();
  expect(PM.toast).toHaveBeenLastCalledWith('Imported sequence', 3400, expect.objectContaining({ key, completed: true, progress: 1 }));
  expect(PM.dismissToast).not.toHaveBeenCalled();
  const failed = createImportProgress('Importing file', PM as any);
  failed.close();
  expect(PM.dismissToast).toHaveBeenCalledOnce();
});

it('shows honest indeterminate stages and keeps cancellation visible until cleanup', () => {
  const PM = { toast: vi.fn(), dismissToast: vi.fn() }, cancel = vi.fn();
  const operation = createImportProgress('Importing file', PM as any, cancel);
  operation.update({ label: 'Decoding audio' });
  expect(PM.toast.mock.lastCall).toMatchObject(['Decoding audio', 2200, { progress: null }]);
  const action = PM.toast.mock.lastCall![2].action;
  expect(action.dismiss).toBe(false);
  action.run(); action.run();
  expect(cancel).toHaveBeenCalledOnce();
  operation.update({ label: 'Saving media', completed: 1, total: 2 });
  expect(PM.toast.mock.lastCall).toMatchObject(['Cancelling import…', 2200, { progress: null }]);
  expect(PM.toast.mock.lastCall![2].action).toBeUndefined();
  operation.finish('Import cancelled'); operation.close();
  expect(PM.dismissToast).not.toHaveBeenCalled();
});
