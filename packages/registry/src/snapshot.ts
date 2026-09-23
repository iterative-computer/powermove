import { encodeTree, hashObject, type GitObjectType, type TreeEntry } from './git/objects';
import { REGISTRY_LIMITS, type RegistryLimits } from './limits';

export interface SnapshotInput { path: string; bytes: Uint8Array }
export interface SnapshotFile { path: string; sha: string; size: number }
export interface SnapshotObject { sha: string; type: GitObjectType; body: Uint8Array }
export interface Snapshot { treeSha: string; files: SnapshotFile[]; objects: SnapshotObject[]; totalBytes: number }
export type SnapshotErrorCode = 'bad_path' | 'path_too_long' | 'duplicate_path' | 'case_collision' |
  'file_dir_collision' | 'too_many_files' | 'file_too_large' | 'tree_too_large' | 'empty_tree' | 'symlink' | 'not_regular';

export class SnapshotError extends Error {
  constructor(public readonly code: SnapshotErrorCode, public readonly path?: string) {
    super(path ? `${code}: ${path}` : code);
    this.name = 'SnapshotError';
  }
}

const encoder = new TextEncoder();

export function isExcludedPath(path: string): boolean {
  return path.split('/').some((segment) => segment.startsWith('.') || segment === 'node_modules');
}

export function normalizePath(path: string): string {
  if (typeof path !== 'string' || !path || path.startsWith('/') || path.includes('\\') || path.includes('\0')) {
    throw new SnapshotError('bad_path', path);
  }
  const normal = path.normalize('NFC');
  if (normal.split('/').some((segment) => !segment || segment === '.' || segment === '..')) {
    throw new SnapshotError('bad_path', path);
  }
  return normal;
}

function comparePaths(a: string, b: string): number {
  const x = encoder.encode(a);
  const y = encoder.encode(b);
  for (let i = 0; i < Math.min(x.length, y.length); i++) {
    if (x[i] !== y[i]) return x[i]! - y[i]!;
  }
  return x.length - y.length;
}

interface TreeNode { files: Map<string, string>; dirs: Map<string, TreeNode> }
function node(): TreeNode { return { files: new Map(), dirs: new Map() }; }

export async function snapshot(inputs: SnapshotInput[], limits: RegistryLimits = REGISTRY_LIMITS): Promise<Snapshot> {
  const selected: SnapshotInput[] = [];
  const exact = new Set<string>();
  const folded = new Map<string, string>();
  for (const input of inputs) {
    const path = normalizePath(input.path);
    if (isExcludedPath(path)) continue;
    if (encoder.encode(path).length > limits.pathChars) throw new SnapshotError('path_too_long', path);
    if (exact.has(path)) throw new SnapshotError('duplicate_path', path);
    const other = folded.get(path.toLowerCase());
    if (other !== undefined) throw new SnapshotError('case_collision', path);
    exact.add(path);
    folded.set(path.toLowerCase(), path);
    selected.push({ path, bytes: input.bytes });
  }
  if (!selected.length) throw new SnapshotError('empty_tree');
  if (selected.length > limits.files) throw new SnapshotError('too_many_files');
  selected.sort((a, b) => comparePaths(a.path, b.path));
  const files: SnapshotFile[] = [];
  const objects: SnapshotObject[] = [];
  const objectShas = new Set<string>();
  const root = node();
  let totalBytes = 0;
  for (const input of selected) {
    if (input.bytes.length > limits.fileBytes) throw new SnapshotError('file_too_large', input.path);
    totalBytes += input.bytes.length;
    if (totalBytes > limits.treeBytes) throw new SnapshotError('tree_too_large', input.path);
    const segments = input.path.split('/');
    let cursor = root;
    for (const segment of segments.slice(0, -1)) {
      if (cursor.files.has(segment)) throw new SnapshotError('file_dir_collision', input.path);
      let child = cursor.dirs.get(segment);
      if (!child) { child = node(); cursor.dirs.set(segment, child); }
      cursor = child;
    }
    const name = segments[segments.length - 1]!;
    if (cursor.dirs.has(name)) throw new SnapshotError('file_dir_collision', input.path);
    const sha = await hashObject('blob', input.bytes);
    cursor.files.set(name, sha);
    files.push({ path: input.path, sha, size: input.bytes.length });
    if (!objectShas.has(sha)) { objects.push({ sha, type: 'blob', body: input.bytes }); objectShas.add(sha); }
  }
  async function writeTree(tree: TreeNode): Promise<string> {
    const entries: TreeEntry[] = [];
    for (const [name, sha] of tree.files) entries.push({ mode: '100644', name, sha });
    for (const [name, child] of tree.dirs) entries.push({ mode: '40000', name, sha: await writeTree(child) });
    const body = encodeTree(entries);
    const sha = await hashObject('tree', body);
    if (!objectShas.has(sha)) { objects.push({ sha, type: 'tree', body }); objectShas.add(sha); }
    return sha;
  }
  const treeSha = await writeTree(root);
  return { treeSha, files, objects, totalBytes };
}
