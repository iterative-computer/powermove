import { expect, test } from 'bun:test';
import { FakeR2 } from './r2';
test('FakeR2 conditional put models absence and existence', async () => {
  const bucket = new FakeR2();
  const first = await bucket.put('key', 'body', { onlyIf: { etagDoesNotMatch: '*' } });
  expect(first).not.toBeNull();
  expect(await bucket.put('key', 'other', { onlyIf: { etagDoesNotMatch: '*' } })).toBeNull();
  expect(await (await bucket.get('key'))?.text()).toBe('body');
  expect(await bucket.put('other', 'body', { onlyIf: { etagDoesNotMatch: 'random', uploadedBefore: new Date(0) } })).not
    .toBeNull();
  expect((await bucket.list()).objects.map((x) => x.key)).toEqual(['key', 'other']);
  await bucket.delete('key');
  expect(await bucket.head('key')).toBeNull();
});
