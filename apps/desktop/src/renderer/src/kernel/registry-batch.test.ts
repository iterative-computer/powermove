import { expect, it } from 'vitest';
import { Registry } from './registry';

it('publishes one replacement for a reload instead of a temporary removal', () => {
  const registry = new Registry<{ id: string; title: string }>();
  const changes: any[] = [];
  const previous = registry.register('ext', { id: 'panel', title: 'Before' });
  registry.onChange(change => changes.push(change));
  const batch = registry.batchChanges();
  previous.dispose(); registry.register('ext', { id: 'panel', title: 'After' });
  expect(changes).toEqual([]);
  batch.dispose(); batch.dispose();
  expect(changes).toEqual([{ id: 'panel', kind: 'replace' }]);
});

it('publishes a real removal if a reload fails to restore its panel', () => {
  const registry = new Registry<{ id: string }>();
  registry.register('ext', { id: 'panel' });
  const changes: any[] = []; registry.onChange(change => changes.push(change));
  const outer = registry.batchChanges(), inner = registry.batchChanges();
  registry.disposeOwner('ext'); outer.dispose();
  expect(changes).toEqual([]);
  inner.dispose();
  expect(changes).toEqual([{ id: 'panel', kind: 'remove' }]);
});
