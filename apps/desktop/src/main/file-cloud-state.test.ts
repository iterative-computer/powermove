import { afterEach, expect, it, vi } from 'vitest';
import { inspectFileCloudState, requireLocalMedia } from './file-cloud-state';
afterEach(() => vi.useRealTimers());

it.each(['icloud', 'cloud'] as const)('explains that %s media needs a download before conversion', async state => {
  const inspect = vi.fn().mockResolvedValue(state);
  await expect(requireLocalMedia('/synthetic.mov', inspect)).rejects.toThrow('Download it in Finder');
  expect(inspect).toHaveBeenCalledExactlyOnceWith('/synthetic.mov');
});

it.each(['local', 'missing', 'unknown'] as const)('leaves %s media to normal file checks', async state => {
  await expect(requireLocalMedia('/synthetic.mov', vi.fn().mockResolvedValue(state))).resolves.toBeUndefined();
});

it('bounds a provider metadata probe and handles a late rejection', async () => {
  vi.useFakeTimers();
  let fail!: (error: Error) => void;
  const probe = new Promise<never>((_, reject) => { fail = reject; });
  const checking = expect(inspectFileCloudState('/synthetic.mov', () => probe, 100)).rejects.toThrow('taking too long');
  await vi.advanceTimersByTimeAsync(100); await checking;
  fail(new Error('late provider failure')); await Promise.resolve();
  expect(vi.getTimerCount()).toBe(0);
});

it('clears the timeout after a successful metadata probe', async () => {
  vi.useFakeTimers();
  await expect(inspectFileCloudState('/synthetic.mov', async () => 'local')).resolves.toBe('local');
  expect(vi.getTimerCount()).toBe(0);
});
