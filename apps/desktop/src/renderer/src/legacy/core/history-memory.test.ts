import { expect, it, vi } from 'vitest';
import { install } from './history';
import { install as installMemory } from './memory';

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

it('keeps the next redo applicable when explicitly trimming all-undone history', () => {
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

it('keeps redo usable when lowering the real history budget', () => {
  vi.stubGlobal('window', { addEventListener: vi.fn() });
  try {
    const { PM } = editor();
    installMemory(PM);
    install(PM);
    PM.hist.do('Add layer', () => { PM.proj.layers.push({ id: 'a', name: 'Original', text: 'x'.repeat(600_000) }); });
    PM.hist.do('Rename layer', () => { PM.proj.layers[0].name = 'Renamed'; });
    PM.hist.do('Delete layer', () => { PM.proj.layers = []; });
    expect(PM.hist.stats().bytes).toBeGreaterThan(1024 * 1024);
    PM.hist.undo(); PM.hist.undo(); PM.hist.undo();

    expect(PM.Memory.setBudget('history', 1024 * 1024)).toBe(true);
    expect(PM.hist.stats().bytes).toBeLessThanOrEqual(1024 * 1024);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.proj.layers).toHaveLength(1);
    expect(PM.proj.layers[0].name).toBe('Original');
    expect(PM.hist.redo()).toBe(true);
    expect(PM.proj.layers[0].name).toBe('Renamed');
    expect(PM.hist.canRedo()).toBe(false);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.layers[0].name).toBe('Original');
  } finally { vi.unstubAllGlobals(); }
});
