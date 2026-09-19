/*
 * Leaf-level project patches. The renderer's history records every edit as a
 * list of these (see legacy/core/history.ts); the remote host replays them
 * onto its authoritative copy of the document and forwards them to other
 * tabs, so one shape and one apply routine serve both sides.
 */
export type PathPart = string | number;
export interface Patch { path: PathPart[]; exists: boolean; value?: any }

export const clonePatchValue = (value: any) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));

/** Applies patches in place and returns the (possibly replaced) root. */
export function applyPatch(root: any, patches: Patch[]): any {
  let nextRoot = root;
  for (const patch of patches) {
    if (!patch.path.length) {
      nextRoot = patch.exists ? clonePatchValue(patch.value) : undefined;
      continue;
    }
    let parent = nextRoot;
    for (let index = 0; index < patch.path.length - 1; index++) {
      parent = parent?.[patch.path[index]!];
      if (parent == null) break;
    }
    if (parent == null) continue;
    const key = patch.path.at(-1)!;
    if (patch.exists) parent[key] = clonePatchValue(patch.value);
    else if (Array.isArray(parent) && typeof key === 'number') parent.splice(key, 1);
    else delete parent[key];
  }
  return nextRoot;
}

export function isPatchList(value: unknown, maxBytes = 32 * 1024 * 1024): value is Patch[] {
  if (!Array.isArray(value) || value.length === 0) return false;
  let bytes = 0;
  for (const item of value) {
    if (!item || typeof item !== 'object' || !Array.isArray((item as Patch).path) || typeof (item as Patch).exists !== 'boolean') return false;
    if (!(item as Patch).path.every((part) => typeof part === 'string' || (typeof part === 'number' && Number.isInteger(part) && part >= 0))) return false;
    if ((item as Patch).exists) {
      try { bytes += JSON.stringify((item as Patch).value ?? null).length; } catch { return false; }
      if (bytes > maxBytes) return false;
    }
  }
  return true;
}
