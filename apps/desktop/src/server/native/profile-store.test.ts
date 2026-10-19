import { expect, it } from 'vitest';
import { profileStore } from './profile-store';
import type { Store } from '../../main/storage';
const copy = (value: any) => JSON.parse(JSON.stringify(value));
function fixture(initial: Record<string, unknown> = {}) {
  const values = copy(initial), events: string[] = [];
  const store = {
    snapshot: () => copy(values),
    set: async (key: string, value: unknown) => { await new Promise(resolve => setTimeout(resolve, 2)); values[key] = copy(value); events.push(key); },
    delete: async (key: string) => { delete values[key]; }, flushAll: async () => { events.push('flushed'); }
  } as unknown as Store;
  return { service: profileStore(store), values, events };
}
it('serializes concurrent creations from stale worker library views', async () => {
  const { service, values, events } = fixture();
  await Promise.all([service.write('projects', [{ id: 'A', name: 'A' }], []), service.write('projects', [{ id: 'B', name: 'B' }], [])]);
  expect(values.projects.map((p: any) => p.id)).toEqual(['B', 'A']);
  expect(events).toEqual(['projects', 'flushed', 'projects', 'flushed']);
});
it('keeps a newer name and an unknown project while applying another worker deletion', async () => {
  const base = [{ id: 'A', name: 'A' }, { id: 'B', name: 'Old B' }];
  const { service, values } = fixture({ projects: [{ id: 'C', name: 'C' }, { id: 'A', name: 'A' }, { id: 'B', name: 'New B' }] });
  await service.write('projects', [base[1]], base);
  expect(values.projects).toEqual([{ id: 'C', name: 'C' }, { id: 'B', name: 'New B' }]);
});
it('preserves ordinary store replacement and read-your-writes', async () => {
  const { service } = fixture({ preference: 1 });
  const written = service.write('preference', 2);
  expect(await service.read('preference')).toBe(2); await written;
  await service.remove('preference'); expect(await service.read('preference')).toBeNull();
});
