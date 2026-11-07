import { EventEmitter } from 'node:events';
import type { ChildProcess } from 'node:child_process';
import { afterEach, expect, it, vi } from 'vitest';
import { watchCompletedExit } from '../../e2e/helpers/completed-exit';

afterEach(() => vi.useRealTimers());
function fixture() {
  vi.useFakeTimers();
  const child = Object.assign(new EventEmitter(), { stderr: new EventEmitter(), exitCode: null as number | null,
    signalCode: null, kill: vi.fn() });
  const reaped = vi.fn();
  const stop = watchCompletedExit(child as unknown as ChildProcess, 'private-completed-exit', reaped);
  return { child, reaped, stop };
}
it('never reaps an editor that has not completed successful exit', async () => {
  const f = fixture();
  f.child.stderr.emit('data', 'Close cancelled; save failed');
  await vi.advanceTimersByTimeAsync(120_000);
  expect(f.child.kill).not.toHaveBeenCalled(); expect(f.reaped).not.toHaveBeenCalled();
  f.stop();
});
it('reaps only a process still alive after its completed-exit marker and grace period', async () => {
  const f = fixture();
  f.child.stderr.emit('data', 'private-completed-'); f.child.stderr.emit('data', 'exit' + '\nNative teardown'.repeat(100));
  await vi.advanceTimersByTimeAsync(2999); expect(f.child.kill).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(f.child.kill).toHaveBeenCalledExactlyOnceWith('SIGKILL'); expect(f.reaped).toHaveBeenCalledOnce();
  expect(f.child.stderr.listenerCount('data')).toBe(0);
});
it('leaves a normally exited process alone and disposes its observer', async () => {
  const f = fixture();
  f.child.stderr.emit('data', 'private-completed-exit');
  f.child.exitCode = 0; f.child.emit('exit', 0);
  await vi.advanceTimersByTimeAsync(3000);
  expect(f.child.kill).not.toHaveBeenCalled(); expect(f.child.stderr.listenerCount('data')).toBe(0);
});
