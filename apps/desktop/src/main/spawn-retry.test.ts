import { describe, expect, it, vi } from 'vitest';
import { EventEmitter } from 'node:events';
import type { ChildProcess } from 'node:child_process';
import { spawnWithRetry } from './spawn-retry';

function child(outcome: 'spawn' | 'deadlock' | 'other'): ChildProcess {
  const process = new EventEmitter() as ChildProcess;
  queueMicrotask(() => process.emit(outcome === 'spawn' ? 'spawn' : 'error',
    outcome === 'deadlock' ? Object.assign(new Error('spawn Unknown system error -11'), { errno: -11, code: 'Unknown system error -11' }) : new Error('permission denied')));
  return process;
}

describe('spawnWithRetry', () => {
  it('retries a macOS deadlock before the child starts', async () => {
    const spawn = vi.fn().mockImplementationOnce(() => child('deadlock')).mockImplementationOnce(() => child('spawn'));
    await expect(spawnWithRetry('/bin/echo', [], {}, 'workspace command', spawn)).resolves.toBeDefined();
    expect(spawn).toHaveBeenCalledTimes(2);
  });

  it('reports the operation after bounded retries', async () => {
    const spawn = vi.fn(() => child('deadlock'));
    await expect(spawnWithRetry('/bin/echo', [], {}, 'workspace command', spawn)).rejects.toThrow('Could not start workspace command (echo): spawn Unknown system error -11');
    expect(spawn).toHaveBeenCalledTimes(3);
  });

  it('does not retry other launch errors', async () => {
    const spawn = vi.fn(() => child('other'));
    await expect(spawnWithRetry('/bin/echo', [], {}, 'workspace command', spawn)).rejects.toThrow('permission denied');
    expect(spawn).toHaveBeenCalledTimes(1);
  });

  it('does not replay an operation after its child starts', async () => {
    const spawn = vi.fn(() => {
      const process = new EventEmitter() as ChildProcess;
      queueMicrotask(() => {
        process.emit('spawn');
        setTimeout(() => process.emit('error', new Error('encoder failed')), 0);
      });
      return process;
    });
    const process = await spawnWithRetry('/bin/echo', [], {}, 'image sequence encoder', spawn);
    await new Promise<void>(resolve => process.once('error', () => resolve()));
    expect(spawn).toHaveBeenCalledTimes(1);
  });
});
