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

function isRecord(value: any): value is Record<string, any> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Record only changed leaves. Arrays with the same length are traversed so a
 * single keyframe/property edit stays tiny; structural array edits use one
 * replacement because index-by-index patches would be larger and less safe.
 */
export function diffPatches(before: any, after: any, path: PathPart[] = [], forward: Patch[] = [], backward: Patch[] = []): { forward: Patch[]; backward: Patch[] } {
  if (Object.is(before, after)) return { forward, backward };
  if (Array.isArray(before) && Array.isArray(after)) {
    const identityOrderChanged = before.length === after.length && before.some((item, index) =>
      isRecord(item) && isRecord(after[index]) && typeof item.id === 'string' && typeof after[index].id === 'string'
      && item.id !== after[index].id);
    if (before.length !== after.length || identityOrderChanged) {
      forward.push({ path, exists: true, value: clonePatchValue(after) });
      backward.push({ path, exists: true, value: clonePatchValue(before) });
      return { forward, backward };
    }
    for (let index = 0; index < before.length; index++) {
      diffPatches(before[index], after[index], [...path, index], forward, backward);
    }
    return { forward, backward };
  }
  if (isRecord(before) && isRecord(after)) {
    const keys = new Set([...Object.keys(before), ...Object.keys(after)]);
    for (const key of keys) {
      const inBefore = Object.hasOwn(before, key);
      const inAfter = Object.hasOwn(after, key);
      if (!inBefore || !inAfter) {
        forward.push(inAfter ? { path: [...path, key], exists: true, value: clonePatchValue(after[key]) } : { path: [...path, key], exists: false });
        backward.push(inBefore ? { path: [...path, key], exists: true, value: clonePatchValue(before[key]) } : { path: [...path, key], exists: false });
      } else diffPatches(before[key], after[key], [...path, key], forward, backward);
    }
    return { forward, backward };
  }
  forward.push({ path, exists: true, value: clonePatchValue(after) });
  backward.push({ path, exists: true, value: clonePatchValue(before) });
  return { forward, backward };
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
