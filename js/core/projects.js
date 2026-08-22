/* Powermove — project registry. Multiple projects, each with its own storage slot,
   metadata and thumbnail. Pure storage logic — no DOM — so it runs headless in tests. */
(() => {
const PM = window.PM;

const R = {
  KEY: 'projects',          // [{id, name, at, thumb}]
  SLOT: 'project.',         // + id → serialized project
  openKey: 'openTabs',      // [id] in stable visible tab order
  trashKey: 'projectTrash', // metadata for recoverable deletion; slots stay intact
};

R.list = () => {
  const l = PM.store.get(R.KEY, null);
  return Array.isArray(l) ? l : [];
};

R.saveList = (list) => PM.store.set(R.KEY, list);

/** Accept both current save envelopes ({v, proj, ws}) and old bare projects. */
R.unwrap = (raw) => {
  if (!raw) return null;
  try { if (typeof raw === 'string') raw = JSON.parse(raw); } catch (e) { return null; }
  return raw && raw.proj && typeof raw.proj === 'object' ? raw.proj : raw;
};

/** Upsert registry metadata for a project and persist its data. */
R.put = (proj, thumb) => {
  if (!proj || !proj.id) return;
  const meta = { id: proj.id, name: proj.name || 'Untitled', at: Date.now() };
  if (thumb) meta.thumb = thumb;
  R.upsertMeta(meta);
  PM.store.set(R.trashKey, R.trashList().filter(x => x.id !== proj.id));
  /* Serialize the project passed to us, not whichever project happens to be
     active. This matters for rename/duplicate and keeps every slot canonical. */
  try { PM.store.set(R.SLOT + proj.id, { v: PM.version || '1.0.0', proj }); } catch (e) { /* quota */ }
  return meta;
};

/** Update metadata only — never touches stored project data. */
R.upsertMeta = (meta) => {
  if (!meta || !meta.id) return;
  const list = R.list();
  const i = list.findIndex(x => x.id === meta.id);
  if (i >= 0) {
    if (!meta.thumb && list[i].thumb) meta.thumb = list[i].thumb;
    list[i] = meta;
  } else list.unshift(meta);
  R.saveList(list);
};

R.get = (id) => {
  if (!id) return null;
  let raw = null;
  try { raw = PM.store.get(R.SLOT + id, null); } catch (e) { }
  if (!raw) {
    // legacy: fall back to the single autosave slot for the most recent project
    const auto = PM.store.get('autosave', null);
    if (auto && auto.proj && auto.proj.id === id) return auto.proj;
    return null;
  }
  return R.unwrap(raw);
};

/** Pure boot choice: content first (open tabs, then registry), then a named
    empty project. Anonymous empty projects never win over the welcome demo. */
R.pickBoot = ({ tabs = R.tabs(), metas = R.list(), get = R.get, legacy = null } = {}) => {
  const ids = [...tabs, ...metas.map(m => m.id).filter(id => !tabs.includes(id))];
  let namedEmpty = null;
  for (const id of ids) {
    const p = R.unwrap(get(id));
    if (!p || typeof p !== 'object') continue;
    if (Array.isArray(p.layers) && p.layers.length) return p;
    if (!namedEmpty && p.name && p.name !== 'Untitled') namedEmpty = p;
  }
  const old = R.unwrap(legacy && legacy.proj ? legacy.proj : legacy);
  if (old && Array.isArray(old.layers) && old.layers.length && !metas.some(m => m.id === old.id)) return old;
  return namedEmpty;
};

R.remove = (id) => {
  R.saveList(R.list().filter(x => x.id !== id));
  PM.store.set(R.trashKey, R.trashList().filter(x => x.id !== id));
  try { PM.store.del(R.SLOT + id); } catch (e) { }
};

/* Recoverable project deletion. A trashed project keeps its storage slot until
   the user explicitly chooses Delete Forever. */
R.trashList = () => {
  const list = PM.store.get(R.trashKey, []);
  return Array.isArray(list) ? list : [];
};
R.trash = (id) => {
  const meta = R.list().find(x => x.id === id);
  if (!meta) return false;
  R.saveList(R.list().filter(x => x.id !== id));
  const trash = R.trashList().filter(x => x.id !== id);
  trash.unshift({ ...meta, deletedAt: Date.now() });
  PM.store.set(R.trashKey, trash);
  R.markClosed(id);
  return true;
};
R.restore = (id) => {
  const meta = R.trashList().find(x => x.id === id);
  if (!meta || !R.get(id)) return false;
  PM.store.set(R.trashKey, R.trashList().filter(x => x.id !== id));
  const clean = { ...meta, at: Date.now() }; delete clean.deletedAt;
  R.upsertMeta(clean);
  return true;
};
R.destroy = (id) => {
  PM.store.set(R.trashKey, R.trashList().filter(x => x.id !== id));
  try { PM.store.del(R.SLOT + id); } catch (e) { }
};

/* ── open-tab bookkeeping ──────────────────────────────── */
R.tabs = () => {
  const t = PM.store.get(R.openKey, []);
  return Array.isArray(t) ? t.filter(id => R.list().some(m => m.id === id)) : [];
};
R.markOpen = (id) => {
  const t = R.tabs();
  /* Selecting an already-open project must never reshuffle the visible strip.
     A genuinely new open is the only implicit ordering change, and it appends
     where the user can predictably find it. */
  if (!t.includes(id)) t.push(id);
  PM.store.set(R.openKey, t.slice(0, 8));
};
R.markClosed = (id) => PM.store.set(R.openKey, R.tabs().filter(x => x !== id));

PM.Projects = R;
})();
