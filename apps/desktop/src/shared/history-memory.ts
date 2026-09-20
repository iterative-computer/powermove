/** Lossless storage for immutable undo records. Live project values stay mutable. */
const FORMAT = 'powermove-history-dictionary';
const MAX_BUCKETS = 16_384;
type RecordValue = Record<string, any>;
const record = (value: any): value is RecordValue => value !== null && typeof value === 'object' && !Array.isArray(value);
export const isHistoryDictionary = (value: any): boolean => record(value) && value.format === FORMAT;
const history = (value: any): boolean => record(value) && value.version === 1 && Array.isArray(value.entries)
  && value.entries.every((entry: any) => record(entry) && Array.isArray(entry.forward) && Array.isArray(entry.backward));
const recordArray = (patch: any): boolean => record(patch) && patch.exists === true && Array.isArray(patch.path) && patch.path.length === 1
  && (patch.path[0] === 'layers' || patch.path[0] === 'edits') && Array.isArray(patch.value);

/** Weak ownership prevents removed projects/Redo branches living in this index. */
export class HistoryRecords {
  private buckets = new Map<number, WeakRef<object>[]>();
  private known = new WeakSet<object>();
  private recent = new Map<string, WeakRef<object>>();

  intern(value: any): any {
    if (!value || typeof value !== 'object' || this.known.has(value)) return value;
    const json = JSON.stringify(value);
    // Structural edits usually leave almost every layer unchanged. Check its
    // latest immutable record before hashing the whole record again.
    const id = typeof value.id === 'string' ? value.id : undefined;
    const recent = id === undefined ? undefined : this.recent.get(id)?.deref();
    if (recent && JSON.stringify(recent) === json) return recent;
    let hash = 2166136261;
    for (let i = 0; i < json.length; i++) hash = Math.imul(hash ^ json.charCodeAt(i), 16777619);
    const live: WeakRef<object>[] = [];
    let match: object | undefined;
    for (const ref of this.buckets.get(hash) ?? []) {
      const candidate = ref.deref();
      if (!candidate) continue;
      live.push(ref);
      // Hashes only select a bucket: content equality is always required.
      if (!match && JSON.stringify(candidate) === json) match = candidate;
    }
    const canonical = match ?? value;
    if (id !== undefined) {
      this.recent.set(id, new WeakRef(canonical));
      if (this.recent.size > MAX_BUCKETS) this.recent.delete(this.recent.keys().next().value!);
    }
    if (match) return match;
    this.known.add(value);
    live.push(new WeakRef(value));
    this.buckets.set(hash, live);
    if (this.buckets.size > MAX_BUCKETS) this.buckets.delete(this.buckets.keys().next().value!);
    return value;
  }

  sharePatches(patches: any[]): void {
    for (const patch of patches) if (recordArray(patch)) {
      for (let i = 0; i < patch.value.length; i++) patch.value[i] = this.intern(patch.value[i]);
    }
  }

  share(value: any): any {
    if (history(value)) for (const entry of value.entries) {
      this.sharePatches(entry.forward); this.sharePatches(entry.backward);
    }
    return value;
  }

  clear(): void { this.buckets.clear(); this.recent.clear(); this.known = new WeakSet(); }
}

/** Call only with owned, immutable history; packing never changes its schema. */
export function packHistory(value: any): any {
  if (!history(value)) return value;
  const records: any[] = [], ids = new Map<any, number>();
  const pack = (patch: any) => {
    if (!recordArray(patch)) return { patch };
    const { value: values, ...metadata } = patch;
    return { patch: metadata, refs: values.map((item: any) => {
      if (!ids.has(item)) { ids.set(item, records.length); records.push(item); }
      return ids.get(item);
    }) };
  };
  const saved = { ...value, entries: value.entries.map((entry: any) => ({ ...entry,
    forward: entry.forward.map(pack), backward: entry.backward.map(pack),
  })) };
  if (!records.length) return value;
  return { format: FORMAT, version: 1, records, history: saved };
}

/** Reject invalid references instead of silently reopening with incomplete Undo. */
export function unpackHistory(value: any): any {
  if (!record(value) || value.format !== FORMAT) return value;
  if (value.version !== 1 || !Array.isArray(value.records) || !history(value.history)) throw new Error('Invalid history dictionary');
  const unpack = (item: any) => {
    if (!record(item) || !record(item.patch)) throw new Error('Invalid history patch');
    if (!Object.hasOwn(item, 'refs')) return item.patch;
    if (!Array.isArray(item.refs) || !Array.isArray(item.patch.path) || item.patch.path.length !== 1
      || !['layers', 'edits'].includes(item.patch.path[0]) || item.patch.exists !== true
      || item.refs.some((id: any) => !Number.isSafeInteger(id) || id < 0 || id >= value.records.length)) throw new Error('Invalid history reference');
    return { ...item.patch, value: item.refs.map((id: number) => value.records[id]) };
  };
  return { ...value.history, entries: value.history.entries.map((entry: any) => ({ ...entry,
    forward: entry.forward.map(unpack), backward: entry.backward.map(unpack),
  })) };
}

export function packStoredHistory(key: string, value: any, records: HistoryRecords): any {
  if (key.startsWith('projectHistory.')) return packHistory(records.share(unpackHistory(value)));
  if (key.startsWith('projectState.') && record(value) && value.history !== undefined) {
    return { ...value, history: packHistory(records.share(unpackHistory(value.history))) };
  }
  return value;
}

export function unpackStoredHistory(key: string, value: any): any {
  if (key.startsWith('projectHistory.')) return unpackHistory(value);
  if (key.startsWith('projectState.') && record(value) && value.history !== undefined) {
    return { ...value, history: unpackHistory(value.history) };
  }
  return value;
}
