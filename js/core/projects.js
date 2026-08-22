/* Powermove — project registry. Multiple projects, each with its own storage slot,
   metadata and thumbnail. Pure storage logic — no DOM — so it runs headless in tests. */
(() => {
const PM = window.PM;

const R = {
  KEY: 'projects',          // [{id, name, at, thumb}]
  SLOT: 'project.',         // + id → serialized project
  openKey: 'openTabs',      // [id] in MRU order
};

R.list = () => {
  const l = PM.store.get(R.KEY, null);
  return Array.isArray(l) ? l : [];
};

R.saveList = (list) => PM.store.set(R.KEY, list);

/** Upsert registry metadata for a project and persist its data. */
R.put = (proj, thumb) => {
  if (!proj || !proj.id) return;
  const meta = { id: proj.id, name: proj.name || 'Untitled', at: Date.now() };
  if (thumb) meta.thumb = thumb;
  R.upsertMeta(meta);
  try { PM.store.set(R.SLOT + proj.id, PM.serialize()); } catch (e) { /* quota */ }
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
  try { return typeof raw === 'string' ? JSON.parse(raw) : raw; } catch (e) { return null; }
};

R.remove = (id) => {
  R.saveList(R.list().filter(x => x.id !== id));
  try { PM.store.del(R.SLOT + id); } catch (e) { }
};

/* ── open-tab bookkeeping ──────────────────────────────── */
R.tabs = () => {
  const t = PM.store.get(R.openKey, []);
  return Array.isArray(t) ? t.filter(id => R.list().some(m => m.id === id)) : [];
};
R.markOpen = (id) => {
  const t = R.tabs().filter(x => x !== id);
  t.unshift(id);
  PM.store.set(R.openKey, t.slice(0, 8));
};
R.markClosed = (id) => PM.store.set(R.openKey, R.tabs().filter(x => x !== id));

PM.Projects = R;
})();
