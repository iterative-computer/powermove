import { expect, it } from 'vitest';
import { modResultForMessage } from './mod-result';
it('renders typed results even after removal and keeps failures explicit', () => {
  for (const status of ['ready', 'error', 'removed'] as const) {
    const modResult = { id: 'pexels', name: 'Pexels Browser', action: 'created' as const, status };
    expect(modResultForMessage({ role: 'assistant', modResult }, [])).toEqual(modResult);
    expect(modResultForMessage({ role: 'user', modResult }, [])).toBeNull();
  }
});
it('upgrades only exact legacy mod notices backed by an installed record', () => {
  const records = [{ id: 'pexels', manifest: { name: 'Pexels Browser' } }];
  expect(modResultForMessage({ role: 'assistant', text: 'Added mod Pexels Browser' }, records))
    .toEqual({ id: 'pexels', name: 'Pexels Browser', action: 'created', status: 'ready' });
  for (const text of ['Added mod Missing', 'I added mod Pexels Browser', 'Added mod Pexels Browser\nMore text']) {
    expect(modResultForMessage({ role: 'assistant', text }, records)).toBeNull();
  }
});
