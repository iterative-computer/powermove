import { expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { packHistory, unpackHistory } from '../../../../shared/history-memory';
import { install as installHistory } from './history';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  PM.proj = PM.mkProject({ name: 'Canonical history', w: 320, h: 180, dur: 2 });
  return PM;
}

function populated() {
  const PM = editor();
  for (let index = 0; index < 12; index++) PM.hist.do(`Add shape ${index}`, () => {
    PM.proj.layers.push(PM.mkLayer('shape', { name: `Shape ${index}` }));
  });
  PM.hist.undo(); PM.hist.undo();
  return PM;
}

it('compacts real composition history without losing actions, pending redo or source records', () => {
  const PM = populated(), history = PM.hist.export({ copy: false });
  expect(history.entries[0].forward[0].path).toEqual(['comps', PM.proj.compId, 'layers']);
  const packed = packHistory(history);
  expect(packed.format).toBe('powermove-history-dictionary');
  expect(packed.records).toHaveLength(12);
  expect(packed.history.index).toBe(9);
  const restored = unpackHistory(JSON.parse(JSON.stringify(packed)));
  expect(restored).toEqual(PM.hist.export());
  const next = editor();
  next.proj = structuredClone(PM.proj);
  expect(next.hist.import(restored)).toBe(true);
  for (let count = 9; count >= 0; count--) {
    expect(next.hist.undo()).toBe(true);
    expect(next.proj.layers).toHaveLength(count);
  }
  expect(next.hist.undo()).toBe(false);
  for (let count = 1; count <= 12; count++) {
    expect(next.hist.redo()).toBe(true);
    expect(next.proj.layers).toHaveLength(count);
  }
  expect(next.hist.redo()).toBe(false);
});

it('interns separately parsed canonical records during prepared import without changing the outgoing history', async () => {
  const PM = populated(), saved = JSON.parse(JSON.stringify(PM.hist.export()));
  const next = editor();
  next.hist.do('Outgoing edit', () => { next.proj.name = 'Outgoing'; });
  const token = await next.hist.prepareImport(saved);
  expect(next.hist.list()).toEqual(['Outgoing edit']);
  next.proj = structuredClone(PM.proj);
  expect(next.hist.import(saved, token)).toBe(true);
  const packed = packHistory(next.hist.export({ copy: false }));
  expect(packed.format).toBe('powermove-history-dictionary');
  expect(packed.records).toHaveLength(12);
  next.hist.undo(); expect(next.proj.layers).toHaveLength(9);
  next.hist.redo(); expect(next.proj.layers).toHaveLength(10);
  next.hist.redo(); expect(next.proj.layers).toHaveLength(11);
});

it('keeps the next canonical redo chain applicable when its serialized byte budget is lowered', () => {
  const PM = editor();
  let trim!: (bytes: number) => void;
  PM.Memory = { register: (_name: string, provider: { trim: typeof trim }) => { trim = provider.trim; } };
  installHistory(PM);
  PM.hist.do('Add large layer', () => {
    const layer = PM.mkLayer('shape', { name: 'Original' });
    layer.d.notes = 'x'.repeat(600_000);
    PM.proj.layers.push(layer);
  });
  PM.hist.do('Rename layer', () => { PM.proj.layers[0].name = 'Renamed'; });
  PM.hist.do('Delete layer', () => { PM.proj.layers = []; });
  expect(PM.hist.stats().bytes).toBeGreaterThan(1024 * 1024);
  PM.hist.undo(); PM.hist.undo(); PM.hist.undo();
  trim(1024 * 1024);
  expect(PM.hist.stats().bytes).toBeLessThanOrEqual(1024 * 1024);
  expect(PM.hist.redo()).toBe(true);
  expect(PM.proj.layers[0].name).toBe('Original');
  expect(PM.hist.redo()).toBe(true);
  expect(PM.proj.layers[0].name).toBe('Renamed');
  expect(PM.hist.redo()).toBe(false);
  expect(PM.hist.undo()).toBe(true);
  expect(PM.proj.layers[0].name).toBe('Original');
});
