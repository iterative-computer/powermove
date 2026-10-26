import { afterEach, expect, it, vi } from 'vitest';
import { AutosaveQueue } from './autosave-queue';
afterEach(() => vi.useRealTimers());
const pending = () => { let resolve!: (ok: boolean) => void; const promise = new Promise<boolean>(r => { resolve = r; }); return { promise, resolve }; };
it('debounces without serializing on edit and acknowledges only the revision actually written', async () => {
  vi.useFakeTimers(); const first = pending(), second = pending();
  const write = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const queue = new AutosaveQueue(write, vi.fn());
  queue.edit('one'); queue.edit('one'); expect(write).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(750); expect(write).toHaveBeenCalledOnce();
  queue.edit('one'); first.resolve(true); await vi.advanceTimersByTimeAsync(0);
  expect(queue.state('one')).toMatchObject({ revision: 3, persisted: 2, phase: 'pending' });
  await vi.advanceTimersByTimeAsync(750); second.resolve(true); await vi.advanceTimersByTimeAsync(0);
  expect(queue.state('one')).toMatchObject({ revision: 3, persisted: 3, phase: 'saved' });
});
it('bounds debounce during sustained completed edits', async () => {
  vi.useFakeTimers(); const write = vi.fn(async () => true); const queue = new AutosaveQueue(write, vi.fn());
  queue.edit('one'); for (let i = 0; i < 10; i++) { await vi.advanceTimersByTimeAsync(499); queue.edit('one'); }
  expect(write).not.toHaveBeenCalled(); await vi.advanceTimersByTimeAsync(10); expect(write).toHaveBeenCalledOnce();
});
it('serializes two independent projects while preserving both newer revisions', async () => {
  vi.useFakeTimers(); const first = pending(); const calls: string[] = [];
  const queue = new AutosaveQueue(async id => { calls.push(id); return calls.length === 1 ? first.promise : true; }, vi.fn());
  queue.edit('heavy-a'); queue.edit('heavy-b'); await vi.advanceTimersByTimeAsync(750);
  expect(calls).toEqual(['heavy-a']); queue.edit('heavy-a'); queue.edit('heavy-b');
  first.resolve(true); await vi.advanceTimersByTimeAsync(0); expect(calls).toEqual(['heavy-a','heavy-b']);
  await vi.advanceTimersByTimeAsync(750); expect(calls).toEqual(['heavy-a','heavy-b','heavy-a']);
  expect(queue.state('heavy-a')?.persisted).toBe(2); expect(queue.state('heavy-b')?.persisted).toBe(2);
});
it('waits for a drag transaction to finish and then writes one completed edit', async () => {
  vi.useFakeTimers(); let dragging = true; const write = vi.fn(async () => true);
  const queue = new AutosaveQueue(write, vi.fn(), () => !dragging);
  queue.edit('one'); await vi.advanceTimersByTimeAsync(6000); expect(write).not.toHaveBeenCalled();
  dragging = false; await vi.advanceTimersByTimeAsync(100); expect(write).toHaveBeenCalledOnce();
});
it('Cmd+S queues a fresh save after an active write and retries a failed acknowledgement', async () => {
  vi.useFakeTimers(); const first = pending(); const write = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  const queue = new AutosaveQueue(write, vi.fn()); queue.edit('one'); await vi.advanceTimersByTimeAsync(750);
  queue.edit('one'); const flush = queue.flush('one'); expect(write).toHaveBeenCalledOnce();
  first.resolve(true); expect(await flush).toBe(false); expect(queue.state('one')).toMatchObject({ persisted: 1, revision: 2, phase: 'failure' });
  expect(await queue.flush('one')).toBe(true); expect(queue.state('one')).toMatchObject({ persisted: 2, phase: 'saved' });
});
