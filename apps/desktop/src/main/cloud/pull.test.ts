import { describe, expect, it } from 'vitest';
import { snapshot } from '@powermove/registry/snapshot';
import { mergeText, planPull, treeShaOf } from './pull';

const e = (path: string, sha: string) => ({ path, sha });
const text = (value: string): Uint8Array => new TextEncoder().encode(value);
const read = (bytes: Uint8Array | null): string | null => (bytes ? new TextDecoder().decode(bytes) : null);

describe('planPull', () => {
  it('decides every path the way a three-way merge does', () => {
    const base = [e('same', 'a'), e('author', 'a'), e('mine', 'a'), e('both', 'a'), e('gone', 'a'), e('edited-gone', 'a'), e('i-removed', 'a')];
    const ours = [e('same', 'a'), e('author', 'a'), e('mine', 'm'), e('both', 'm'), e('gone', 'a'), e('edited-gone', 'm'), e('local-only', 'x')];
    const theirs = [e('same', 'a'), e('author', 't'), e('mine', 'a'), e('both', 't'), e('edited-gone', 'a'), e('i-removed', 'a'), e('new', 'n')];
    expect(planPull(base, ours, theirs)).toEqual({
      take: [e('author', 't'), e('new', 'n')],
      keep: ['edited-gone', 'local-only', 'mine', 'same'],
      drop: ['gone'],
      merge: [{ path: 'both', base: 'a', ours: 'm', theirs: 't' }],
      conflicts: []
    });
  });

  it('flags a file changed on one side and deleted on the other', () => {
    const plan = planPull([e('a', '1'), e('b', '1')], [e('a', '2')], [e('b', '2')]);
    expect(plan.conflicts).toEqual(['a', 'b']);
  });

  it('merges a path both sides added, from an empty base', () => {
    expect(planPull([], [e('a', '1')], [e('a', '2')]).merge).toEqual([{ path: 'a', base: null, ours: '1', theirs: '2' }]);
  });

  it('is a fast-forward when this computer changed nothing', () => {
    const base = [e('a', '1'), e('b', '1')];
    const plan = planPull(base, base, [e('a', '2'), e('c', '3')]);
    expect(plan).toEqual({ take: [e('a', '2'), e('c', '3')], keep: [], drop: ['b'], merge: [], conflicts: [] });
  });
});

describe('mergeText', () => {
  it('keeps both sides when they changed different lines', () => {
    const base = text('one\ntwo\nthree\nfour\n');
    expect(read(mergeText(base, text('ONE\ntwo\nthree\nfour\n'), text('one\ntwo\nthree\nFOUR\n')))).toBe('ONE\ntwo\nthree\nFOUR\n');
  });

  it('refuses when both changed the same line differently', () => {
    expect(mergeText(text('a\nb\nc'), text('a\nX\nc'), text('a\nY\nc'))).toBeNull();
  });

  it('accepts the same change made on both sides', () => {
    expect(read(mergeText(text('a\nb\nc'), text('a\nX\nc'), text('a\nX\nc')))).toBe('a\nX\nc');
  });

  it('refuses binary or non-UTF-8 files', () => {
    expect(mergeText(text('a'), new Uint8Array([0, 1, 2]), text('b'))).toBeNull();
    expect(mergeText(text('a'), new Uint8Array([0xff, 0xfe]), text('b'))).toBeNull();
  });

  it('treats two different additions of one file as a conflict', () => {
    expect(mergeText(null, text('mine'), text('theirs'))).toBeNull();
  });
});

describe('treeShaOf', () => {
  it('matches the tree snapshot builds from the same files', async () => {
    const files = [
      { path: 'manifest.json', bytes: text('{"id":"a"}\n') },
      { path: 'index.ts', bytes: text('export default 1\n') },
      { path: 'shaders/glass.frag', bytes: text('void main() {}\n') },
      { path: 'shaders/deep/edge.frag', bytes: text('// edge\n') }
    ];
    const snap = await snapshot(files);
    expect(await treeShaOf(snap.files)).toBe(snap.treeSha);
    expect(await treeShaOf(snap.files.slice(1))).not.toBe(snap.treeSha);
  });
});
