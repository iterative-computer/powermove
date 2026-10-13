import { describe, expect, it } from 'vitest';
import { applyMemoryOperation, listMemory, MAX_MEMORY_ENTRIES, memoryIndex, sanitizeMemory, type MemoryOperation } from './memory';
import type { MemoryEntry } from './types/project';

let n = 0;
const ctx = (now = 1000) => ({ now, uid: () => `m${++n}`, author: 'agent' as const });
const run = (entries: MemoryEntry[], op: MemoryOperation, now?: number) => applyMemoryOperation(entries, op, ctx(now));

describe('project memory', () => {
  it('adds an entry with its reason and author', () => {
    const { entries, entry } = run([], { operation: 'add', kind: 'decision', text: ' Cut intro to 2s ', why: 'VO starts at 1.8s' });
    expect(entries).toHaveLength(1);
    expect(entry).toMatchObject({ kind: 'decision', text: 'Cut intro to 2s', why: 'VO starts at 1.8s', author: 'agent', createdAt: 1000, updatedAt: 1000 });
  });

  it('keeps one goal: adding a second edits the first', () => {
    const first = run([], { operation: 'add', kind: 'goal', text: '30s teaser' });
    const second = run(first.entries, { operation: 'add', kind: 'goal', text: '45s teaser', why: 'user extended it' }, 2000);
    expect(second.entries).toHaveLength(1);
    expect(second.entry).toMatchObject({ id: first.entry!.id, text: '45s teaser', why: 'user extended it', createdAt: 1000, updatedAt: 2000 });
  });

  it('updates and clears fields without touching the input', () => {
    const { entries, entry } = run([], { operation: 'add', kind: 'open', text: 'Need logo', why: 'waiting' });
    const frozen = JSON.stringify(entries);
    const done = run(entries, { operation: 'update', id: entry!.id, resolved: true, why: '' }, 3000);
    expect(JSON.stringify(entries)).toBe(frozen);
    expect(done.entry).toMatchObject({ resolved: true, updatedAt: 3000 });
    expect(done.entry!.why).toBeUndefined();
  });

  it('rejects empty text, bad ids and a second goal by retyping', () => {
    expect(() => run([], { operation: 'add', kind: 'decision', text: '   ' })).toThrow('empty');
    expect(() => run([], { operation: 'remove', id: 'nope' })).toThrow('No memory entry');
    const goal = run([], { operation: 'add', kind: 'goal', text: 'g' });
    const note = run(goal.entries, { operation: 'add', kind: 'feedback', text: 'punchier' });
    expect(() => run(note.entries, { operation: 'update', id: note.entry!.id, kind: 'goal' })).toThrow('already has a goal');
  });

  it('removes an entry', () => {
    const { entries, entry } = run([], { operation: 'add', kind: 'feedback', text: 'x' });
    expect(run(entries, { operation: 'remove', id: entry!.id }).entries).toEqual([]);
  });

  it('drops the oldest finished, non-standing entry at the cap', () => {
    let entries: MemoryEntry[] = run([], { operation: 'add', kind: 'goal', text: 'g' }).entries;
    entries = run(entries, { operation: 'add', kind: 'open', text: 'done one', resolved: true }).entries;
    for (let i = 0; i < MAX_MEMORY_ENTRIES; i++) entries = run(entries, { operation: 'add', kind: 'decision', text: `d${i}` }).entries;
    expect(entries).toHaveLength(MAX_MEMORY_ENTRIES);
    expect(entries.some(entry => entry.text === 'done one')).toBe(false);
    expect(entries.some(entry => entry.kind === 'goal')).toBe(true);
  });

  it('indexes standing guidance in full and only the latest history', () => {
    let entries: MemoryEntry[] = [];
    entries = run(entries, { operation: 'add', kind: 'goal', text: 'g' }, 1).entries;
    entries = run(entries, { operation: 'add', kind: 'constraint', text: 'brand blue' }, 2).entries;
    entries = run(entries, { operation: 'add', kind: 'open', text: 'logo' }, 3).entries;
    entries = run(entries, { operation: 'add', kind: 'open', text: 'old', resolved: true }, 4).entries;
    for (let i = 0; i < 12; i++) entries = run(entries, { operation: 'add', kind: 'decision', text: `d${i}` }, 10 + i).entries;
    const index = memoryIndex(entries);
    expect(index.total).toBe(entries.length);
    expect(index.goal?.text).toBe('g');
    expect(index.constraints.map(entry => entry.text)).toEqual(['brand blue']);
    expect(index.open.map(entry => entry.text)).toEqual(['logo']);
    expect(index.recent).toHaveLength(8);
    expect(index.recent[0]!.text).toBe('d11');
    expect(memoryIndex(undefined)).toEqual({ total: 0, goal: null, constraints: [], open: [], recent: [] });
  });

  it('lists newest first, filtered and limited', () => {
    let entries: MemoryEntry[] = [];
    entries = run(entries, { operation: 'add', kind: 'decision', text: 'a' }, 1).entries;
    entries = run(entries, { operation: 'add', kind: 'rejected', text: 'b' }, 2).entries;
    entries = run(entries, { operation: 'add', kind: 'decision', text: 'c' }, 3).entries;
    expect(listMemory(entries, { kind: 'decision' }).map(entry => entry.text)).toEqual(['c', 'a']);
    expect(listMemory(entries, { limit: 1 }).map(entry => entry.text)).toEqual(['c']);
  });

  it('drops malformed saved entries and clamps the rest', () => {
    const loaded = sanitizeMemory([
      { id: 'a', kind: 'goal', text: 'ok', author: 'user', createdAt: 5, updatedAt: 6 },
      { id: 'a', kind: 'goal', text: 'duplicate id' },
      { id: 'b', kind: 'nonsense', text: 'bad kind' },
      { id: 'c', kind: 'decision', text: '' },
      { kind: 'decision', text: 'no id' },
      null, 'x',
      { id: 'd', kind: 'decision', text: 'x'.repeat(5000), resolved: 'yes' }
    ], 99);
    expect(loaded.map(entry => entry.id)).toEqual(['a', 'd']);
    expect(loaded[0]).toMatchObject({ author: 'user', createdAt: 5, updatedAt: 6 });
    expect(loaded[1]!.text).toHaveLength(600);
    expect(loaded[1]).toMatchObject({ author: 'agent', createdAt: 99 });
    expect(loaded[1]!.resolved).toBeUndefined();
    expect(sanitizeMemory('nope')).toEqual([]);
  });
});
