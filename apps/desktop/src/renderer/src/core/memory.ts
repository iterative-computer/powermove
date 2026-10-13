import { MEMORY_KINDS, type MemoryEntry, type MemoryKind } from './types/project';

// Project memory is the short, interpreted layer over the edit log: why the
// video looks as it does. Bounded so it can be read at the start of every run.
export const MAX_MEMORY_ENTRIES = 100;
export const MAX_MEMORY_TEXT = 600;
export const MAX_MEMORY_WHY = 600;
const RECENT_IN_INDEX = 8;
/** One of these exists at a time; adding another edits the current one. */
const SINGLETON: readonly MemoryKind[] = ['goal'];
/** Always shown in full in the index, whatever their age. */
const STANDING: readonly MemoryKind[] = ['goal', 'constraint'];

const isKind = (value: unknown): value is MemoryKind => (MEMORY_KINDS as readonly unknown[]).includes(value);
const clip = (value: unknown, max: number): string => typeof value === 'string' ? value.trim().slice(0, max) : '';
const time = (value: unknown, fallback: number): number => typeof value === 'number' && Number.isFinite(value) ? value : fallback;

/** On by default; only an explicit false turns it off. */
export const memoryEnabled = (project: { memoryEnabled?: boolean } | null | undefined): boolean => project?.memoryEnabled !== false;

export const MEMORY_DISABLED_MESSAGE = 'Project memory is turned off for this project. The user can turn it on in Settings > Memory.';

/** Loads untrusted saved data; anything malformed is dropped rather than repaired. */
export function sanitizeMemory(raw: unknown, now = Date.now()): MemoryEntry[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  const out: MemoryEntry[] = [];
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue;
    const source = item as Record<string, unknown>;
    const text = clip(source.text, MAX_MEMORY_TEXT);
    const id = typeof source.id === 'string' ? source.id.slice(0, 80) : '';
    if (!id || !text || !isKind(source.kind) || seen.has(id)) continue;
    seen.add(id);
    const why = clip(source.why, MAX_MEMORY_WHY);
    const createdAt = time(source.createdAt, now);
    out.push({
      id, kind: source.kind, text, ...(why ? { why } : {}),
      author: source.author === 'user' ? 'user' : 'agent',
      createdAt, updatedAt: time(source.updatedAt, createdAt),
      ...(source.resolved === true ? { resolved: true } : {})
    });
  }
  return out.slice(-MAX_MEMORY_ENTRIES);
}

export type MemoryOperation =
  | { operation: 'add'; kind: MemoryKind; text: string; why?: string; resolved?: boolean }
  | { operation: 'update'; id: string; text?: string; why?: string; kind?: MemoryKind; resolved?: boolean }
  | { operation: 'remove'; id: string };

export interface MemoryContext { now: number; uid: () => string; author: 'agent' | 'user' }

/** Applies one write and returns the new list; the input is never mutated. */
export function applyMemoryOperation(entries: readonly MemoryEntry[], op: MemoryOperation, ctx: MemoryContext): { entries: MemoryEntry[]; entry: MemoryEntry | null } {
  if (op.operation === 'remove') {
    if (!entries.some(entry => entry.id === op.id)) throw new Error(`No memory entry ${op.id}. Use project_memory list to see the ids.`);
    return { entries: entries.filter(entry => entry.id !== op.id), entry: null };
  }
  const patch = (current: MemoryEntry, change: { text?: string; why?: string; kind?: MemoryKind; resolved?: boolean }): MemoryEntry => {
    const text = change.text === undefined ? current.text : clip(change.text, MAX_MEMORY_TEXT);
    if (!text) throw new Error('Memory text cannot be empty.');
    const why = change.why === undefined ? current.why : clip(change.why, MAX_MEMORY_WHY);
    const { resolved: _resolved, why: _why, ...rest } = current;
    const resolved = change.resolved ?? current.resolved;
    return { ...rest, kind: change.kind ?? current.kind, text, ...(why ? { why } : {}), ...(resolved ? { resolved: true } : {}), updatedAt: ctx.now };
  };
  if (op.operation === 'update') {
    const current = entries.find(entry => entry.id === op.id);
    if (!current) throw new Error(`No memory entry ${op.id}. Use project_memory list to see the ids.`);
    if (op.kind && SINGLETON.includes(op.kind) && op.kind !== current.kind && entries.some(entry => entry.kind === op.kind)) {
      throw new Error(`The project already has a ${op.kind}. Update that entry instead.`);
    }
    const next = patch(current, op);
    return { entries: entries.map(entry => entry.id === op.id ? next : entry), entry: next };
  }
  if (!isKind(op.kind)) throw new Error(`kind must be one of ${MEMORY_KINDS.join(', ')}.`);
  const text = clip(op.text, MAX_MEMORY_TEXT);
  if (!text) throw new Error('Memory text cannot be empty.');
  // A project has one goal, so a second one edits the first instead of contradicting it.
  const existing = SINGLETON.includes(op.kind) ? entries.find(entry => entry.kind === op.kind) : undefined;
  if (existing) {
    const next = patch(existing, { text, why: op.why });
    return { entries: entries.map(entry => entry === existing ? next : entry), entry: next };
  }
  const why = clip(op.why, MAX_MEMORY_WHY);
  const entry: MemoryEntry = {
    id: ctx.uid(), kind: op.kind, text, ...(why ? { why } : {}), author: ctx.author,
    createdAt: ctx.now, updatedAt: ctx.now, ...(op.resolved ? { resolved: true } : {})
  };
  return { entries: trim([...entries, entry]), entry };
}

/** Over the cap, drop the oldest entry that is not standing guidance, preferring finished ones. */
function trim(entries: MemoryEntry[]): MemoryEntry[] {
  const out = entries.slice();
  while (out.length > MAX_MEMORY_ENTRIES) {
    const droppable = (entry: MemoryEntry) => !STANDING.includes(entry.kind);
    const index = out.findIndex(entry => droppable(entry) && entry.resolved);
    const fallback = index >= 0 ? index : out.findIndex(droppable);
    out.splice(fallback >= 0 ? fallback : 0, 1);
  }
  return out;
}

export interface MemoryIndex {
  total: number;
  goal: MemoryEntry | null;
  constraints: MemoryEntry[];
  /** Unfinished work the next run should pick up. */
  open: MemoryEntry[];
  /** The latest decisions, rejected ideas and feedback; older ones are in project_memory list. */
  recent: MemoryEntry[];
}

/** What every run reads first: standing guidance in full, plus only the latest history. */
export function memoryIndex(entries: readonly MemoryEntry[] | undefined, recent = RECENT_IN_INDEX): MemoryIndex {
  const all = entries ?? [];
  const history = all.filter(entry => !STANDING.includes(entry.kind) && entry.kind !== 'open').sort((a, b) => b.updatedAt - a.updatedAt);
  return {
    total: all.length,
    goal: all.find(entry => entry.kind === 'goal') ?? null,
    constraints: all.filter(entry => entry.kind === 'constraint'),
    open: all.filter(entry => entry.kind === 'open' && !entry.resolved),
    recent: history.slice(0, recent)
  };
}

export function listMemory(entries: readonly MemoryEntry[] | undefined, filter: { kind?: MemoryKind; limit?: number } = {}): MemoryEntry[] {
  const limit = Math.max(1, Math.min(MAX_MEMORY_ENTRIES, Math.trunc(filter.limit ?? MAX_MEMORY_ENTRIES)));
  return (entries ?? []).filter(entry => !filter.kind || entry.kind === filter.kind).sort((a, b) => b.updatedAt - a.updatedAt).slice(0, limit);
}
