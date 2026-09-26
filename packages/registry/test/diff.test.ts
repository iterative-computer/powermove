import { expect, test } from 'bun:test';
import { diffTrees } from '../src/diff';
test('bytewise paths and counts', () => {
  expect(diffTrees([{ path: 'b', sha: '1' }, { path: 'same', sha: 'x' }, { path: 'z', sha: '0' }], [{ path: 'a', sha: '2' }, { path: 'b', sha: '2' }, { path: 'same', sha: 'x' }])).toEqual({ files: [{ path: 'a', status: 'added' }, { path: 'b', status: 'modified' }, { path: 'z', status: 'removed' }], counts: { added: 1, modified: 1, removed: 1 } });
});
test('empty and unchanged', () => {
  expect(diffTrees([], [])).toEqual({ files: [], counts: { added: 0, removed: 0, modified: 0 } });
  expect(diffTrees([{ path: 'x', sha: 'a' }], [{ path: 'x', sha: 'a' }]).files).toEqual([]);
});
