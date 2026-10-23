/** Workers keep synchronous preference views. Apply their library-index deltas
 * against the current host value so one project's autosave cannot erase a
 * project another worker created or renamed after that view was seeded. */
import type { Store } from '../../main/storage';
import type { DocumentHostServices } from '../../engine/document-host/protocol';

const INDEXES = new Set(['projects', 'projectTrash']);
const entries = (value: unknown): Array<Record<string, unknown> & { id: string }> => Array.isArray(value)
  ? value.filter(item => item && typeof item.id === 'string') : [];
export function profileStore(store: Store, changed?: (keys: string[]) => void): DocumentHostServices['store'] {
  let writes: Promise<unknown> = Promise.resolve();
  const queue = (write: () => Promise<void>): Promise<void> => {
    const result = writes.then(write); writes = result.catch(() => {}); return result;
  };
  return {
    read: async key => { await writes; const raw = store.getSerialized?.(key); return raw == null ? store.snapshot()[key] ?? null : JSON.parse(raw); },
    snapshot: async () => { await writes; return Object.fromEntries(Object.entries(store.snapshot()).filter(([key]) => !/^(project|projectState|projectHistory|projectJournal|agentThreads|agentAttachment|documentHost)\./.test(key))); },
    write: (key, value, base) => queue(async () => {
      if (INDEXES.has(key) && base !== undefined && Array.isArray(value)) {
        const previous = new Map(entries(base).map(item => [item.id, JSON.stringify(item)]));
        const incoming = entries(value), ids = new Set(incoming.map(item => item.id));
        const removed = new Set([...previous.keys()].filter(id => !ids.has(id)));
        const changed = incoming.filter(item => previous.get(item.id) !== JSON.stringify(item));
        const changedIds = new Set(changed.map(item => item.id));
        value = [...changed, ...entries(store.snapshot()[key]).filter(item => !removed.has(item.id) && !changedIds.has(item.id))];
      }
      await store.set(key, value); await store.flushAll(); changed?.([key]);
    }),
    remove: key => queue(async () => { await store.delete(key); await store.flushAll(); changed?.([key]); })
  };
}
