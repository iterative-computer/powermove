/* Powermove — generative library. Sections (saved layer groups) and Looks (saved
   shader presets) with live thumbnails, so AI-generated work is browsable and
   reusable instead of living only in the undo stack. Stored inside the project,
   so it saves, autosaves, and round-trips through .pmv files. */
(() => {
const PM = window.PM;
const MAX_PER_KIND = 24;
const MAX_VERSIONS = 12;
const clone = (value) => JSON.parse(JSON.stringify(value));
const SCHEMA_VERSION = 1;

const ensureLibrary = (project = PM.proj) => {
  if (!project.library || typeof project.library !== 'object') project.library = { sections: [], looks: [] };
  if (!Array.isArray(project.library.sections)) project.library.sections = [];
  if (!Array.isArray(project.library.looks)) project.library.looks = [];
  project.library.sections.forEach(s => {
    s.schemaVersion = SCHEMA_VERSION;
    s.id = s.id || PM.uid('S');
    s.sourceProjectId = s.sourceProjectId || project.id;
    s.sourceProjectName = s.sourceProjectName || project.name || 'Untitled';
    s.tags = Array.isArray(s.tags) ? s.tags.filter(tag => typeof tag === 'string') : [];
    s.deletedAt = Number.isFinite(s.deletedAt) ? s.deletedAt : null;
    if (!Array.isArray(s.versions)) s.versions = [];
    if (!s.versions.length && Array.isArray(s.layers)) {
      s.versions.push({ id: PM.uid('SV'), schemaVersion: SCHEMA_VERSION, layers: clone(s.layers), thumb: s.thumb || null, at: s.at || Date.now() });
    }
    s.versions.forEach(v => { v.id = v.id || PM.uid('SV'); v.schemaVersion = SCHEMA_VERSION; });
  });
  project.library.looks.forEach(look => {
    look.schemaVersion = SCHEMA_VERSION;
    look.id = look.id || PM.uid('K');
    look.sourceProjectId = look.sourceProjectId || project.id;
    look.sourceProjectName = look.sourceProjectName || project.name || 'Untitled';
    look.deletedAt = Number.isFinite(look.deletedAt) ? look.deletedAt : null;
  });
  return project.library;
};
const lib = () => ensureLibrary(PM.proj);

function projectMeta(project) {
  return PM.Projects?.list?.().find(m => m.id === project.id) || null;
}
function persistLibraryProject(project) {
  ensureLibrary(project);
  if (project.id === PM.proj.id) PM.bus.emit('library');
  else PM.Projects?.put?.(project, projectMeta(project)?.thumb);
}
function resolveSection(id, sourceProjectId = PM.proj.id) {
  const project = sourceProjectId === PM.proj.id ? PM.proj : PM.Projects?.get?.(sourceProjectId);
  if (!project) return null;
  const entry = ensureLibrary(project).sections.find(section => section.id === id);
  return entry ? { entry, project } : null;
}
function allProjects() {
  const projects = [PM.proj];
  (PM.Projects?.list?.() || []).forEach(meta => {
    if (meta.id === PM.proj.id) return;
    const project = PM.Projects.get(meta.id);
    if (project) projects.push(project);
  });
  return projects;
}
function catalog(scope = 'project', includeDeleted = false) {
  const projects = scope === 'global' ? allProjects() : [PM.proj];
  return projects.flatMap(project => ensureLibrary(project).sections
    .filter(section => includeDeleted || !section.deletedAt)
    .map(section => ({
      ...section,
      sourceProjectId: project.id,
      sourceProjectName: project.name || section.sourceProjectName || 'Untitled',
      isCurrentProject: project.id === PM.proj.id,
    })));
}
function lookCatalog(scope = 'project', includeDeleted = false) {
  const projects = scope === 'global' ? allProjects() : [PM.proj];
  return projects.flatMap(project => ensureLibrary(project).looks
    .filter(look => includeDeleted || !look.deletedAt)
    .map(look => ({ ...look, sourceProjectId: project.id, sourceProjectName: project.name || look.sourceProjectName || 'Untitled' })));
}
function resolveLook(id, sourceProjectId = PM.proj.id) {
  const project = sourceProjectId === PM.proj.id ? PM.proj : PM.Projects?.get?.(sourceProjectId);
  if (!project) return null;
  const entry = ensureLibrary(project).looks.find(look => look.id === id);
  return entry ? { entry, project } : null;
}

function thumb() {
  try { return PM.Export.snapshot(PM.time, 340); } catch (e) { return null; }
}
function trim(kind) {
  const list = lib()[kind];
  while (list.length > MAX_PER_KIND) {
    const deleted = list.findLastIndex?.(item => item.deletedAt) ?? -1;
    list.splice(deleted >= 0 ? deleted : list.length - 1, 1);
  }
}

/** Save the selected layers (or the whole composition) as a reusable section.
    The thumbnail is captured at the current playhead. */
function saveSection(name, ids) {
  const layers = PM.proj.layers.filter(l => !ids || ids.includes(l.id));
  if (!layers.length) { PM.toast('Nothing to save — no layers'); return null; }
  const at = Date.now(), preview = thumb(), snapshot = clone(layers);
  const entry = {
    schemaVersion: SCHEMA_VERSION,
    id: PM.uid('S'),
    name: name || ('Section ' + (lib().sections.length + 1)),
    at,
    thumb: preview,
    sourceProjectId: PM.proj.id,
    sourceProjectName: PM.proj.name,
    tags: [...new Set(snapshot.map(layer => layer.type))],
    deletedAt: null,
    comments: [],
    layers: snapshot,
    versions: [{ id: PM.uid('SV'), schemaVersion: SCHEMA_VERSION, layers: clone(snapshot), thumb: preview, at }],
  };
  const command = { type: 'create_section', section: entry };
  if (PM.Edit) {
    const result = PM.Edit.apply(command, { label: 'Save section', origin: 'library' });
    if (!result.ok) { PM.toast(result.message); return null; }
    return lib().sections.find(section => section.id === entry.id) || entry;
  }
  lib().sections.unshift(entry); trim('sections'); PM.bus.emit('library');
  return entry;
}

/** Save the current selection (or full composition) as the newest version. */
function saveVersion(id, ids, options = {}) {
  const found = resolveSection(id, options.sourceProjectId);
  if (!found) return null;
  const { entry: s, project } = found;
  const layers = PM.proj.layers.filter(l => !ids || ids.includes(l.id));
  if (!layers.length) { PM.toast('Nothing to save — no layers'); return null; }
  const at = Date.now(), preview = thumb(), snapshot = clone(layers);
  const version = { id: PM.uid('SV'), schemaVersion: SCHEMA_VERSION, layers: clone(snapshot), thumb: preview, at };
  if (project.id === PM.proj.id && PM.Edit) {
    const result = PM.Edit.apply({ type: 'update_section', sectionId: id, layers: snapshot, thumb: preview, at, version }, {
      label: 'Update section', origin: 'library',
    });
    if (!result.ok) { PM.toast(result.message); return null; }
  } else {
    s.layers = snapshot; s.thumb = preview; s.at = at;
    s.tags = [...new Set(snapshot.map(layer => layer.type))];
    s.versions.push(version);
    if (s.versions.length > MAX_VERSIONS) s.versions.splice(0, s.versions.length - MAX_VERSIONS);
    persistLibraryProject(project);
  }
  PM.toast(`Saved version ${s.versions.length} of "${s.name}"`);
  return s;
}

/** Save a shader layer's code + uniform values as a Look. */
function saveLook(name, L) {
  L = L || PM.firstSel();
  if (!L || L.type !== 'shader') { PM.toast('Select a shader layer to save a look'); return null; }
  const uniforms = {};
  for (const k in L.d.uniforms) uniforms[k] = JSON.parse(JSON.stringify(L.d.uniforms[k]));
  const entry = {
    schemaVersion: SCHEMA_VERSION,
    id: PM.uid('K'),
    name: name || L.name,
    at: Date.now(),
    thumb: thumb(),
    comments: [],
    sourceProjectId: PM.proj.id, sourceProjectName: PM.proj.name, deletedAt: null,
    code: L.d.code,
    uniforms,
  };
  lib().looks.unshift(entry);
  trim('looks');
  PM.bus.emit('library');
  return entry;
}

/** Re-insert a saved section at the playhead with fresh identities.
    Parenting inside the section survives; parenting across the boundary is cut. */
function insertSection(id, options = {}) {
  const found = resolveSection(id, options.sourceProjectId);
  if (!found || found.entry.deletedAt || !found.entry.layers.length) return null;
  const s = found.entry;
  const start = PM.snapF(PM.clamp(PM.time, 0, PM.proj.dur), PM.proj.fps);
  let base = Infinity;
  s.layers.forEach(l => { base = Math.min(base, l.from); });
  if (!Number.isFinite(base)) base = 0;
  const clones = s.layers.map(src => {
    const c = PM.cloneLayer(JSON.parse(JSON.stringify(src)));
    c.from = Math.max(0, c.from + (start - base));
    return c;
  });
  const instanceId = PM.uid('SI');
  const map = new Map(s.layers.map((src, i) => [src.id, clones[i]]));
  clones.forEach((c, i) => {
    const src = s.layers[i];
    c.parent = src.parent && map.has(src.parent) ? map.get(src.parent).id : null;
    c.sectionRef = {
      schemaVersion: SCHEMA_VERSION, sectionId: s.id, sourceProjectId: found.project.id,
      sourceVersionId: s.versions.at(-1)?.id || null, instanceId, sourceLayerId: src.id,
    };
  });
  PM.hist.do('Insert section', () => {
    for (let i = clones.length - 1; i >= 0; i--) PM.addLayer(clones[i], 0);
    PM.selectLayers(clones.map(c => c.id));
  });
  PM.toast('Inserted "' + s.name + '" · ' + clones.length + (clones.length === 1 ? ' layer' : ' layers'));
  return clones;
}

let editSession = null;
function openSection(id, options = {}) {
  const clones = insertSection(id, options);
  if (!clones) return null;
  editSession = { sectionId: id, sourceProjectId: options.sourceProjectId || PM.proj.id, layerIds: clones.map(layer => layer.id) };
  PM.toast('Section opened as editable layers. Edit normally, then save from Library.');
  return editSession;
}
function saveEditSession(asNew = false, name) {
  if (!editSession) return null;
  const ids = editSession.layerIds.filter(id => PM.L(id));
  if (!ids.length) { PM.toast('The editable section layers are no longer in this composition'); return null; }
  if (asNew) {
    const made = saveSection(name || 'Section copy', ids);
    editSession = made ? { sectionId: made.id, sourceProjectId: PM.proj.id, layerIds: ids } : editSession;
    return made;
  }
  return saveVersion(editSession.sectionId, ids, { sourceProjectId: editSession.sourceProjectId });
}
function currentEdit() { return editSession ? { ...editSession } : null; }

function usage(id, sourceProjectId = PM.proj.id) {
  return allProjects().reduce((count, project) => count + (project.layers || []).filter(layer =>
    layer.sectionRef?.sectionId === id && layer.sectionRef?.sourceProjectId === sourceProjectId).length, 0);
}

function renameSection(id, name, sourceProjectId) {
  const found = resolveSection(id, sourceProjectId);
  if (!found) return false;
  found.entry.name = String(name || '').trim() || found.entry.name;
  found.entry.at = Date.now(); persistLibraryProject(found.project); return true;
}
function duplicateSection(id, sourceProjectId, targetProjectId = PM.proj.id) {
  const found = resolveSection(id, sourceProjectId); if (!found) return null;
  const target = targetProjectId === PM.proj.id ? PM.proj : PM.Projects?.get?.(targetProjectId);
  if (!target) return null;
  const copyEntry = clone(found.entry);
  copyEntry.id = PM.uid('S'); copyEntry.name += ' copy'; copyEntry.at = Date.now(); copyEntry.deletedAt = null;
  copyEntry.sourceProjectId = target.id; copyEntry.sourceProjectName = target.name;
  copyEntry.versions = copyEntry.versions.map(version => ({ ...version, id: PM.uid('SV') }));
  ensureLibrary(target).sections.unshift(copyEntry); persistLibraryProject(target); return copyEntry;
}
function trashSection(id, sourceProjectId) {
  const found = resolveSection(id, sourceProjectId); if (!found) return false;
  found.entry.deletedAt = Date.now(); persistLibraryProject(found.project); return true;
}
function restoreSection(id, sourceProjectId) {
  const found = resolveSection(id, sourceProjectId); if (!found) return false;
  found.entry.deletedAt = null; found.entry.at = Date.now(); persistLibraryProject(found.project); return true;
}

/** Create a shader layer from a saved Look. */
function applyLook(id, options = {}) {
  const found = resolveLook(id, options.sourceProjectId);
  const k = found?.entry;
  if (!k || k.deletedAt) return null;
  PM.hist.do('Apply look', () => {
    const L = PM.mkLayer('shader', { name: k.name });
    L.d.code = k.code;
    PM.addLayer(L, 0);
    PM.syncShaderUniforms(L);
    for (const un in (k.uniforms || {})) {
      if (L.d.uniforms[un]) {
        L.d.uniforms[un].v = k.uniforms[un].v;
        L.d.uniforms[un].kf = JSON.parse(JSON.stringify(k.uniforms[un].kf || []));
      }
    }
    PM.GL.dropProgram(L._shaderKey);
    PM.selectLayers(L.id);
  });
  PM.toast('Applied look "' + k.name + '"');
  return true;
}

function drop(kind, id) {
  if (kind === 'sections') { trashSection(id, PM.proj.id); PM.toast('Moved to Library Trash'); return; }
  if (kind === 'looks') {
    const found = resolveLook(id, PM.proj.id);
    if (found) { found.entry.deletedAt = Date.now(); persistLibraryProject(found.project); PM.toast('Moved to Library Trash'); }
    return;
  }
  const list = lib()[kind];
  const before = list.length;
  PM.hist.do('Delete from library', () => {
    PM.proj.library[kind] = list.filter(x => x.id !== id);
  });
  if (PM.proj.library[kind].length !== before) { PM.bus.emit('library'); PM.toast('Removed'); }
}
function renameLook(id, name, sourceProjectId) {
  const found = resolveLook(id, sourceProjectId); if (!found) return false;
  found.entry.name = String(name || '').trim() || found.entry.name;
  found.entry.at = Date.now(); persistLibraryProject(found.project); return true;
}
function trashLook(id, sourceProjectId) {
  const found = resolveLook(id, sourceProjectId); if (!found) return false;
  found.entry.deletedAt = Date.now(); persistLibraryProject(found.project); return true;
}
function restoreLook(id, sourceProjectId) {
  const found = resolveLook(id, sourceProjectId); if (!found) return false;
  found.entry.deletedAt = null; found.entry.at = Date.now(); persistLibraryProject(found.project); return true;
}

/** Attach a note/comment to a library entry. */
function comment(kind, id, text) {
  const e = lib()[kind].find(x => x.id === id);
  if (!e || !text || !text.trim()) return null;
  (e.comments = e.comments || []).push({ at: Date.now(), text: text.trim() });
  PM.bus.emit('library');
  return e.comments;
}

PM.Library = {
  all: lib, ensure: ensureLibrary, catalog, lookCatalog, resolveSection, resolveLook, saveSection, saveVersion, saveLook,
  insertSection, openSection, saveEditSession, currentEdit, usage, renameSection, duplicateSection,
  trashSection, restoreSection, renameLook, trashLook, restoreLook, applyLook, drop, comment,
};
})();
