import { afterEach, expect, it, vi } from 'vitest';
import { parseAsync } from './parse-async';
import { stringifyAsync } from './serialize-async';
import { yieldTurn } from './yield-task';

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('loads and saves large documents when background timers cannot run', async () => {
  // Older Electron versions use MessageChannel; newer ones use scheduler.yield.
  vi.stubGlobal('scheduler', undefined);
  const timeout = globalThis.setTimeout;
  vi.spyOn(globalThis, 'setTimeout').mockImplementation(((callback: any, delay?: number, ...args: any[]) => {
    if (!delay) throw new Error('Background timer is suspended');
    return timeout(callback, delay, ...args);
  }) as typeof setTimeout);
  const document = Array.from({ length: 30_000 }, (_, index) => ({ index, values: [1, 2, 3], name: `Layer ${index}` }));
  const source = await stringifyAsync(document);
  expect(source).toBe(JSON.stringify(document));
  expect(await parseAsync(source)).toEqual(document);
});

it('takes a turn behind waiting tasks even where scheduler.yield would resume ahead of them', async () => {
  const prioritized = vi.fn(() => Promise.resolve());
  vi.stubGlobal('scheduler', { yield: prioritized });
  const order: string[] = [];
  const waiting = new MessageChannel();
  waiting.port1.onmessage = () => { order.push('waiting task'); waiting.port1.close(); };
  waiting.port2.postMessage(null);
  await yieldTurn();
  order.push('resumed');
  expect(prioritized).not.toHaveBeenCalled();
  expect(order).toEqual(['waiting task', 'resumed']);
});
