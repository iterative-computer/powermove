import { afterEach, describe, expect, it, vi } from 'vitest';
import { AgentThreadMonitor } from './agent-thread-monitor';
import type { CodexRunRequest } from '../shared/ipc';

const request = { id: 'run', threadId: 'thread', projectId: 'project', provider: 'claude', model: 'claude-sonnet-5-5', reasoningEffort: 'medium' } as CodexRunRequest;
afterEach(() => vi.useRealTimers());
describe('thread run monitor', () => {
  it('resolves an announced run once it starts, and keeps its terminal result immutable', async () => {
    const monitor = new AgentThreadMonitor();
    const queued = monitor.reserve('project', 'thread', 'run');
    expect(monitor.begin(request)).toBe(queued);
    monitor.progress(queued, 'Working');
    monitor.finish(queued, { ok: true, text: 'Completed', access: 'project' });
    monitor.finish(queued, { ok: false, error: 'Later cancellation', cancelled: true });
    monitor.progress(queued, 'Later work');
    await expect(queued.done).resolves.toMatchObject({ status: 'completed', summary: 'Completed', providerInstanceId: 'claude', reasoningEffort: 'medium' });
    expect(queued.state.progress).toBe('Working');
    expect(() => monitor.begin(request)).toThrow('already finished');
    expect(() => monitor.get('other-project', 'thread', 'run')).toThrow('another thread');
  });

  it('retains a dependency until every watcher releases it, preventing cycles', () => {
    const monitor = new AgentThreadMonitor();
    const one = monitor.watch('a', 'b'), two = monitor.watch('a', 'b');
    one(); expect(() => monitor.watch('b', 'a')).toThrow('cycle');
    two(); const opposite = monitor.watch('b', 'a'); opposite();
    expect(() => monitor.watch('a', 'a')).toThrow('cycle');
  });

  it('does not leave a watch pending forever when the announced run never starts', async () => {
    vi.useFakeTimers();
    const monitor = new AgentThreadMonitor();
    const queued = monitor.reserve('project', 'thread', 'run');
    await vi.advanceTimersByTimeAsync(30_000);
    await expect(queued.done).resolves.toMatchObject({ status: 'failed', summary: expect.stringContaining('did not start') });
    const next = monitor.reserve('project', 'thread', 'next-run');
    monitor.shutdown(); await expect(next.done).resolves.toMatchObject({ status: 'cancelled' });
  });
});
