import { afterEach, expect, it, vi } from 'vitest';
import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
import { createForkUpdater } from './update-fork';

const sandbox = vi.hoisted(() => vi.fn(async () => ({ ok: true, skipped: false })));
vi.mock('../store/sandbox-check', () => ({ checkInSandbox: sandbox }));
afterEach(() => { resetBridgeForTests(); sandbox.mockReset(); sandbox.mockResolvedValue({ ok: true, skipped: false }); });

function harness() {
  const updated = { kind: 'updated' as const, version: '2.0.0', projectId: 'fork-updates', changeSetId: 'update-1' };
  const updateFork = vi.fn(async (_request: { id: string }): Promise<any> => updated);
  const restoreChangeSet = vi.fn(async () => undefined);
  installBridgeForTests({ codex: { updateFork, restoreChangeSet } } as never);
  const record = { id: 'custom-tool', manifest: { name: 'My Tool' }, health: { state: 'ok' } };
  const reload = vi.fn(async () => undefined);
  const PM = { toast: vi.fn(), dismissToast: vi.fn(), Kernel: { loader: { reload, whenIdle: async () => {}, records: () => [record] } } };
  const startAgent = vi.fn(async () => undefined);
  return { updated, updateFork, restoreChangeSet, record, reload, PM, startAgent, update: createForkUpdater(PM, startAgent) };
}

it('completes a straightforward update without starting an agent and offers Undo', async () => {
  const h = harness();
  await h.update('custom-tool');
  expect(h.updateFork).toHaveBeenCalledExactlyOnceWith({ id: 'custom-tool' });
  expect(h.startAgent).not.toHaveBeenCalled();
  expect(h.reload).toHaveBeenCalledWith('custom-tool');
  const [message, , options] = h.PM.toast.mock.calls.at(-1)!;
  expect(message).toContain('customizations were kept');
  expect(options.action.label).toBe('Undo');
  options.action.run();
  options.action.run();
  await vi.waitFor(() => expect(h.restoreChangeSet).toHaveBeenCalledExactlyOnceWith({ projectId: 'fork-updates', changeSetId: 'update-1' }));
  await vi.waitFor(() => expect(h.PM.toast).toHaveBeenCalledWith('My Tool update undone.'));
});

it.each(['conflict', 'validation'])('offers agent help for %s only after the user chooses it', async reason => {
  const h = harness();
  h.updateFork.mockResolvedValue({ kind: 'needs-agent', reason, conflicts: ['index.ts'] });
  await h.update('custom-tool');
  expect(h.startAgent).not.toHaveBeenCalled();
  expect(h.reload).not.toHaveBeenCalled();
  const [message, , options] = h.PM.toast.mock.calls.at(-1)!;
  expect(message).toContain('current version was kept');
  expect(options.action.label).toBe('Update with agent');
  options.action.run();
  expect(h.startAgent).toHaveBeenCalledExactlyOnceWith('custom-tool');
});

it('restores the previous version when the updated extension fails to load', async () => {
  const h = harness();
  h.record.health.state = 'activation-error';
  await h.update('custom-tool');
  expect(h.restoreChangeSet).toHaveBeenCalledOnce();
  expect(h.startAgent).not.toHaveBeenCalled();
  expect(h.PM.toast.mock.calls.at(-1)![0]).toContain('previous version was restored');
});

it('checks sandbox compatibility and rolls back a failing result', async () => {
  const h = harness();
  Object.assign(h.PM.Kernel, { deps: {} });
  sandbox.mockResolvedValue({ ok: false, skipped: false });
  await h.update('custom-tool');
  expect(sandbox).toHaveBeenCalledWith(h.PM, 'custom-tool');
  expect(h.restoreChangeSet).toHaveBeenCalledOnce();
  expect(h.PM.toast.mock.calls.at(-1)![2].action.label).toBe('Update with agent');
});

it('serializes multiple updates and ignores duplicate clicks', async () => {
  const h = harness();
  let release!: () => void;
  h.updateFork.mockImplementationOnce(async () => {
    await new Promise<void>(resolve => { release = resolve; });
    return h.updated;
  });
  const first = h.update('custom-tool');
  const duplicate = h.update('custom-tool');
  const second = h.update('other-tool');
  await vi.waitFor(() => expect(h.updateFork).toHaveBeenCalledTimes(1));
  release();
  await Promise.all([first, duplicate, second]);
  expect(h.updateFork.mock.calls.map(([request]) => request.id)).toEqual(['custom-tool', 'other-tool']);
});

it('reports failed Undo without claiming success', async () => {
  const h = harness();
  await h.update('custom-tool');
  h.restoreChangeSet.mockRejectedValue(new Error('Newer extension changes exist.'));
  h.PM.toast.mock.calls.at(-1)![2].action.run();
  await vi.waitFor(() => expect(h.PM.toast).toHaveBeenCalledWith('Newer extension changes exist.'));
  expect(h.PM.toast).not.toHaveBeenCalledWith('My Tool update undone.');
});
