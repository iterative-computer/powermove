import { describe, expect, it } from 'vitest';
import { HistoryRecords, packHistory, unpackHistory, packStoredHistory, unpackStoredHistory } from './history-memory';

function fixture() {
  const layer = { id: 'a', d: { keys: [{ t: 1, v: 2 }], text: 'unchanged' } };
  return { version: 1, index: 0, entries: Array.from({ length: 3 }, (_, i) => ({ label: `Edit ${i}`,
    forward: [{ path: ['layers'], exists: true, value: [structuredClone(layer), { id: `b${i}` }] }],
    backward: [{ path: ['layers'], exists: true, value: [structuredClone(layer)] }],
  })) };
}

describe('lossless history memory', () => {
  it('checks content when an identifier is reused and finds older revisions', () => {
    const records = new HistoryRecords();
    const first = records.intern({ id: 'same', value: 1 });
    const second = records.intern({ id: 'same', value: 2 });
    expect(second).not.toBe(first);
    expect(records.intern({ id: 'same', value: 1 })).toBe(first);
    expect(records.intern({ id: 'same', value: 1 })).toBe(first);
    records.clear();
    const next = { id: 'same', value: 1 };
    expect(records.intern(next)).toBe(next);
  });

  it('shares only identical immutable records and preserves every entry and cursor through disk JSON', () => {
    const value = fixture(), expected = structuredClone(value), records = new HistoryRecords();
    records.share(value);
    expect(value.entries[0]!.forward[0]!.value[0]).toBe(value.entries[2]!.backward[0]!.value[0]);
    expect(value.entries[0]!.forward[0]!.value[1]).not.toBe(value.entries[1]!.forward[0]!.value[1]);
    const packed = packHistory(value);
    expect(packed.records).toHaveLength(4);
    const restored = unpackHistory(JSON.parse(JSON.stringify(packed)));
    expect(restored).toEqual(expected);
    expect(restored.entries[0].forward[0].value[0]).toBe(restored.entries[2].backward[0].value[0]);
  });

  it('does not reinterpret unrelated store values or invalid legacy histories', () => {
    const unrelated = { entries: [{ payload: 'recovery metadata' }] };
    const records = new HistoryRecords();
    expect(packStoredHistory('projectHistory.demo', unrelated, records)).toBe(unrelated);
    expect(packStoredHistory('theme', fixture(), records).format).toBeUndefined();
    expect(unpackStoredHistory('theme', unrelated)).toBe(unrelated);
  });

  it('retains legacy session metadata, undo-only media and empty/removal patches', () => {
    const value: any = fixture();
    value.entries[0].forward.push({ path: ['edits'], exists: true, value: [] }, { path: ['assets', 'gone'], exists: false });
    value.entries[0].backward.push({ path: ['assets', 'gone'], exists: true, value: { id: 'gone', storageKey: 'file' } });
    const state = { time: 12, history: value }, expected = structuredClone(state);
    const packed = packStoredHistory('projectState.demo', state, new HistoryRecords());
    expect(unpackStoredHistory('projectState.demo', JSON.parse(JSON.stringify(packed)))).toEqual(expected);
  });

  it.each([-1, 9, .5, '0', null])('rejects invalid record reference %s without creating a truncated history', id => {
    const packed = packHistory(fixture());
    packed.history.entries[0].forward[0].refs = [id];
    expect(() => unpackHistory(packed)).toThrow('Invalid history reference');
  });
});
