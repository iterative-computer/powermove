/*
 * Reactive mirror of the extension records for the Mods panel.
 *
 * The loader owns the truth (records come from main over the bridge); this
 * module is the read model. `$state.raw` on the array keeps updates to
 * reference swaps — records are replaced wholesale, never mutated in place, so
 * a panel deriving over `extensionsStore.records` re-runs exactly once per
 * refresh instead of once per field.
 */
import type { ExtensionHealth, ExtensionRecord } from '../../../shared/extensions';

class ExtensionsStore {
  records: ExtensionRecord[] = $state.raw([]);
  activeIds: string[] = $state.raw([]);

  get byId(): Map<string, ExtensionRecord> {
    return new Map(this.records.map((record) => [record.id, record]));
  }
}

export const extensionsStore = new ExtensionsStore();

export function setRecords(records: ExtensionRecord[]): void {
  extensionsStore.records = [...records];
}

export function records(): ExtensionRecord[] {
  return extensionsStore.records;
}

export function recordFor(id: string): ExtensionRecord | undefined {
  return extensionsStore.records.find((record) => record.id === id);
}

/** Replace one record in place (by id), keeping list order stable. */
export function patchRecord(id: string, patch: Partial<ExtensionRecord>): ExtensionRecord | undefined {
  let next: ExtensionRecord | undefined;
  extensionsStore.records = extensionsStore.records.map((record) => {
    if (record.id !== id) return record;
    next = { ...record, ...patch };
    return next;
  });
  return next;
}

export function setHealth(id: string, health: ExtensionHealth): void {
  patchRecord(id, { health });
}

export function setActiveIds(ids: string[]): void {
  extensionsStore.activeIds = [...ids];
}

export function activeIds(): string[] {
  return extensionsStore.activeIds;
}

/** Test seam. */
export function resetExtensionsStore(): void {
  extensionsStore.records = [];
  extensionsStore.activeIds = [];
}
