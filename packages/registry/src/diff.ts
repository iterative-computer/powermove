export interface DiffEntry { path: string; status: 'added' | 'removed' | 'modified' }
export interface DiffResult { files: DiffEntry[]; counts: { added: number; removed: number; modified: number } }
export function diffTrees(base: { path: string; sha: string }[], head: { path: string; sha: string }[]): DiffResult {
  const before = new Map(base.map(({ path, sha }) => [path, sha]));
  const after = new Map(head.map(({ path, sha }) => [path, sha]));
  const paths = [...new Set([...before.keys(), ...after.keys()])].sort((a, b) => a < b ? -1 : a > b ? 1 : 0);
  const files: DiffEntry[] = [];
  const counts = { added: 0, removed: 0, modified: 0 };
  for (const path of paths) {
    const old = before.get(path), current = after.get(path);
    if (old === current) continue;
    const status = old === undefined ? 'added' : current === undefined ? 'removed' : 'modified';
    files.push({ path, status });
    counts[status]++;
  }
  return { files, counts };
}
