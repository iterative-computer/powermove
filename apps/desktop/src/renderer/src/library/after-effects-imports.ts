import { EXTENSION_ID } from '../../../shared/extensions';

interface ImportHost {
  store: { get(key: string, fallback?: unknown): unknown; set(key: string, value: unknown): void };
  bus?: { emit(event: string): void };
}

const IMPORTED_EXTENSIONS_KEY = 'afterEffectsExtensions';

/** Local provenance survives renames and restarts without saving Adobe source. */
export function afterEffectsExtensionIds(PM: ImportHost): Set<string> {
  const saved = PM.store.get(IMPORTED_EXTENSIONS_KEY, []);
  return new Set(Array.isArray(saved) ? saved.filter((id): id is string => typeof id === 'string' && EXTENSION_ID.test(id)) : []);
}

export function recordAfterEffectsExtensions(PM: ImportHost, changes: readonly { id: string; action: string }[]): void {
  const ids = afterEffectsExtensionIds(PM);
  for (const change of changes) {
    if (!EXTENSION_ID.test(change.id)) continue;
    if (change.action === 'removed') ids.delete(change.id);
    else if (change.action === 'created' || change.action === 'updated') ids.add(change.id);
  }
  PM.store.set(IMPORTED_EXTENSIONS_KEY, [...ids]);
  PM.bus?.emit('after-effects:imported');
}
