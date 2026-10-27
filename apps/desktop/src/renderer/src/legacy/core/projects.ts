/* Ported from js/core/projects.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { bridge as hostBridge } from '../../kernel/bridge';
import { needsNewerReader } from '../../../../shared/reader-version';
import { writeDocument } from '../../engine-doc/write';
import { parseAsync } from './parse-async';
import { yieldTask } from './yield-task';
import { digestJSON, serializeInSlices } from './json-slices';
import { projectTexts } from './project-text';
import { applyPatches, entriesAfter, recoverProject, supersede } from '../../../../shared/project-journal';

export { supersede };

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
/* The store's limit for a journal value (LIMITS.storeValueBytes): past it main refuses entries. */
const STORE_JOURNAL_BYTES = 32 * 1024 * 1024;
/* A periodic checkpoint is serialized in slices only while the journal since
   the last one is at most this long (half of main's journal limit). */
const SLICED_JOURNAL_BYTES = 4 * MAX_JOURNAL_BYTES;

/** The journal entries this window appended since its last checkpoint, with each one's size (see `append`). */
type JournalState = { id: string; entries: any[]; sizes: number[] };
let written: JournalState | null = null;
/* SAVE-1: with a store that keeps journals (main's, project-durability.ts),
   the window only appends entries and posts slots; main decides what is on
   disk. Without one (tests, hosts without main) the window writes the whole
   journal and deletes it at a checkpoint, as before. */
const mainKeepsJournal = () => typeof PM.store.appendJournal === 'function';

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
/* Projects whose slot or journal write failed (refused, or not persisted).
   Main keeps the older slot and every journal entry since (SAVE-1), so no
   edit already on disk is lost; this window's next checkpoint writes rather
   than checks, and its next autosave checkpoints even when it journaled
   since, until a slot write lands. */
const unsaved = new Set<string>();
/* Projects whose journal main refused (it reached the store's limit while
   slot writes fail). Main then refuses every later entry until a slot
   holding all of them lands (project-durability.ts); this window keeps
   sending them, so main knows the last one, but stops keeping its own copy
   and checkpointing at the bound. The edits stay in the document and the
   save error stays reported. It ends when a flush finds the journal no
   longer failing (storage:saved), not when a checkpoint is merely posted. */
const journalFull = new Set<string>();
/* A closed journal reopens only when a slot posted after the last refused
   edit has landed. Waiting for autosave (its debounce grows to a large
   project's last write time, up to 60 s) kept "not saved" up for minutes; the
   project is checkpointed shortly after the last refused edit instead.
   A checkpoint of a large project costs about a second of serialization on
   this thread and its whole size in IPC, and every refused edit after it was
   posted moves main's close point past it. So one is in flight at a time
   (posted, then a flush until it is on disk); refused edits meanwhile are
   covered by the next one (trailing), whose timer each refused edit
   restarts. While the journal stays closed after one landed, the wait grows
   (doubling, and at least four times the last checkpoint's own cost): up to
   CLOSED_CHECKPOINT_MAX_MS while edits keep being refused, and up to
   autosave's own ceiling when none was and the slot write itself failed (a
   full disk). It starts again at CLOSED_CHECKPOINT_MS once the journal
   reopens. It ends only when the journal reopens or the project
   leaves the screen (its switch checkpoints it), so a closed journal still
   converges to a landed slot. */
const CLOSED_CHECKPOINT_MS = 250;
const CLOSED_CHECKPOINT_MAX_MS = 8_000, CLOSED_RETRY_MAX_MS = 60_000;
/** `refused`: an edit was refused since the checkpoint in flight (or the last one) was posted. */
type ClosedCheckpoint = { timer?: ReturnType<typeof setTimeout>; running: boolean; refused: boolean; delay: number };
const closedCheckpoints = new Map<string, ClosedCheckpoint>();
function checkpointSoon(id: string) {
  let pending = closedCheckpoints.get(id);
  if (!pending) closedCheckpoints.set(id, pending = { running: false, refused: false, delay: CLOSED_CHECKPOINT_MS });
  pending.refused = true; pending.delay = Math.min(pending.delay, CLOSED_CHECKPOINT_MAX_MS);
  // One in flight: the next, after it lands, covers this edit too.
  if (pending.running) return;
  if (pending.timer !== undefined) clearTimeout(pending.timer);
  const scheduled = pending;
  pending.timer = setTimeout(() => { void closedCheckpoint(id, scheduled); }, pending.delay);
}
async function closedCheckpoint(id: string, pending: ClosedCheckpoint) {
  pending.timer = undefined;
  if (closedCheckpoints.get(id) !== pending) return;
  if (PM.proj?.id !== id || !journalFull.has(id)) { closedCheckpoints.delete(id); return; }
  pending.running = true; pending.refused = false;
  const began = Date.now();
  try { R.put(PM.proj); } catch (error) { console.warn('Project checkpoint failed', error); }
  const cost = Date.now() - began;
  // On disk, or failed: a flush answers which keys are still failing (storage:saved reopens).
  if (typeof PM.store.flush === 'function') { try { await PM.store.flush(); } catch { /* reported by the store */ } }
  pending.running = false;
  if (closedCheckpoints.get(id) !== pending) return;
  if (PM.proj?.id !== id || !journalFull.has(id) || typeof PM.store.flush !== 'function') { closedCheckpoints.delete(id); return; }
  // Still closed: an edit refused after it was posted, or its write failed. Back off, then once more.
  pending.delay = Math.min(pending.refused ? CLOSED_CHECKPOINT_MAX_MS : CLOSED_RETRY_MAX_MS, Math.max(pending.delay * 2, cost * 4));
  pending.timer = setTimeout(() => { void closedCheckpoint(id, pending); }, pending.delay);
}
PM.bus?.on?.('storage:saved', (key: any) => {
  if (typeof key === 'string' && key.startsWith(R.JOURNAL)) journalFull.delete(key.slice(R.JOURNAL.length));
});
PM.bus?.on?.('storage:error', (event: any) => {
  const key = typeof event?.key === 'string' ? event.key : '';
  const id = key.startsWith(R.SLOT) ? key.slice(R.SLOT.length) : key.startsWith(R.JOURNAL) ? key.slice(R.JOURNAL.length) : '';
  if (!id) return;
  slots.delete(id);
  unsaved.add(id);
});

/* Announced document changes, so a check that read through one is repeated. */
let announced = 0;
for (const event of ['layers', 'project', 'library', 'assets', 'comps', 'history:project-patch']) PM.bus?.on?.(event, () => { announced++; });
const changeToken = () => `${announced}:${PM.animVersion?.() ?? 0}`;

/* The JSON this window just wrote for a project, for the rest of the writing
   task while nothing announced a change: opening and switching projects
   fingerprint the document right after checkpointing it. */
let justWritten: { project: object; token: string; text: string } | null = null;
R.writtenJSON = (proj: any): string | null => {
  const entry = justWritten;
  if (!entry || entry.project !== proj || entry.token !== changeToken() || typeof proj?.toJSON === 'function') return null;
  const prefix = `{"v":${JSON.stringify(PM.version || '1.0.0')},"proj":`;
  return entry.text.startsWith(prefix) && entry.text.endsWith('}') ? entry.text.slice(prefix.length, -1) : null;
};

/** The slot's text and the engine's canonical text of a project, from one
 *  serialization in slices (project-text.ts); null when it has to be
 *  serialized as a whole. */
R.texts = (proj: any) => projectTexts(PM.version || '1.0.0', proj);

/** `serialized`: the envelope's JSON text, already made (`checkpointInSlices`). */
function writeSlot(proj: any, serialized?: string) {
  const envelope = { v: PM.version || '1.0.0', proj };
  slotWrites.set(proj.id, (slotWrites.get(proj.id) || 0) + 1);
  checkpointsDue.delete(proj.id);
  const stored = serialized !== undefined && PM.store.setText?.(R.SLOT + proj.id, serialized) || PM.store.set(R.SLOT + proj.id, envelope);
  if (stored === false) throw new Error('Project storage is unavailable or full');
  unsaved.delete(proj.id);
  const text = typeof PM.store.writtenText === 'function' ? PM.store.writtenText(R.SLOT + proj.id) : null;
  if (text) {
    const entry = justWritten = { project: proj, token: changeToken(), text };
    queueMicrotask(() => { if (justWritten === entry) justWritten = null; });
  }
  // Main shrinks its journal itself once this slot is on disk.
  if (!mainKeepsJournal()) PM.store.del(R.JOURNAL + proj.id);
  if (written?.id === proj.id) written = null;
  const digest = typeof PM.store.writtenDigest === 'function' ? PM.store.writtenDigest(R.SLOT + proj.id) : null;
  slots.set(proj.id, {
    project: proj, digest, chars: digest ? PM.store.writtenChars?.(R.SLOT + proj.id) ?? 0 : 0,
    assets: JSON.stringify(proj.assets), journal: 'none',
  });
}

/** Whether this window journaled anything for `id` since its last checkpoint. */
const sinceCheckpoint = (id: string): any[] => { const state = written; return state && state.id === id ? state.entries : []; };
const journaledSince = (id: string) => mainKeepsJournal() ? sinceCheckpoint(id).length > 0 : journal(id).length > 0;

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
          && (slot.journal === 'none' ? !journaledSince(id) : JSON.stringify(proj.assets) === slot.journaledAssets);
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

/** Upsert registry metadata for a project and persist its data.
 *  `serialized`: the slot's text, already made (projectTexts, for an open). */
R.put = (proj: any, thumb: any, serialized?: string) => {
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
  const slot = unsaved.has(proj.id) ? undefined : slots.get(proj.id);
  if (slot && slot.project === proj && slot.digest && slot.chars >= VERIFY_MIN_CHARS && slot.journal !== 'other'
    && (slot.journal === 'assets' || !journaledSince(proj.id))) checkSlot(proj);
  else writeSlot(proj, serialized);
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
  const onlyAssets = !!slot && slot.project === proj && (slot.journal === 'assets' || slot.journal === 'none' && !journaledSince(proj.id));
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

/*
 * Periodic checkpoints (SAVE-1): every CHECKPOINT_REVISIONS revisions, or
 * once the journal passes MAX_JOURNAL_BYTES, the slot is written again so the
 * journal stays short. Nothing waits on one: main keeps every entry until a
 * slot holding it lands. A large project's slot was serialized in one task
 * (0.5 to 1 s for the private 180 MB project, every 50 edits); it is now
 * serialized in slices and posted only if nothing changed while it was read
 * (announced changes, document writes, the engine barrier's epoch, an open
 * history transaction, another slot write). Then it is the document as it
 * was when posted, with exactly the entries posted before it, as main
 * numbers it. Otherwise it is due: the next autosave tries again.
 */
const slotWrites = new Map<string, number>();
const checkpointsDue = new Set<string>();
const slicing = new Set<string>();
const sliceToken = (id: string) => `${changeToken()}:${PM.EngineDoc?.barrier?.changeEpoch ?? ''}:${slotWrites.get(id) || 0}`;
async function checkpointInSlices(proj: any) {
  const id = proj.id;
  checkpointsDue.add(id);
  if (slicing.has(id) || PM.hist?.busy?.()) return;
  slicing.add(id);
  const work = (async () => {
    try {
      const token = sliceToken(id), parts: string[] = [];
      await serializeInSlices({ v: PM.version || '1.0.0', proj }, text => { parts.push(text); });
      // Changed while it was read, or no longer this window's document: due.
      if (token !== sliceToken(id) || PM.hist?.busy?.() || PM.proj !== proj || !checkpointsDue.has(id)) return;
      writeSlot(proj, parts.join(''));
    } finally { slicing.delete(id); }
  })();
  PM.store.track?.(R.SLOT + id, work);
  await work;
}

/** Lightweight crash recovery: compact typed-history patches between checkpoints. */
R.recover = (proj: any, thumb: any, serialized?: string) => {
  if (!proj?.id) return null;
  const meta: any = { id: proj.id, name: proj.name || 'Untitled', at: Date.now(), w: proj.w, h: proj.h };
  if (thumb) meta.thumb = thumb;
  R.upsertMeta(meta);
  PM.store.set(R.trashKey, R.trashList().filter((item: any) => item.id !== proj.id));
  const entries = mainKeepsJournal() ? sinceCheckpoint(proj.id) : journal(proj.id);
  if (unsaved.has(proj.id) || !slotExists(proj.id) || !entries.length) return R.put(proj, thumb, serialized);
  const bytes = journalBytes(entries);
  if (checkpointsDue.has(proj.id) || bytes > MAX_JOURNAL_BYTES || Number(proj.revision || 0) % CHECKPOINT_REVISIONS === 0) {
    if (!sliceable(proj, bytes)) return R.put(proj, thumb);
    void checkpointInSlices(proj).catch(error => console.warn('Project checkpoint failed', error));
  }
  return meta;
};
/** Whether a periodic checkpoint of `proj`, whose journal since its last one
 *  is `journaled` bytes, is serialized in slices: main keeps the journal and
 *  takes slots as text, the slot is not a small one this window wrote and
 *  measured, and the journal is short of SLICED_JOURNAL_BYTES. Otherwise it is
 *  written at once, as before: edits that keep landing while one is read must
 *  not grow the journal to main's limit, where main refuses entries. */
function sliceable(proj: any, journaled: number): boolean {
  const slot = slots.get(proj.id);
  return mainKeepsJournal() && typeof PM.store.setText === 'function' && !(slot && slot.chars < VERIFY_MIN_CHARS) && !needsNewerReader(proj)
    && journaled <= SLICED_JOURNAL_BYTES;
}

/** Playback-safe checkpoint. Explicit save/close still uses the synchronous
 * barrier in put, which supersedes any in-progress background checkpoint.
 * When main keeps the journal (SAVE-1) the slot is posted synchronously: main
 * numbers a slot against the entries posted before it, so a snapshot captured
 * earlier and posted later would claim edits it does not hold. */
R.recoverAsync = async (proj: any, thumb: any, isCurrent: () => boolean, serialized?: string) => {
  if (!proj?.id || !isCurrent()) return false;
  if (!PM.store.setSnapshotAsync || mainKeepsJournal()) { R.recover(proj, thumb, serialized); return true; }
  const meta: any = { id: proj.id, name: proj.name || 'Untitled', at: Date.now(), w: proj.w, h: proj.h };
  if (thumb) meta.thumb = thumb;
  R.upsertMeta(meta);
  PM.store.set(R.trashKey, R.trashList().filter((item: any) => item.id !== proj.id));
  // A project written for a newer reader keeps the copy stored when it opened (see put).
  if (needsNewerReader(proj) && PM.store.has(R.SLOT + proj.id)) return true;
  const entries = journal(proj.id);
  if (PM.store.has(R.SLOT + proj.id) && entries.length && journalBytes(entries) <= MAX_JOURNAL_BYTES
      && Number(proj.revision || 0) % CHECKPOINT_REVISIONS !== 0) return true;
  const saved = await PM.store.setSnapshotAsync(R.SLOT + proj.id, { v: PM.version || '1.0.0', proj }, isCurrent);
  if (!saved) return false;
  // Edits can arrive while native storage commits. Retain patches newer than
  // the captured prefix instead of deleting their recovery journal.
  const remaining = journal(proj.id).slice(entries.length);
  if (remaining.length) PM.store.set(R.JOURNAL + proj.id, remaining);
  else PM.store.del(R.JOURNAL + proj.id);
  return true;
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
  const active = PM.proj && PM.proj.id === id ? PM.proj : null;
  // A background tab of this window: its live document, not the stored copy.
  const parked = active ? null : PM.liveProject?.(id) ?? null;
  const live = active || parked;
  const meta = R.list().find((x: any) => x.id === id);
  /* SAVE-1c: a project is written only by the window that has it. Without a
     live copy here, the stored one lacks the newest edits of whichever window
     has it open (main refuses that write); so that window renames it. Only if
     none has it is the stored copy renamed here, as before. */
  if (!live && PM.windows?.supported && typeof PM.windows.renameProject === 'function') {
    const next = String(name == null ? '' : name).trim() || (meta && meta.name) || 'Untitled';
    void PM.windows.renameProject(id, next).then((routed: boolean) => {
      if (!routed) renameStored(id, next);
      PM.bus?.emit?.('projects:open');
    });
    return next;
  }
  if (!live) return renameStored(id, name);
  const previous = live.name || (meta && meta.name) || 'Untitled';
  const next = String(name == null ? '' : name).trim() || previous;
  // Persist the live document; do not change its name until the write succeeds.
  R.put({ ...live, name: next });
  if (parked) { markFileDirty(id, next !== previous); parked.name = next; }
  if (active) writeDocument(PM, [['name']], () => { active.name = next; });
  PM.bus?.emit?.('projects:open');
  if (active) { PM.touch?.(); PM.bus?.emit?.('project'); }
  return next;
};

/** Renames a project no window holds a live copy of, from its stored copy. */
function renameStored(id: any, name: any) {
  const raw = R.get(id);
  const meta = R.list().find((x: any) => x.id === id);
  const previous = (raw && raw.name) || (meta && meta.name) || 'Untitled';
  const next = String(name == null ? '' : name).trim() || previous;
  if (raw) R.put({ ...raw, name: next });
  else if (meta) R.upsertMeta({ ...meta, name: next, at: Date.now() });
  else return null;
  markFileDirty(id, next !== previous);
  PM.bus?.emit?.('projects:open');
  return next;
}

/* An inactive project emits no active-document autosave events: its file
   association is marked dirty here, so the Projects screen and close guard
   never imply that a rename already reached the external .pmv file. */
function markFileDirty(id: any, changed: boolean) {
  if (!changed) return;
  const file = PM.projectFileState?.(id);
  if (file) file.dirty = true;
}

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
  if (typeof raw === 'string') { try { raw = JSON.parse(raw); } catch (e) { return null; } }
  if (!R.unwrap(raw)) return null;
  // The slot with the journal entries after it (shared/project-journal.ts).
  return recoverProject(raw, journal(id));
};

/** The opening path owns a fresh parsed copy, without synchronous IPC or a
 * second full-document clone. Small journal patches are applied between yields. */
R.getAsync = async (id: string) => {
  if (!PM.store.getAsync) return R.get(id);
  const entries = await PM.store.getAsync(R.JOURNAL + id, []);
  const raw = await PM.store.getAsync(R.SLOT + id, null);
  const envelope = typeof raw === 'string' ? await parseAsync(raw) : raw;
  let project = R.unwrap(envelope);
  if (!project) {
    const autosave = await PM.store.getAsync('autosave', null);
    const old = R.unwrap(typeof autosave === 'string' ? await parseAsync(autosave) : autosave);
    return old?.id === id ? old : null;
  }
  // The journal entries after the slot (shared/project-journal.ts), applied
  // between yields to this fresh copy. A slot without a write number (no
  // SAVE-1 journal in main) may have been checkpointed after the entries it
  // already holds: those are at or below its revision.
  const numbered = typeof envelope?.seq === 'number';
  let deadline = performance.now() + 4;
  for (const entry of entriesAfter(envelope, entries)) {
    if (!numbered && typeof entry.revision === 'number' && Number.isFinite(project.revision) && entry.revision <= project.revision) continue;
    project = applyPatches(project, entry.patches);
    if (performance.now() >= deadline) { await yieldTask(); deadline = performance.now() + 4; }
  }
  return project;
};

R.getStateAsync = async (id: string) => {
  if (!PM.store.getAsync) return R.getState(id);
  const metadata = await PM.store.getAsync(`projectMeta.${id}`, null);
  const history = await PM.store.getAsync(`projectHistory.${id}`, undefined);
  const legacy = !metadata || history === undefined ? await PM.store.getAsync(R.STATE + id, null) : null;
  const state = metadata ? { ...legacy, ...metadata } : legacy;
  return history === undefined ? state : { ...state, history };
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
  slots.delete(id); unsaved.delete(id); journalFull.delete(id);
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
  slots.delete(id); unsaved.delete(id); journalFull.delete(id);
  PM.store.set(R.trashKey, R.trashList().filter((x: any) => x.id !== id));
  try { PM.store.del(R.SLOT + id); } catch (e) { }
  try { PM.store.del(R.STATE + id); } catch (e) { }
  for (const prefix of ['projectMeta.', 'projectHistory.', 'projectWorkspace.', 'documentHost.']) {
    try { PM.store.del(prefix + id); } catch (e) { }
  }
  // Main deletes the journal with the slot (SAVE-1).
  if (!mainKeepsJournal()) { try { PM.store.del(R.JOURNAL + id); } catch (e) { } }
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
  const id = entry.projectId;
  const slot = slots.get(id);
  if (slot) slot.journal = 'other';
  /* The entries this window journaled since its last checkpoint, with each
     one's size: an append neither reads the journal back from the store nor
     serializes all of it again to measure it. */
  const cached = written;
  const state: JournalState = cached && cached.id === id ? cached : (() => {
    const entries = mainKeepsJournal() ? [] : journal(id);
    return { id, entries, sizes: entries.map(item => journalBytes(item)) };
  })();
  const item: any = { revision: Number(entry.revision) || 0, patches: entry.patches };
  /* Keep the journal bounded before the next debounced checkpoint (its JSON:
     brackets, the entries, and a comma between each two). */
  const measure = () => 2 + state.sizes.reduce((sum, size) => sum + size, 0) + Math.max(0, state.sizes.length - 1);
  if (mainKeepsJournal()) {
    PM.store.appendJournal(R.JOURNAL + id, item).catch(() => { journalFull.add(id); unsaved.add(id); checkpointSoon(id); });
    // Refused (main's journal is closed): sent, so main knows, but not kept here.
    if (journalFull.has(id)) return;
    supersede(state.entries, entry.patches, index => { state.sizes[index] = journalBytes(state.entries[index]); });
    state.entries.push(item);
    state.sizes.push(journalBytes(item));
    written = state;
    /* Past the store's limit this copy (only a measure for the bound) is
       dropped; main, which holds the real journal, refuses what it cannot
       keep and closes it (journalFull follows from that refusal). */
    if (measure() > STORE_JOURNAL_BYTES) { state.entries = []; state.sizes = []; }
    /* Past the bound the project is checkpointed, unless its slot writes are
       failing: then main's journal holds these edits on disk. */
    if (measure() > MAX_JOURNAL_BYTES && !unsaved.has(id) && PM.proj?.id === id) {
      if (sliceable(PM.proj, measure())) void checkpointInSlices(PM.proj).catch(error => console.warn('Project checkpoint failed', error));
      else R.put(PM.proj);
    }
    return;
  }
  supersede(state.entries, entry.patches, index => { state.sizes[index] = journalBytes(state.entries[index]); });
  state.entries.push(item);
  state.sizes.push(journalBytes(item));
  if (measure() > MAX_JOURNAL_BYTES) {
    written = null;
    if (PM.proj?.id === id) R.put(PM.proj);
    return;
  }
  /* Serialized off the edit's task when the store can (the journal is a
     snapshot; a newer write supersedes a pending one, and a checkpoint's
     delete supersedes both). Cloning and posting it in the edit's task cost
     6 ms per edit at 1,000 layers (docs/engine/perf.md). */
  if (PM.store.setAsync) void PM.store.setAsync(R.JOURNAL + id, state.entries.slice());
  else PM.store.set(R.JOURNAL + id, state.entries);
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
