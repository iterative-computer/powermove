import { describe, expect, it, vi } from 'vitest';
import { Registry, type RegistryChange } from './registry';

interface Item {
  id: string;
  from: string;
}

const item = (id: string, from: string): Item => ({ id, from });

describe('Registry', () => {
  it('registers, reads, and lists tops in first-registration order of ids', () => {
    const registry = new Registry<Item>();
    registry.register('a', item('one', 'a'));
    registry.register('b', item('two', 'b'));
    registry.register('a', item('three', 'a'));

    expect(registry.get('two')?.from).toBe('b');
    expect(registry.has('two')).toBe(true);
    expect(registry.has('nope')).toBe(false);
    expect(registry.list().map((entry) => entry.id)).toEqual(['one', 'two', 'three']);
    expect(registry.ids()).toEqual(['one', 'two', 'three']);
  });

  it('overrides by id and restores the previous entry when the top is disposed', () => {
    const registry = new Registry<Item>();
    registry.register('base', item('timeline', 'base'));
    const override = registry.register('fork', item('timeline', 'fork'));

    expect(registry.get('timeline')?.from).toBe('fork');
    expect(registry.list()).toHaveLength(1);

    override.dispose();
    expect(registry.get('timeline')?.from).toBe('base');
    expect(registry.has('timeline')).toBe(true);
  });

  it('removes a buried entry without changing the active one', () => {
    const registry = new Registry<Item>();
    const buried = registry.register('base', item('timeline', 'base'));
    registry.register('fork', item('timeline', 'fork'));

    buried.dispose();
    expect(registry.get('timeline')?.from).toBe('fork');
    // The stack is now a single entry: disposing the top empties it.
    expect(registry.ownerEntries('base')).toHaveLength(0);
  });

  it('emits add / replace / remove as the top changes, and nothing for buried removals', () => {
    const registry = new Registry<Item>();
    const changes: RegistryChange[] = [];
    const off = registry.onChange((change) => void changes.push(change));

    const first = registry.register('a', item('panel', 'a'));
    const second = registry.register('b', item('panel', 'b'));
    first.dispose(); // buried → silent
    expect(changes).toEqual([
      { id: 'panel', kind: 'add' },
      { id: 'panel', kind: 'replace' }
    ]);

    second.dispose();
    expect(changes.at(-1)).toEqual({ id: 'panel', kind: 'remove' });

    off.dispose();
    registry.register('a', item('panel', 'a'));
    expect(changes).toHaveLength(3);
  });

  it('emits replace when a covered top is disposed', () => {
    const registry = new Registry<Item>();
    const changes: RegistryChange[] = [];
    registry.register('a', item('fx', 'a'));
    registry.onChange((change) => void changes.push(change));
    const top = registry.register('b', item('fx', 'b'));
    top.dispose();
    expect(changes).toEqual([
      { id: 'fx', kind: 'replace' },
      { id: 'fx', kind: 'replace' }
    ]);
  });

  it('disposeOwner releases every entry of one owner, buried included', () => {
    const registry = new Registry<Item>();
    registry.register('keep', item('one', 'keep'));
    registry.register('ext', item('one', 'ext'));
    registry.register('ext', item('two', 'ext'));
    registry.register('ext', item('three', 'ext'));

    expect(registry.ownerEntries('ext')).toHaveLength(3);
    registry.disposeOwner('ext');

    expect(registry.get('one')?.from).toBe('keep');
    expect(registry.has('two')).toBe(false);
    expect(registry.has('three')).toBe(false);
    expect(registry.ownerEntries('ext')).toEqual([]);
  });

  it('is idempotent on double dispose and rejects items without an id', () => {
    const registry = new Registry<Item>();
    const handle = registry.register('a', item('x', 'a'));
    handle.dispose();
    handle.dispose();
    expect(registry.has('x')).toBe(false);
    expect(() => registry.register('a', { id: '', from: 'a' })).toThrow(/item\.id/);
  });

  it('survives a throwing change listener and clears everything on clear()', () => {
    const registry = new Registry<Item>();
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    registry.onChange(() => {
      throw new Error('boom');
    });
    expect(() => registry.register('a', item('x', 'a'))).not.toThrow();
    expect(error).toHaveBeenCalled();
    registry.clear();
    expect(registry.list()).toEqual([]);
    error.mockRestore();
  });
});
