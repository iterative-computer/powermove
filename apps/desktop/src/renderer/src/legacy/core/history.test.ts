import { describe, expect, it, vi } from 'vitest';

import { createServicesRegistry } from '../../kernel/services';
import type { PMRegistry } from '../registry';
import { install } from './history';
import { install as installMemory } from './memory';

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
  it('retains the complete undo and redo chain under memory pressure', () => {
    const previous = (globalThis as any).window;
    (globalThis as any).window = { addEventListener: vi.fn() };
    try {
      const PM = historyRegistry();
      installMemory(PM);
      PM.Memory.setBudget('history', 1024 * 1024);
      install(PM);
      for (let i = 0; i < 4; i++) PM.hist.do(`Edit ${i}`, () => { PM.proj.text = String(i).repeat(100_000); });
      PM.hist.undo();
      const saved = PM.hist.export();
      PM.Memory.pressure('critical');
      expect(PM.hist.export()).toEqual(saved);
      expect(PM.hist.redo()).toBe(true);
      expect(PM.proj.text).toBe('3'.repeat(100_000));
    } finally { (globalThis as any).window = previous; }
  });
  it('parks a background tab\'s undo stack and resumes it untouched', () => {
    const PM = historyRegistry();
    const cleanup = vi.fn();
    PM.hist.do('Set value', () => { PM.proj.value = 2; });
    // A runtime-only entry, the kind export() cannot carry across a reload.
    PM.hist.external('Replace media', vi.fn(), vi.fn(), { bytes: 1, cleanup });

    const parked = PM.hist.suspend();
    expect(PM.hist.list()).toEqual([]);
    expect(PM.hist.canUndo()).toBe(false);
    // Another tab's edits live on their own stack meanwhile.
    PM.hist.do('Other tab', () => { PM.proj.value = 9; });

    expect(PM.hist.resume(parked)).toBe(true);
    expect(PM.hist.list()).toEqual(['Set value', 'Replace media']);
    expect(PM.hist.label()).toBe('Replace media');
    // Parking and resuming is not closing: nothing was cleaned up.
    expect(cleanup).not.toHaveBeenCalled();

    PM.hist.discard(PM.hist.suspend());
    expect(cleanup).toHaveBeenCalledOnce();
  });

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

  it('counts scoped Unicode, added and removed patches with the unchanged serialized budget', () => {
    const PM = historyRegistry();
    PM.proj.old = 'remove me';
    PM.hist.beginScoped('Several scopes');
    PM.hist.track([['value'], ['old'], ['new'], ['layers']]);
    PM.proj.value = '東京 🎬';
    delete PM.proj.old;
    PM.proj.new = 'added';
    PM.hist.commit();
    const entry = PM.hist.export().entries[0];
    const encoder = new TextEncoder();
    expect(PM.hist.stats().bytes).toBe(encoder.encode(JSON.stringify(entry.forward)).byteLength
      + encoder.encode(JSON.stringify(entry.backward)).byteLength);
    PM.hist.undo();
    expect(PM.proj).toEqual({ value: 1, old: 'remove me', layers: [] });
    PM.hist.redo();
    expect(PM.proj).toEqual({ value: '東京 🎬', new: 'added', layers: [] });
  });

  it('publishes whether a project patch came from the agent or the interface', () => {
    const PM = historyRegistry();
    PM.proj.id = 'project-1';
    PM.hist.begin('Agent edit', null, 'agent');
    PM.proj.value = 2;
    PM.hist.commit();
    expect(PM.bus.emit).toHaveBeenLastCalledWith('history:project-patch', expect.objectContaining({
      projectId: 'project-1', origin: 'agent',
    }));

    PM.hist.do('Interface edit', () => { PM.proj.value = 3; });
    expect(PM.bus.emit).toHaveBeenLastCalledWith('history:project-patch', expect.objectContaining({
      projectId: 'project-1', origin: 'interface',
    }));
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
