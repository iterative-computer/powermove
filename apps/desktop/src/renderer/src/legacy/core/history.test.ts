import { describe, expect, it, vi } from 'vitest';

import { createServicesRegistry } from '../../kernel/services';
import type { PMRegistry } from '../registry';
import { install } from './history';

function historyRegistry(): PMRegistry {
  let nextId = 0;
  const PM: PMRegistry = {
    Kernel: { services: createServicesRegistry() },
    proj: { value: 1, layers: [] },
    uid: (prefix: any) => `${prefix}-${++nextId}`,
    replaceProject(project: any) { PM.proj = project; },
    touch: vi.fn(),
    bus: { emit: vi.fn() },
    invalidate: vi.fn(),
    toast: vi.fn(),
    autosave: vi.fn(),
    store: { get: (_key: any, fallback: any) => fallback, set: vi.fn() },
  };
  install(PM);
  return PM;
}

describe('legacy history install', () => {
  it('records one source transaction and reverses and reapplies it', () => {
    const PM = historyRegistry();

    PM.hist.do('Set value', () => { PM.proj.value = 2; });

    expect(PM.hist.list()).toEqual(['Set value']);
    expect(PM.hist.canUndo()).toBe(true);
    expect(PM.hist.label()).toBe('Set value');
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.value).toBe(1);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.proj.value).toBe(2);
  });

  it('stores a small patch and reports a strict byte budget', () => {
    const PM = historyRegistry();
    PM.proj.large = Array.from({ length: 10_000 }, (_, index) => ({ index, stable: true }));
    PM.hist.do('Set value', () => { PM.proj.value = 3; });

    const stats = PM.hist.stats();
    expect(stats.entries).toBe(1);
    expect(stats.bytes).toBeLessThan(1_000);
    expect(stats.maxBytes).toBe(256 * 1024 * 1024);
  });

  it('retains a deep run of compact edits instead of discarding them at 120 steps', () => {
    const PM = historyRegistry();
    for (let value = 2; value <= 302; value++) {
      PM.hist.do(`Set ${value}`, () => { PM.proj.value = value; });
    }

    expect(PM.hist.stats().entries).toBe(301);
    for (let value = 301; value >= 1; value--) expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.value).toBe(1);
  });
});

describe('selection history', () => {
  it('restores selection when no timeline service is registered', () => {
    const PM = historyRegistry();
    PM.proj.layers = [{ id: 'a' }, { id: 'b' }];
    const before = { layers: ['a'], keys: ['key-a'], chan: 'opacity' };
    const after = { layers: ['b'], keys: ['key-b'], chan: 'position.x' };
    PM.sel = after;

    expect(PM.Kernel.services.get('timeline')).toBeNull();
    PM.hist.selection(before, after);
    expect(() => PM.hist.undo()).not.toThrow();
    expect(PM.sel).toEqual(before);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.sel).toEqual(after);
  });

  it('restores selection without changing source and suppresses selection inside edits', () => {
    const PM = historyRegistry();
    PM.proj.layers = [{id:'a'}, {id:'b'}];
    const before = {layers:['a'],keys:[],chan:null}, after = {layers:['b'],keys:[],chan:null};
    PM.sel = after;
    PM.hist.selection(before, after);
    PM.hist.undo(); expect(PM.sel.layers).toEqual(['a']);
    PM.hist.redo(); expect(PM.sel.layers).toEqual(['b']);
    PM.hist.begin('Edit');
    PM.hist.selection(after, before);
    PM.proj.value = 2; PM.hist.commit();
    expect(PM.hist.list()).toEqual(['Selection','Edit']);
    PM.hist.selection(before,before);
    expect(PM.hist.list()).toHaveLength(2);
    expect(PM.autosave).toHaveBeenCalledTimes(1);
  });
});
