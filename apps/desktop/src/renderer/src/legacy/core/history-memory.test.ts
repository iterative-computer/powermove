import { expect, it, vi } from 'vitest';
import { install } from './history';

function editor() {
  let provider: { trim: (bytes: number) => void };
  const PM: any = {
    proj: { layers: [] }, sel: {}, uid: () => Math.random().toString(),
    replaceProject: (project: any) => { PM.proj = project; },
    bus: { emit: vi.fn() }, touch: vi.fn(), invalidate: vi.fn(), autosave: vi.fn(),
    Memory: { register: (_name: string, value: typeof provider) => { provider = value; } },
  };
  install(PM);
  return { PM, trim: (bytes: number) => provider.trim(bytes) };
}

it('keeps the next redo applicable when memory pressure arrives after undoing all edits', () => {
  const { PM, trim } = editor();
  PM.hist.do('Add layer', () => { PM.proj.layers.push({ id: 'a', name: 'Original' }); });
  PM.hist.do('Rename layer', () => { PM.proj.layers[0].name = 'Renamed'; });
  PM.hist.undo();
  PM.hist.undo();
  trim(1);

  expect(PM.hist.redo()).toBe(true);
  expect(PM.proj.layers).toEqual([{ id: 'a', name: 'Original' }]);
  expect(PM.hist.canRedo()).toBe(false);
  expect(PM.hist.undo()).toBe(true);
  expect(PM.proj.layers).toEqual([]);
});

it('keeps a contiguous undo and redo chain when trimming inside the history', () => {
  const { PM, trim } = editor();
  PM.hist.do('Add layer', () => { PM.proj.layers.push({ id: 'a', name: 'Original' }); });
  PM.hist.do('Rename layer', () => { PM.proj.layers[0].name = 'Renamed'; });
  PM.hist.do('Delete layer', () => { PM.proj.layers = []; });
  PM.hist.undo();
  PM.hist.undo();
  trim(1);

  expect(PM.hist.undo()).toBe(true);
  expect(PM.proj.layers).toEqual([]);
  expect(PM.hist.redo()).toBe(true);
  expect(PM.proj.layers).toEqual([{ id: 'a', name: 'Original' }]);
});

it('cleans up evicted redo callbacks without invoking them', () => {
  const { PM, trim } = editor();
  const cleanup = vi.fn();
  const redo = vi.fn();
  PM.hist.do('Add layer', () => { PM.proj.layers.push({ id: 'a' }); });
  PM.hist.external('Interface change', vi.fn(), redo, { cleanup });
  PM.hist.undo();
  PM.hist.undo();
  trim(1);
  expect(cleanup).toHaveBeenCalledOnce();
  expect(redo).not.toHaveBeenCalled();
  expect(PM.hist.stats().entries).toBe(1);
});
