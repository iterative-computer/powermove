/* Compact, byte-budgeted project history. */
import type { PMRegistry } from '../registry';
import { timelineService } from './services';
import { HistoryRecords, unpackHistory } from '../../../../shared/history-memory';

import { applyPatch, clonePatchValue, diffPatches as diff, type Patch, type PathPart } from '../../../../shared/patch';

// Patch entries are tiny for ordinary edits. Keep a deep practical timeline
// while the byte budget remains the hard memory bound for structural changes.
const MAX_ENTRIES = 1_000;
const DEFAULT_MAX_BYTES = 256 * 1024 * 1024;

const clone = clonePatchValue;
const encodedBytes = (value: any) => new TextEncoder().encode(JSON.stringify(value)).byteLength;

function isRecord(value: any): value is Record<string, any> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

function valueAt(root: any, path: PathPart[]): { exists: boolean; value?: any } {
  if (!path.length) return { exists: root !== undefined, value: clone(root) };
  let parent = root;
  for (let index = 0; index < path.length - 1; index++) {
    parent = parent?.[path[index]!];
    if (parent == null) return { exists: false };
  }
  const key = path.at(-1)!;
  return parent != null && Object.hasOwn(parent, key)
    ? { exists: true, value: clone(parent[key]) }
    : { exists: false };
}

export function install(PM: PMRegistry): void {
  let records = new HistoryRecords();
  let maxBytes = PM.Memory?.budget?.('history') || DEFAULT_MAX_BYTES;
  type Entry = {
    id: string;
    label: string;
    group?: any;
    bytes: number;
    undo: () => void;
    redo: () => void;
    project?: boolean;
    forward?: Patch[];
    backward?: Patch[];
    cleanup?: () => void;
  };

  let stack: Entry[] = [];
  let idx = -1;
  let pending: any = null;
  let depth = 0;
  let totalBytes = 0;

  const snap = () => JSON.stringify(PM.proj);
  const restore = (json: string, selection?: any) => {
    PM.replaceProject(JSON.parse(json), selection ? { selection } : {});
  };
  const publish = () => {
    PM.bus.emit('history');
    PM.invalidate('status');
  };
  const trim = () => {
    while (stack.length > MAX_ENTRIES || totalBytes > maxBytes && stack.length > 1) {
      const removed = stack.shift()!;
      totalBytes -= removed.bytes;
      removed.cleanup?.();
      idx--;
    }
    idx = Math.min(idx, stack.length - 1);
  };
  const discardRedo = () => {
    if (idx >= stack.length - 1) return;
    for (const entry of stack.slice(idx + 1)) {
      totalBytes -= entry.bytes;
      entry.cleanup?.();
    }
    stack = stack.slice(0, idx + 1);
  };
  const push = (entry: Omit<Entry, 'id'> & { id?: string }) => {
    discardRedo();
    if (entry.forward) records.sharePatches(entry.forward);
    if (entry.backward) records.sharePatches(entry.backward);
    const next: Entry = { ...entry, id: entry.id || PM.uid('history') };
    stack.push(next);
    totalBytes += next.bytes;
    idx = stack.length - 1;
    trim();
    publish();
    return next.id;
  };
  const restorePatches = (patches: Patch[]) => {
    const selection = clone(PM.sel);
    const next = applyPatch(PM.proj, patches);
    PM.replaceProject(next, { selection });
  };
  const publishProjectPatch = (patches?: Patch[], origin: any = 'interface') => {
    if (!patches?.length || !PM.proj?.id) return;
    PM.bus.emit('history:project-patch', {
      projectId: PM.proj.id, revision: PM.proj.revision || 0,
      origin: origin === 'agent' ? 'agent' : 'interface', patches: clone(patches),
    });
  };

  const scopedPatches = (current: any) => {
    const forward: Patch[] = [];
    const backward: Patch[] = [];
    // Compare and count the same serialization. Structural scopes can contain
    // megabytes; serializing them again solely for the budget delays commits.
    const encoder = new TextEncoder();
    let bytes = 4; // Two JSON arrays: [] and [].
    for (const item of current.scopes.values()) {
      const after = valueAt(PM.proj, item.path);
      const next = { path: item.path, ...after }, previous = { path: item.path, ...item.before };
      const nextJson = JSON.stringify(next), previousJson = JSON.stringify(previous);
      if (nextJson === previousJson) continue;
      if (forward.length) bytes += 2; // One comma in each array.
      bytes += encoder.encode(nextJson).byteLength + encoder.encode(previousJson).byteLength;
      forward.push(next);
      backward.push(previous);
    }
    return { forward, backward, bytes };
  };

  const pathKey = (path: PathPart[]) => JSON.stringify(path);

  const H: any = {
    begin(label: any, group: any = null, origin: any = 'interface') {
      if (depth++ > 0) return pending?.before || null;
      pending = { label, before: snap(), group, origin };
      return pending.before;
    },
    beginScoped(label: any, group: any = null, origin: any = 'interface') {
      if (depth++ > 0) return false;
      pending = { label, group, origin, scoped: true, scopes: new Map() };
      return true;
    },
    track(paths: PathPart[][]) {
      if (!pending?.scoped) return false;
      for (const rawPath of paths || []) {
        const path = [...rawPath];
        /* A parent snapshot already covers this path. If a new parent arrives,
           remove narrower children so a value is never restored twice. */
        const covered = [...pending.scopes.values()].some((item: any) =>
          item.path.length <= path.length && item.path.every((part: any, index: number) => part === path[index]));
        if (covered) continue;
        const before = valueAt(PM.proj, path);
        for (const [key, item] of pending.scopes) {
          if (path.length < item.path.length && path.every((part, index) => part === item.path[index])) {
            // The live parent already includes earlier child edits. Fold the
            // original child values into its snapshot before discarding them.
            before.value = applyPatch(before.value, [{ path: item.path.slice(path.length), ...item.before }]);
            pending.scopes.delete(key);
          }
        }
        pending.scopes.set(pathKey(path), { path, before });
      }
      return true;
    },
    hasChanges() {
      if (!pending) return false;
      if (!pending.scoped) return snap() !== pending.before;
      return scopedPatches(pending).forward.length > 0;
    },
    commit(label: any) {
      if (depth <= 0) return false;
      if (--depth > 0) return false;
      if (!pending) return false;
      const current = pending;
      pending = null;
      let patches: { forward: Patch[]; backward: Patch[]; bytes?: number };
      if (current.scoped) patches = scopedPatches(current);
      else {
        const afterJson = snap();
        if (afterJson === current.before) return false;
        patches = diff(JSON.parse(current.before), JSON.parse(afterJson));
      }
      if (!patches.forward.length) return false;
      const bytes = patches.bytes ?? encodedBytes(patches.forward) + encodedBytes(patches.backward);
      push({
        label: label || current.label,
        group: current.group,
        bytes,
        project: true,
        forward: patches.forward,
        backward: patches.backward,
        undo: () => restorePatches(patches.backward),
        redo: () => restorePatches(patches.forward),
      });
      publishProjectPatch(patches.forward, current.origin);
      PM.touch();
      PM.autosave?.();
      return true;
    },
    cancel() { depth = 0; pending = null; },
    rollback(selection?: any) {
      if (!pending) return false;
      const current = pending;
      depth = 0;
      pending = null;
      if (current.scoped) {
        const patches: Patch[] = [...current.scopes.values()].map((item: any) => ({ path: item.path, ...item.before }));
        const next = applyPatch(PM.proj, patches);
        PM.replaceProject(next, selection ? { selection } : {});
      } else restore(current.before, selection);
      return true;
    },
    pendingSnapshot: () => pending?.before || null,
    /** Wrap a legacy direct mutation. PM.Edit uses the same compact patch entry. */
    do(label: any, fn: any, origin: any = 'interface') {
      H.begin(label, null, origin);
      try { return fn(); }
      catch (error) { H.rollback(); throw error; }
      finally { if (pending) H.commit(label); }
    },
    undo() {
      if (idx < 0) return false;
      const entry = stack[idx--]!;
      entry.undo();
      publishProjectPatch(entry.backward, 'interface');
      publish();
      return true;
    },
    redo() {
      if (idx >= stack.length - 1) return false;
      const entry = stack[++idx]!;
      entry.redo();
      publishProjectPatch(entry.forward, 'interface');
      publish();
      return true;
    },
    canUndo: () => idx >= 0,
    canRedo: () => idx < stack.length - 1,
    label: () => idx >= 0 ? stack[idx]!.label : null,
    mark() { return { index: idx, topId: stack[idx]?.id || null }; },
    /** Combine a bounded agent run without materializing another project copy. */
    squash(mark: any, label: any = 'Agent change', group: any = null) {
      if (!mark || !Number.isInteger(mark.index)) return null;
      const baseIndex = mark.topId ? stack.findIndex(entry => entry.id === mark.topId) : -1;
      if (mark.topId && baseIndex < 0) return null;
      if (mark.index < -1 || baseIndex > idx || idx !== stack.length - 1) return null;
      const entries = stack.slice(baseIndex + 1);
      if (!entries.length || entries.some(entry => !entry.project || group && entry.group !== group)) return null;
      stack = stack.slice(0, baseIndex + 1);
      totalBytes = stack.reduce((sum, entry) => sum + entry.bytes, 0);
      idx = stack.length - 1;
      return push({
        label,
        bytes: entries.reduce((sum, entry) => sum + entry.bytes, 0),
        project: true,
        forward: entries.flatMap(entry => entry.forward || []),
        backward: [...entries].reverse().flatMap(entry => entry.backward || []),
        undo: () => { for (let index = entries.length - 1; index >= 0; index--) entries[index]!.undo(); },
        redo: () => { for (const entry of entries) entry.redo(); },
        cleanup: entries.some(entry => entry.cleanup)
          ? () => { for (const entry of entries) entry.cleanup?.(); }
          : undefined,
      });
    },
    selection(before: any, after: any) {
      if (pending || JSON.stringify(before) === JSON.stringify(after)) return null;
      const previous = clone(before), next = clone(after), projectId = PM.proj.id;
      const restoreSelection = (selection: any) => {
        if (PM.proj.id !== projectId) return;
        const ids = new Set((PM.curComp?.() || PM.proj).layers.map((layer: any) => layer.id));
        Object.assign(PM.sel, clone(selection), { layers: selection.layers.filter((id: string) => ids.has(id)) });
        const timeline = timelineService(PM);
        if (timeline) timeline.keySelectionActive = !!PM.sel.keys.length;
        PM.bus.emit('sel'); PM.invalidate();
      };
      return push({ label: 'Selection', bytes: encodedBytes(previous) + encodedBytes(next),
        undo: () => restoreSelection(previous), redo: () => restoreSelection(next) });
    },
    external(label: any, undo: any, redo: any, options: any = {}) {
      if (typeof undo !== 'function' || typeof redo !== 'function') return null;
      return push({
        label: label || 'Interface change', undo, redo,
        bytes: Math.max(256, Number(options.bytes) || 0),
        cleanup: typeof options.cleanup === 'function' ? options.cleanup : undefined,
      });
    },
    undoIfTop(id: any) {
      if (!id || stack[idx]?.id !== id) return false;
      return H.undo();
    },
    restoreSnapshot(json: any, label: any = 'Restore checkpoint', origin: any = 'interface') {
      let parsed: any;
      try { parsed = JSON.parse(json); } catch { return false; }
      if (!parsed || typeof parsed !== 'object' || !Array.isArray(parsed.layers)) return false;
      H.do(label, () => PM.replaceProject(parsed), origin);
      return true;
    },
    /** Persist document edits; runtime UI callbacks belong to this session only. */
    export(options: { copy?: boolean } = {}) {
      const entries = stack.filter(entry => entry.project && entry.forward && entry.backward);
      const saved = { version: 1,
        index: stack.slice(0, idx + 1).filter(entry => entry.project && entry.forward && entry.backward).length - 1,
        entries: entries.map(entry => ({ label: entry.label, forward: entry.forward, backward: entry.backward })),
      };
      // History patches are immutable after publication. Internal serializers can
      // read them without cloning the entire undo history before every save.
      return options.copy === false ? saved : clone(saved);
    },
    import(saved: any) {
      H.clear();
      try { saved = unpackHistory(structuredClone(saved)); }
      catch { return false; }
      const validPatches = (patches: any) => Array.isArray(patches) && patches.every(patch =>
        patch && Array.isArray(patch.path) && typeof patch.exists === 'boolean'
        && patch.path.every((part: any) => typeof part === 'string'
          ? !['__proto__', 'prototype', 'constructor'].includes(part)
          : Number.isSafeInteger(part) && part >= 0));
      if (saved?.version !== 1 || !Array.isArray(saved.entries) || saved.entries.length > MAX_ENTRIES
          || !Number.isInteger(saved.index) || saved.index < -1 || saved.index >= saved.entries.length
          || !saved.entries.every((entry: any) => typeof entry?.label === 'string'
            && validPatches(entry.forward) && validPatches(entry.backward))) return false;
      records.share(saved);
      for (const source of saved.entries) {
        const { label, forward, backward } = source;
        stack.push({ id: PM.uid('history'), label, project: true, forward, backward,
          bytes: encodedBytes(forward) + encodedBytes(backward),
          undo: () => restorePatches(backward), redo: () => restorePatches(forward) });
      }
      idx = saved.index;
      totalBytes = stack.reduce((sum, entry) => sum + entry.bytes, 0);
      // Remove distant redo entries first when reopening under a smaller budget.
      while (totalBytes > maxBytes && stack.length > idx + 1) totalBytes -= stack.pop()!.bytes;
      trim();
      idx = Math.max(-1, idx);
      publish();
      return true;
    },
    clear() {
      stack.forEach(entry => entry.cleanup?.());
      stack = [];
      idx = -1;
      pending = null;
      depth = 0;
      records.clear();
      totalBytes = 0;
      publish();
    },
    /**
     * Sets the live undo stack aside, untouched, for a project going into a
     * background tab, and leaves an empty one. Unlike export(), this keeps the
     * session-only entries (a media replacement, say) and their retained
     * runtimes, so resuming the tab puts back exactly what it had.
     */
    suspend() {
      const parked = { stack, idx, totalBytes, records };
      stack = [];
      idx = -1;
      pending = null;
      depth = 0;
      totalBytes = 0;
      records = new HistoryRecords();
      publish();
      return parked;
    },
    /** Puts back a stack set aside by suspend(), replacing the current one. */
    resume(parked: any) {
      if (!parked || !Array.isArray(parked.stack)) return false;
      H.clear();
      ({ stack, idx, totalBytes, records } = parked);
      publish();
      return true;
    },
    /** Lets go of a parked stack that will never be resumed. */
    discard(parked: any) {
      parked?.stack?.forEach?.((entry: Entry) => entry.cleanup?.());
    },
    list: () => stack.map(entry => entry.label),
    stats: () => ({ entries: stack.length, bytes: totalBytes, maxBytes, estimated: false, byteKind: 'serialized' }),
  };
  PM.hist = H;
  PM.Memory?.register?.('history', {
    bytes: () => totalBytes,
    entries: () => stack.length,
    // Undo is durable document data, not a reconstructible render cache.
    // Release the optional lookup index without discarding any saved actions.
    pressure: () => records.clear(),
    trim: (target: number) => {
      maxBytes = Math.max(0, target);
      while (totalBytes > target && stack.length > 1) {
        // Redo patches depend on every preceding step. Discard the distant
        // redo tail before removing applied entries from the undo head.
        if (stack.length > idx + 1) {
          const removed = stack.pop()!;
          totalBytes -= removed.bytes;
          removed.cleanup?.();
          continue;
        }
        const removed = stack.shift()!;
        totalBytes -= removed.bytes;
        removed.cleanup?.();
        idx--;
      }
      idx = Math.max(-1, Math.min(idx, stack.length - 1));
    },
  });

  /* Takes remain intentional named project versions, separate from bounded Undo. */
  PM.takes = {
    all: () => PM.store.get('takes', []),
    save(label: any) {
      const list = PM.takes.all();
      list.unshift({ id: PM.uid('t'), label: label || 'Take ' + (list.length + 1), at: Date.now(), json: snap() });
      PM.store.set('takes', list.slice(0, 24));
      PM.bus.emit('takes');
      return list[0];
    },
    restore(id: any) {
      const take = PM.takes.all().find((item: any) => item.id === id);
      if (!take) return false;
      PM.hist.do('Restore ' + take.label, () => restore(take.json));
      return true;
    },
    drop(id: any) {
      PM.store.set('takes', PM.takes.all().filter((item: any) => item.id !== id));
      PM.bus.emit('takes');
    },
  };
}
