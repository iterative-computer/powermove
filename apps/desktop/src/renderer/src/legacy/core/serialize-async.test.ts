import { expect, it } from 'vitest';
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
