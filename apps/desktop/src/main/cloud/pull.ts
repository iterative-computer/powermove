/*
 * Updates as a git pull (store plan §2.6). An installed extension has three
 * trees: the release it was installed from (base), the folder on this computer
 * (ours) and the new release (theirs). Comparing blob shas per path decides
 * each file the way git's three-way merge does, so only what the new release
 * changed is fetched, and changes made here survive an update:
 *
 *   ours = theirs          keep (nothing to do)
 *   ours = base            take theirs (or drop it if the release removed it)
 *   theirs = base          keep ours (only this computer changed it)
 *   otherwise              both changed it: merge the lines, or it conflicts
 *
 * A conflict means the update can't be applied here; the installer then
 * leaves the new release beside the folder for the agent, as before.
 */
import { encodeTree, hashObject, type TreeEntry } from '@powermove/registry/git';
import { diff3Merge } from 'node-diff3';

export interface PullEntry { path: string; sha: string }

export interface PullPlan {
  /** Files that become the release's blob. */
  take: PullEntry[];
  /** Files left exactly as they are on this computer. */
  keep: string[];
  /** Files the release removed that this computer hadn't changed. */
  drop: string[];
  /** Files both sides changed: base is null when both added the path. */
  merge: Array<{ path: string; base: string | null; ours: string; theirs: string }>;
  /** Changed on one side and deleted on the other: no line merge can settle it. */
  conflicts: string[];
}

export function planPull(base: readonly PullEntry[], ours: readonly PullEntry[], theirs: readonly PullEntry[]): PullPlan {
  const index = (entries: readonly PullEntry[]): Map<string, string> => new Map(entries.map((entry) => [entry.path, entry.sha]));
  const b = index(base);
  const o = index(ours);
  const t = index(theirs);
  const plan: PullPlan = { take: [], keep: [], drop: [], merge: [], conflicts: [] };
  const paths = [...new Set([...b.keys(), ...o.keys(), ...t.keys()])].sort();
  for (const path of paths) {
    const inBase = b.get(path) ?? null;
    const inOurs = o.get(path) ?? null;
    const inTheirs = t.get(path) ?? null;
    if (inOurs === inTheirs) {
      if (inOurs !== null) plan.keep.push(path);
    } else if (inOurs === inBase) {
      if (inTheirs === null) plan.drop.push(path);
      else plan.take.push({ path, sha: inTheirs });
    } else if (inTheirs === inBase) {
      if (inOurs !== null) plan.keep.push(path);
    } else if (inOurs === null || inTheirs === null) {
      plan.conflicts.push(path);
    } else {
      plan.merge.push({ path, base: inBase, ours: inOurs, theirs: inTheirs });
    }
  }
  return plan;
}

const decoder = new TextDecoder('utf-8', { fatal: true });

/** UTF-8 text without NUL bytes, or null for anything that isn't text. */
function asText(bytes: Uint8Array): string | null {
  if (bytes.includes(0)) return null;
  try {
    return decoder.decode(bytes);
  } catch {
    return null;
  }
}

/**
 * A line-level three-way merge of one file. Returns the merged bytes, or
 * null when the sides changed the same lines differently or a side isn't
 * text: the caller treats null as a conflict.
 */
export function mergeText(base: Uint8Array | null, ours: Uint8Array, theirs: Uint8Array): Uint8Array | null {
  const o = asText(ours);
  const t = asText(theirs);
  const b = base === null ? '' : asText(base);
  if (o === null || t === null || b === null) return null;
  const regions = diff3Merge(o.split('\n'), b.split('\n'), t.split('\n'));
  const lines: string[] = [];
  for (const region of regions) {
    if ('conflict' in region && region.conflict) return null;
    if ('ok' in region && region.ok) lines.push(...region.ok);
  }
  return new TextEncoder().encode(lines.join('\n'));
}

/**
 * The git tree sha of a file list, computed the way `snapshot` builds it,
 * from paths and blob shas alone. A pull fetches only some bytes, so this is
 * how it proves a listing is the release's tree before trusting it.
 */
export async function treeShaOf(entries: readonly PullEntry[]): Promise<string> {
  interface Dir { files: Map<string, string>; dirs: Map<string, Dir> }
  const dir = (): Dir => ({ files: new Map(), dirs: new Map() });
  const root = dir();
  for (const entry of entries) {
    const segments = entry.path.split('/');
    let cursor = root;
    for (const segment of segments.slice(0, -1)) {
      let child = cursor.dirs.get(segment);
      if (!child) { child = dir(); cursor.dirs.set(segment, child); }
      cursor = child;
    }
    cursor.files.set(segments[segments.length - 1]!, entry.sha);
  }
  const write = async (tree: Dir): Promise<string> => {
    const items: TreeEntry[] = [];
    for (const [name, sha] of tree.files) items.push({ mode: '100644', name, sha });
    for (const [name, child] of tree.dirs) items.push({ mode: '40000', name, sha: await write(child) });
    return hashObject('tree', encodeTree(items));
  };
  return write(root);
}
