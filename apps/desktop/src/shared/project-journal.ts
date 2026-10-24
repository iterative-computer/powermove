/*
 * A project's recovery journal (SAVE-1, docs/engine/host-processes.md).
 *
 * Main's store engine owns a project's durability (main/project-durability.ts).
 * It gives every slot write and every journal entry the next number of one
 * per-project sequence (`seq`), in the order they reached it, and stamps each
 * slot with a checkpoint id (`cp`) and each entry with the `cp` of the slot
 * that was on disk when it was appended:
 *
 *   project.<id>         {"cp":"…","seq":7,"v":"…","proj":{…}}
 *   projectJournal.<id>  [{"revision":…,"patches":[…],"seq":8,"cp":"…"}, …]
 *
 * A slot holds every edit made before it was posted, and an entry is an edit
 * made after everything with a lower number. So a slot is recovered with
 * exactly the entries numbered after it, in order, whichever slot and journal
 * a crash left on disk. Data written before SAVE-1 (no `seq`) replays as it
 * always did: every entry, or those tagged with the slot's `cp`.
 */

export type JournalEntry = { revision?: number; patches?: any[]; seq?: number; cp?: string };

const clone = (value: any) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));

/** The members a list of patches writes whole (a patch at a member of the project itself). */
function members(patches: any[]): Set<string> {
  const out = new Set<string>();
  for (const patch of patches) {
    if (Array.isArray(patch?.path) && patch.path.length === 1 && typeof patch.path[0] === 'string') out.add(patch.path[0]);
  }
  return out;
}

const writesUnder = (written: Set<string>) => (patch: any) => Array.isArray(patch?.path) && patch.path.length >= 1 && written.has(patch.path[0]);

/**
 * The journal without what `patches` overwrites, in place. A patch at a member
 * of the project itself (`['edits']`, `['revision']`) replaces everything
 * earlier patches wrote under that member, and the project is an object, so no
 * write between them can move that member elsewhere: replaying the journal
 * gives the same document without them. Every edit writes the whole provenance
 * log (`['edits']`), so without this each entry kept a copy of the log
 * (docs/engine/perf.md). Entries stay, possibly with no patches. Every replay
 * is a suffix of the journal (see `entriesAfter`), so a superseded patch is
 * never needed: the entry that overwrites it is replayed whenever it is.
 */
export function supersede(entries: any[], patches: any[], changed?: (index: number) => void): any[] {
  const written = members(patches);
  if (!written.size) return entries;
  const overwritten = writesUnder(written);
  entries.forEach((item, index) => {
    if (!Array.isArray(item?.patches)) return;
    const kept = item.patches.filter((patch: any) => !overwritten(patch));
    if (kept.length !== item.patches.length) { item.patches = kept; changed?.(index); }
  });
  return entries;
}

/** `supersede` without changing `entries` or its items. */
export function superseded(entries: JournalEntry[], patches: any[]): JournalEntry[] {
  const written = members(patches);
  if (!written.size) return entries.slice();
  const overwritten = writesUnder(written);
  return entries.map(item => {
    if (!Array.isArray(item?.patches) || !item.patches.some(overwritten)) return item;
    return { ...item, patches: item.patches.filter(patch => !overwritten(patch)) };
  });
}

/** The journal entries that follow a slot, in order. */
export function entriesAfter(slot: { seq?: unknown; cp?: unknown } | null | undefined, journal: unknown): JournalEntry[] {
  const entries = (Array.isArray(journal) ? journal : []).filter((entry): entry is JournalEntry => !!entry && typeof entry === 'object');
  const slotSeq = typeof slot?.seq === 'number' ? slot.seq : null;
  const slotCp = typeof slot?.cp === 'string' ? slot.cp : null;
  if (slotSeq !== null) {
    // Unnumbered entries were journaled before any numbered slot was written.
    return entries.filter(entry => typeof entry.seq === 'number' && entry.seq > slotSeq).sort((a, b) => (a.seq as number) - (b.seq as number));
  }
  // A slot written before SAVE-1, or by an older build: every entry, as before.
  return entries.filter(entry => slotCp === null || typeof entry.cp !== 'string' || entry.cp === slotCp);
}

export function applyPatches(root: any, patches: any[] | undefined): any {
  let next = root;
  for (const patch of patches || []) {
    if (!Array.isArray(patch?.path)) continue;
    if (!patch.path.length) { next = patch.exists ? clone(patch.value) : undefined; continue; }
    let parent = next;
    for (let index = 0; index < patch.path.length - 1; index++) {
      parent = parent?.[patch.path[index]];
      if (parent == null) break;
    }
    if (parent == null) continue;
    const key = patch.path.at(-1);
    if (patch.exists) parent[key] = clone(patch.value);
    else if (Array.isArray(parent) && typeof key === 'number') parent.splice(key, 1);
    else delete parent[key];
  }
  return next;
}

/** A project as a restart recovers it: its slot (envelope or bare project) and the entries after it. */
export function recoverProject(raw: unknown, journal: unknown): any {
  if (!raw || typeof raw !== 'object') return null;
  const envelope = raw as Record<string, any>;
  let project = envelope.proj && typeof envelope.proj === 'object' ? envelope.proj : envelope;
  project = clone(project);
  for (const entry of entriesAfter(envelope, journal)) project = applyPatches(project, entry.patches);
  return project;
}
