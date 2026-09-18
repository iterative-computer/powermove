/* Ported from js/core/projects.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

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

/** Upsert registry metadata for a project and persist its data. */
R.put = (proj: any, thumb: any) => {
  if (!proj || !proj.id) return;
  const meta: any = { id: proj.id, name: proj.name || 'Untitled', at: Date.now(), w: proj.w, h: proj.h };
  if (thumb) meta.thumb = thumb;
  R.upsertMeta(meta);
  PM.store.set(R.trashKey, R.trashList().filter((x: any) => x.id !== proj.id));
  /* Serialize the project passed to us, not whichever project happens to be
     active. This matters for rename/duplicate and keeps every slot canonical. */
  if (PM.store.set(R.SLOT + proj.id, { v: PM.version || '1.0.0', proj }) === false) throw new Error('Project storage is unavailable or full');
  PM.store.del(R.JOURNAL + proj.id);
  return meta;
};

/** Lightweight crash recovery: compact typed-history patches between checkpoints. */
R.recover = (proj: any, thumb: any) => {
  if (!proj?.id) return null;
  const meta: any = { id: proj.id, name: proj.name || 'Untitled', at: Date.now(), w: proj.w, h: proj.h };
  if (thumb) meta.thumb = thumb;
  R.upsertMeta(meta);
  PM.store.set(R.trashKey, R.trashList().filter((item: any) => item.id !== proj.id));
  const raw = PM.store.get(R.SLOT + proj.id, null);
  const entries = journal(proj.id);
  if (!raw || !entries.length || journalBytes(entries) > MAX_JOURNAL_BYTES
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
  if (active) active.name = next;
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
  PM.store.set(R.trashKey, R.trashList().filter((x: any) => x.id !== id));
  try { PM.store.del(R.SLOT + id); } catch (e) { }
  try { PM.store.del(R.STATE + id); } catch (e) { }
  for (const prefix of ['projectMeta.', 'projectHistory.', 'projectWorkspace.']) {
    try { PM.store.del(prefix + id); } catch (e) { }
  }
  try { PM.store.del(R.JOURNAL + id); } catch (e) { }
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
  const entries = journal(entry.projectId);
  entries.push({ revision: Number(entry.revision) || 0, patches: entry.patches });
  /* Keep the journal bounded even before the next debounced checkpoint. */
  if (journalBytes(entries) > MAX_JOURNAL_BYTES) {
    if (PM.proj?.id === entry.projectId) R.put(PM.proj);
    return;
  }
  PM.store.set(R.JOURNAL + entry.projectId, entries);
});
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
