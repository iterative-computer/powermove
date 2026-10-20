import { expect, it, vi } from 'vitest';
import { stringifyAsync } from './serialize-async';

it('matches JSON for project data, shared values, escaped text and sparse arrays', async () => {
  const shared = { value: 'quote"\n🟢', kf: [null, undefined, NaN, -0, 3] };
  const value = { a: shared, b: shared, absent: undefined, list: new Array(3) };
  expect(await stringifyAsync(value)).toBe(JSON.stringify(value));
});
it('yields while saving a massive project and rejects circular data', async () => {
  const value = Array.from({ length: 20_000 }, (_, id) => ({ id, p: { x: { v: id, kf: [] } } }));
  let yields = 0;
  expect(await stringifyAsync(value, async () => { yields++; })).toBe(JSON.stringify(value));
  expect(yields).toBeGreaterThan(0);
  const circular: any = {}; circular.self = circular;
  await expect(stringifyAsync(circular)).rejects.toThrow('circular');
});
it('preserves escaped characters and surrogate pairs across long string slices', async () => {
  const value = { text: 'x'.repeat(16_383) + '🟢' + '\n"\\\ud800'.repeat(20_000) };
  expect(await stringifyAsync(value)).toBe(JSON.stringify(value));
});

it('preserves small leaf values and arrays without invoking custom project serialization', async () => {
  const values = [null, false, true, 0, -0, NaN, Infinity, undefined, 'quote"\n🟢', '\ud800'];
  const leaf = { values, absent: undefined, ignored: () => 'ignored', symbol: Symbol('ignored') };
  const project = { leaves: values.map(value => ({ value, empty: [], handles: [0, .33, 1], missing: undefined })), leaf };
  expect(await stringifyAsync(project)).toBe(JSON.stringify(project));
  const custom = { value: 1, toJSON: () => ({ changed: true }) };
  expect(await stringifyAsync(custom)).toBe('{"value":1}');
});

it('omits function fields without invoking their custom JSON hooks', async () => {
  const hook = vi.fn(() => ({ changed: true }));
  const callback = Object.assign(() => undefined, { toJSON: hook });
  expect(await stringifyAsync({ value: 1, callback })).toBe('{"value":1}');
  expect(hook).not.toHaveBeenCalled();
  // Scalar functions in arrays and at the root kept native JSON behavior.
  expect(await stringifyAsync([callback])).toBe('[{"changed":true}]');
  expect(await stringifyAsync(callback)).toBe('{"changed":true}');
  expect(hook.mock.calls).toEqual([[''], ['']]);
});

it('keeps large leaf-heavy checkpoints interruptible between bounded slices', async () => {
  const project = Array.from({ length: 100_000 }, (_, i) => ({ i, t: i / 30, v: i % 100, kf: [], handles: [0, 1] }));
  const superseded = new Error('Snapshot superseded');
  let yields = 0;
  await expect(stringifyAsync(project, async () => {
    if (++yields === 2) throw superseded;
  })).rejects.toBe(superseded);
  expect(yields).toBe(2);
  expect(await stringifyAsync(project, async () => {})).toBe(JSON.stringify(project));
});

it('does not expand a huge track before giving Cancel a chance to run', async () => {
  let reads = 0, clock = 0;
  const keys = new Proxy(Array.from({ length: 20_000 }, (_, i) => ({ t: i, v: i })), {
    get(target, key, receiver) { if (typeof key === 'string' && /^\d+$/.test(key)) reads++; return Reflect.get(target, key, receiver); },
  });
  const now = vi.spyOn(performance, 'now').mockImplementation(() => clock += 5);
  try {
    const cancelled = new Error('Cancel Close');
    await expect(stringifyAsync(keys, async () => { throw cancelled; })).rejects.toBe(cancelled);
    expect(reads).toBeLessThan(64);
  } finally { now.mockRestore(); }
});
