/* Ported from js/core/projects.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { bridge as hostBridge } from '../../kernel/bridge';
import { needsNewerReader } from '../../../../shared/reader-version';
import { writeDocument } from '../../engine-doc/write';
import { digestJSON } from './json-slices';

/**
 * The journal without what `patches` overwrites. A patch at a member of the
 * project itself (`['edits']`, `['revision']`) replaces everything earlier
 * patches wrote under that member, and the project is an object, so no write
 * between them can move that member elsewhere: replaying the journal gives
 * the same document without them. Every edit writes the whole provenance log
 * (`['edits']`), so without this each entry kept a copy of the log and every
 * edit rewrote all of them (docs/engine/perf.md). Entries stay, possibly with
 * no patches.
 */
export function supersede(entries: any[], patches: any[], changed?: (index: number) => void): any[] {
  const members = new Set<string>();
  for (const patch of patches) {
    if (Array.isArray(patch?.path) && patch.path.length === 1 && typeof patch.path[0] === 'string') members.add(patch.path[0]);
  }
  if (!members.size) return entries;
  entries.forEach((item, index) => {
    if (!Array.isArray(item?.patches)) return;
    const kept = item.patches.filter((patch: any) => !(Array.isArray(patch?.path) && patch.path.length >= 1 && members.has(patch.path[0])));
    if (kept.length !== item.patches.length) { item.patches = kept; changed?.(index); }
  });
  return entries;
}

export function install(PM: PMRegistry): void {
const R: any = {
  KEY: 'projects',          // [{id, name, at, thumb}]
  SLOT: 'project.',         // + id → serialized project
  STATE: 'projectState.',   // + id → window-owned editor/workspace session
  /* [id] for every project that has an editor window, in window order. The
     native side owns this list — only it can see every window — so nothing here
     writes it; the renderer reads it to know what is already open elsewhere. */
  openKey: 'openWindows',
  legacyOpenKey: 'openTabs', // the pre-window profiles' key, read once
  trashKey: 'projectTrash', // metadata for recoverable deletion; slots stay intact
  JOURNAL: 'projectJournal.',
};
const MAX_JOURNAL_BYTES = 4 * 1024 * 1024;
const CHECKPOINT_REVISIONS = 50;
const journalBytes = (value: any) => new TextEncoder().encode(JSON.stringify(value)).byteLength;
const clone = (value: any) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));

function applyPatches(root: any, patches: any[]) {
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

/** The journal as this window last wrote it (see the patch listener). */
type JournalState = { id: string; entries: any[]; sizes: number[] };
let written: JournalState | null = null;

function journal(id: any) {
  const value = id ? PM.store.get(R.JOURNAL + id, []) : [];
  return Array.isArray(value) ? value : [];
}

R.list = () => {
  const l = PM.store.get(R.KEY, null);
  return Array.isArray(l) ? l : [];
};

R.saveList = (list: any) => PM.store.set(R.KEY, list);

/** Accept both current save envelopes ({v, proj, ws}) and old bare projects. */
R.unwrap = (raw: any) => {
  if (!raw) return null;
  try { if (typeof raw === 'string') raw = JSON.parse(raw); } catch (e) { return null; }
  return raw && raw.proj && typeof raw.proj === 'object' ? raw.proj : raw;
};

/** Whether a project's slot is stored, without reading (and cloning) it. */
function slotExists(id: string): boolean {
  return typeof PM.store.has === 'function' ? PM.store.has(R.SLOT + id) : PM.store.get(R.SLOT + id, null) != null;
}

/*
 * Whole-project checkpoints (OPENWRITE-1). An open used to write the same
 * document five times: switchProject's put, the autosave it schedules, the
 * poster backfill's checkpoint, the session capture as the open finishes, and
 * the autosave after it. A large slot written from this very project object
 * is not written again blindly: the document is serialized in slices and
 * digested, off the critical path and without long tasks, and written
 * (synchronously, as a fresh atomic snapshot) only if its JSON differs, or if
 * something announced a change while it was read. A crash before that check
 * finishes recovers the slot, which then holds what the check found.
 *
 * Small slots are written directly, as before: their write costs less than
 * the check and is on disk sooner.
 */
const VERIFY_MIN_CHARS = 4 << 20;
type Slot = {
  project: object; chars: number; digest: Promise<string | null> | null;
  /** `assets` as the slot holds it (JSON text; undefined when absent). */
  assets: string | undefined;
  /** What the journal holds since: nothing, only `checkpointAssets` entries
   *  (the last one replacing `assets` with `journaledAssets`), or other entries. */
  journal: 'none' | 'assets' | 'other';
  journaledAssets?: string;
};
const slots = new Map<string, Slot>();
const checks = new Map<string, { project: object; again: boolean }>();
/* Announced document changes, so a check that read through one is repeated. */
let announced = 0;
for (const event of ['layers', 'project', 'library', 'assets', 'comps', 'history:project-patch']) PM.bus?.on?.(event, () => { announced++; });
const changeToken = () => `${announced}:${PM.animVersion?.() ?? 0}`;

function writeSlot(proj: any) {
  const envelope = { v: PM.version || '1.0.0', proj };
  if (PM.store.set(R.SLOT + proj.id, envelope) === false) throw new Error('Project storage is unavailable or full');
  PM.store.del(R.JOURNAL + proj.id);
  if (written?.id === proj.id) written = null;
  const digest = typeof PM.store.writtenDigest === 'function' ? PM.store.writtenDigest(R.SLOT + proj.id) : null;
  slots.set(proj.id, {
    project: proj, digest, chars: digest ? PM.store.writtenChars?.(R.SLOT + proj.id) ?? 0 : 0,
    assets: JSON.stringify(proj.assets), journal: 'none',
  });
}

/** The project as its slot holds it, if the journal only replaced `assets`. */
function asWritten(proj: any, slot: Slot): any {
  if (slot.journal === 'none') return proj;
  if (slot.assets === undefined) return null;
  return { ...proj, assets: JSON.parse(slot.assets) };
}

/** Whether the slot plus its journal is `proj` (checked against `proj` now); rewrites the slot otherwise. */
function checkSlot(proj: any): void {
  const id = proj.id;
  const running = checks.get(id);
  if (running && running.project === proj) { running.again = true; return; }
  const check = { project: proj, again: true };
  checks.set(id, check);
  const work = (async () => {
    try {
      for (let attempt = 0; check.again; attempt++) {
        check.again = false;
        const found = slots.get(id);
        // Deleted, renamed, or written again since: nothing left to check.
        if (!found || found.project !== proj || checks.get(id) !== check) return;
        const slot: Slot = found;
        const expected = slot.digest ? await slot.digest : null;
        const token = changeToken();
        const written = expected && slot.journal !== 'other' ? asWritten(proj, slot) : null;
        const actual = written ? (await digestJSON({ v: PM.version || '1.0.0', proj: written })).digest : null;
        if (slots.get(id) !== slot || checks.get(id) !== check) return;
        const same = actual !== null && actual === expected && slot.journal !== 'other'
          && (slot.journal === 'none' ? !journal(id).length : JSON.stringify(proj.assets) === slot.journaledAssets);
        if (same && token === changeToken()) continue;
        // Changes announced while it was read: read again (a few times at most).
        if (same && attempt < 2) { check.again = true; continue; }
        writeSlot(proj);
        return;
      }
    } finally { if (checks.get(id) === check) checks.delete(id); }
  })();
  PM.store.track?.(R.SLOT + id, work);
}

/** Upsert registry metadata for a project and persist its data. */
R.put = (proj: any, thumb: any) => {
  if (!proj || !proj.id) return;
  const meta: any = { id: proj.id, name: proj.name || 'Untitled', at: Date.now(), w: proj.w, h: proj.h };
  if (thumb) meta.thumb = thumb;
  /* A project written for a newer reader is stored once, as opened, so its
     tab and restart still work. Later writes keep only the library metadata:
     this build never autosaves a document it may not fully understand. */
  const frozen = needsNewerReader(proj) && slotExists(proj.id);
  R.upsertMeta(meta);
  PM.store.set(R.trashKey, R.trashList().filter((x: any) => x.id !== proj.id));
  if (frozen) return meta;
  /* Serialize the project passed to us, not whichever project happens to be
     active. This matters for rename/duplicate and keeps every slot canonical. */
  const slot = slots.get(proj.id);
  if (slot && slot.project === proj && slot.digest && slot.chars >= VERIFY_MIN_CHARS && slot.journal !== 'other'
    && (slot.journal === 'assets' || !journal(proj.id).length)) checkSlot(proj);
  else writeSlot(proj);
  return meta;
};

/**
 * Asset metadata changes outside typed history (a restore's poster backfill,
 * durations, renditions). They used to force a full checkpoint, as the journal
 * had no entry for them. Now they are one journal entry replacing `assets`
 * (about 20 KB for the large private project, instead of its 180 MB slot).
 */
R.checkpointAssets = (proj: any) => {
  if (!proj?.id) return;
  if (needsNewerReader(proj) || !slotExists(proj.id) || PM.proj?.id !== proj.id) { R.put(proj); return; }
  const slot = slots.get(proj.id), assets = JSON.stringify(proj.assets);
  const onlyAssets = !!slot && slot.project === proj && (slot.journal === 'assets' || slot.journal === 'none' && !journal(proj.id).length);
  append({
    projectId: proj.id, revision: Number(proj.revision) || 0,
    patches: [assets === undefined ? { path: ['assets'], exists: false } : { path: ['assets'], exists: true, value: JSON.parse(assets) }],
  });
  // `append` marks other entries; this one keeps the slot checkable.
  if (slot && slots.get(proj.id) === slot) {
    if (onlyAssets) { slot.journal = 'assets'; slot.journaledAssets = assets; }
    else slot.journal = 'other';
  }
};

/** Lightweight crash recovery: compact typed-history patches between checkpoints. */
R.recover = (proj: any, thumb: any) => {
  if (!proj?.id) return null;
  const meta: any = { id: proj.id, name: proj.name || 'Untitled', at: Date.now(), w: proj.w, h: proj.h };
  if (thumb) meta.thumb = thumb;
  R.upsertMeta(meta);
  PM.store.set(R.trashKey, R.trashList().filter((item: any) => item.id !== proj.id));
  const entries = journal(proj.id);
  if (!slotExists(proj.id) || !entries.length || journalBytes(entries) > MAX_JOURNAL_BYTES
      || Number(proj.revision || 0) % CHECKPOINT_REVISIONS === 0) {
    return R.put(proj, thumb);
  }
  return meta;
};

/** Update metadata only — never touches stored project data. */
R.upsertMeta = (meta: any) => {
  if (!meta || !meta.id) return;
  const list = R.list();
  const i = list.findIndex((x: any) => x.id === meta.id);
  if (i >= 0) {
    meta = { ...list[i], ...meta };
    if (!meta.thumb && list[i].thumb) meta.thumb = list[i].thumb;
    list[i] = meta;
  } else list.unshift(meta);
  R.saveList(list);
};

/** Rename one canonical project without ever writing the active composition
    into a different project's storage slot. Blank names keep the old name. */
R.rename = (id: any, name: any) => {
  if (!id) return null;
  const raw = R.get(id);
  const active = PM.proj && PM.proj.id === id ? PM.proj : null;
  const meta = R.list().find((x: any) => x.id === id);
  const previous = (active && active.name) || (raw && raw.name) || (meta && meta.name) || 'Untitled';
  const next = String(name == null ? '' : name).trim() || previous;
  if (active || raw) {
    // Persist the live document when active; the stored snapshot may predate
    // recent edits. Do not mutate the live name until the write succeeds.
    R.put({ ...(active || raw), name: next });
  } else if (meta) {
    R.upsertMeta({ ...meta, name: next, at: Date.now() });
  } else return null;
  /* Inactive projects do not emit the active-document autosave events. Mark
     their file association dirty here so the Projects screen and close guard
     never imply that a rename already reached the external .pmv file. */
  if (!active && next !== previous) {
    const file = PM.projectFileState?.(id);
    if (file) file.dirty = true;
  }
  if (active) writeDocument(PM, [['name']], () => { active.name = next; });
  PM.bus?.emit?.('projects:open');
  if (active) { PM.touch?.(); PM.bus?.emit?.('project'); }
  return next;
};

R.get = (id: any) => {
  if (!id) return null;
  let raw = null;
  try { raw = PM.store.get(R.SLOT + id, null); } catch (e) { }
  if (!raw) {
    // legacy: fall back to the single autosave slot for the most recent project
    const auto = PM.store.get('autosave', null);
    if (auto && auto.proj && auto.proj.id === id) return auto.proj;
    return null;
  }
  let project = R.unwrap(raw);
  if (!project) return null;
  project = clone(project);
  for (const entry of journal(id)) project = applyPatches(project, entry.patches);
  return project;
};

R.getState = (id: any, options: { history?: boolean } = {}) => {
  if (!id) return null;
  const history = options.history === false ? undefined : PM.store.get(`projectHistory.${id}`, undefined);
  const metadata = PM.store.get(`projectMeta.${id}`, null);
  const legacy = !metadata || options.history !== false && history === undefined
    ? PM.store.get(R.STATE + id, null, { omitHistory: options.history === false || history !== undefined }) : null;
  const state = metadata ? { ...legacy, ...metadata } : legacy;
  return history === undefined ? state : { ...state, history };
};
R.putState = (id: any, state: any) => {
  if (!id || !state || typeof state !== 'object') return;
  if (PM.store.separateHistory !== true) {
    const previous = state.history === undefined ? PM.store.get(R.STATE + id, null) : null;
    PM.store.set(R.STATE + id, { ...previous, ...state });
    return;
  }
  const { history, ...metadata } = state;
  if (history !== undefined) {
    if (PM.store.setAsync) void PM.store.setAsync(`projectHistory.${id}`, history);
    else PM.store.set(`projectHistory.${id}`, history);
  }
  // Keep the legacy envelope as a recovery fallback until the separate history
  // has reached disk. Small metadata updates never clone or overwrite it.
  PM.store.set(`projectMeta.${id}`, metadata);
};

/** Pure boot choice: content first (projects with a window, then registry),
    then a named empty project. Anonymous empty projects never win over the
    welcome demo. */
R.pickBoot = ({ open = R.openProjects(), metas = R.list(), get = R.get, getState = R.getState, legacy = null, taken = [] }: any = {}) => {
  /* A document belongs to one window at a time, so a second window must never
     boot into something its sibling already has open. */
  const spoken = new Set((Array.isArray(taken) ? taken : []).filter((id: any) => typeof id === 'string'));
  const free = (id: any) => typeof id === 'string' && !spoken.has(id);
  const active = open.filter(free)
    .map((id: string) => ({ id, at: Number(getState(id, { history: false })?.lastActiveAt) || 0 }))
    .filter((item: any) => item.at > 0).sort((a: any, b: any) => b.at - a.at);
  for (const item of active) {
    const project = R.unwrap(get(item.id));
    if (project && Array.isArray(project.layers)) return project;
  }
  const ids = [...new Set([...open, ...metas.map((m: any) => m.id)])].filter(free);
  let namedEmpty = null;
  for (const id of ids) {
    const p = R.unwrap(get(id));
    if (!p || typeof p !== 'object') continue;
    if (Array.isArray(p.layers) && p.layers.length) return p;
    if (!namedEmpty && p.name && p.name !== 'Untitled') namedEmpty = p;
  }
  const old = R.unwrap(legacy && legacy.proj ? legacy.proj : legacy);
  if (old && Array.isArray(old.layers) && old.layers.length && free(old.id) && !metas.some((m: any) => m.id === old.id)) return old;
  return namedEmpty;
};

R.remove = (id: any) => {
  slots.delete(id);
  R.saveList(R.list().filter((x: any) => x.id !== id));
  PM.store.set(R.trashKey, R.trashList().filter((x: any) => x.id !== id));
  try { PM.store.del(R.SLOT + id); } catch (e) { }
};

/* Recoverable project deletion. A trashed project keeps its storage slot until
   the user explicitly chooses Delete Forever. */
R.trashList = () => {
  const list = PM.store.get(R.trashKey, []);
  return Array.isArray(list) ? list : [];
};
R.trash = (id: any) => {
  const meta = R.list().find((x: any) => x.id === id);
  if (!meta) return false;
  R.saveList(R.list().filter((x: any) => x.id !== id));
  const trash = R.trashList().filter((x: any) => x.id !== id);
  trash.unshift({ ...meta, deletedAt: Date.now() });
  PM.store.set(R.trashKey, trash);
  return true;
};
R.restore = (id: any) => {
  const meta = R.trashList().find((x: any) => x.id === id);
  if (!meta || !R.get(id)) return false;
  PM.store.set(R.trashKey, R.trashList().filter((x: any) => x.id !== id));
  const clean = { ...meta, at: Date.now() }; delete clean.deletedAt;
  R.upsertMeta(clean);
  return true;
};
R.destroy = (id: any) => {
  slots.delete(id);
  PM.store.set(R.trashKey, R.trashList().filter((x: any) => x.id !== id));
  try { PM.store.del(R.SLOT + id); } catch (e) { }
  try { PM.store.del(R.STATE + id); } catch (e) { }
  for (const prefix of ['projectMeta.', 'projectHistory.', 'projectWorkspace.', 'documentHost.']) {
    try { PM.store.del(prefix + id); } catch (e) { }
  }
  try { PM.store.del(R.JOURNAL + id); } catch (e) { }
  if (written?.id === id) written = null;
  /* The agent's workspace and artifacts live exactly as long as the project.
     Imported media was copied into the project, so nothing it uses is lost. */
  void hostBridge()?.artifacts?.forget?.(id)?.catch?.((error: any) => console.warn('Agent workspace cleanup failed', error));
};

/* ── open windows ──────────────────────────────────────── */
/** Projects that currently have an editor window, this one included. */
R.openProjects = () => {
  const stored = PM.store.get(R.openKey, null);
  const list = Array.isArray(stored) ? stored : PM.store.get(R.legacyOpenKey, []);
  if (!Array.isArray(list)) return [];
  const known = new Set(R.list().map((m: any) => m?.id));
  return [...new Set(list)].filter(id => typeof id === 'string' && known.has(id));
};

PM.Projects = R;
PM.bus?.on?.('history:project-patch', (entry: any) => {
  if (!entry?.projectId || !Array.isArray(entry.patches) || !entry.patches.length) return;
  // Read-only (newer) projects keep no recovery journal; see R.put.
  if (PM.proj?.id === entry.projectId && needsNewerReader(PM.proj)) return;
  append(entry);
});
/** Appends one entry to a project's recovery journal. */
function append(entry: { projectId: string; revision: number; patches: any[] }) {
  const slot = slots.get(entry.projectId);
  if (slot) slot.journal = 'other';
  /* The journal this window wrote last, with each entry's size: an append
     neither reads the journal back from the store (a structured clone of all
     of it) nor serializes all of it again to measure it. */
  const cached = written;
  const state: JournalState = cached && cached.id === entry.projectId ? cached : (() => {
    const entries = journal(entry.projectId);
    return { id: entry.projectId, entries, sizes: entries.map(item => journalBytes(item)) };
  })();
  supersede(state.entries, entry.patches, index => { state.sizes[index] = journalBytes(state.entries[index]); });
  const item = { revision: Number(entry.revision) || 0, patches: entry.patches };
  state.entries.push(item);
  state.sizes.push(journalBytes(item));
  /* Keep the journal bounded even before the next debounced checkpoint
     (its JSON: brackets, the entries, and a comma between each two). */
  const bytes = 2 + state.sizes.reduce((sum, size) => sum + size, 0) + Math.max(0, state.sizes.length - 1);
  if (bytes > MAX_JOURNAL_BYTES) {
    written = null;
    if (PM.proj?.id === entry.projectId) R.put(PM.proj);
    return;
  }
  /* Serialized off the edit's task when the store can (the journal is a
     snapshot; a newer write supersedes a pending one, and a checkpoint's
     delete supersedes both). Cloning and posting it in the edit's task cost
     6 ms per edit at 1,000 layers (docs/engine/perf.md). */
  if (PM.store.setAsync) void PM.store.setAsync(R.JOURNAL + entry.projectId, state.entries.slice());
  else PM.store.set(R.JOURNAL + entry.projectId, state.entries);
  written = state;
}
}

// Registry methods hold no editor session state. Refresh them in place during
// development without propagating a name-workflow edit into a window reload.
if (import.meta.hot) {
  import.meta.hot.accept(next => {
    if (next && window.PM) {
      const PM = window.PM;
      next.install(PM);
      if (PM.proj?.id && PM.hist?.export) {
        PM.Projects.putState(PM.proj.id, { ...PM.Projects.getState(PM.proj.id, { history: false }),
          history: PM.hist.export({ copy: false }), workspace: PM.WS?.snapshot(), time: PM.time });
      }
    }
  });
}
