/* Ported from js/core/library.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const MAX_PER_KIND: any = 24;
const MAX_VERSIONS: any = 12;
const clone: any = (value: any) => JSON.parse(JSON.stringify(value));
const SCHEMA_VERSION: any = 1;

const ensureLibrary: any = (project: any = PM.proj) => {
  if (!project.library || typeof project.library !== 'object') project.library = { sections: [], looks: [] };
  if (!Array.isArray(project.library.sections)) project.library.sections = [];
  if (!Array.isArray(project.library.looks)) project.library.looks = [];
  project.library.sections.forEach((s: any) => {
    s.schemaVersion = SCHEMA_VERSION;
    s.id = s.id || PM.uid('S');
    s.sourceProjectId = s.sourceProjectId || project.id;
    s.sourceProjectName = s.sourceProjectName || project.name || 'Untitled';
    s.tags = Array.isArray(s.tags) ? s.tags.filter((tag: any) => typeof tag === 'string') : [];
    s.deletedAt = Number.isFinite(s.deletedAt) ? s.deletedAt : null;
    if (!Array.isArray(s.versions)) s.versions = [];
    if (!s.versions.length && Array.isArray(s.layers)) {
      s.versions.push({ id: PM.uid('SV'), schemaVersion: SCHEMA_VERSION, layers: clone(s.layers), thumb: s.thumb || null, at: s.at || Date.now() });
    }
    s.versions.forEach((v: any) => { v.id = v.id || PM.uid('SV'); v.schemaVersion = SCHEMA_VERSION; });
  });
  project.library.looks.forEach((look: any) => {
    look.schemaVersion = SCHEMA_VERSION;
    look.id = look.id || PM.uid('K');
    look.sourceProjectId = look.sourceProjectId || project.id;
    look.sourceProjectName = look.sourceProjectName || project.name || 'Untitled';
    look.deletedAt = Number.isFinite(look.deletedAt) ? look.deletedAt : null;
  });
  return project.library;
};
const lib: any = () => ensureLibrary(PM.proj);

function projectMeta(project: any) {
  return PM.Projects?.list?.().find((m: any) => m.id === project.id) || null;
}
function persistLibraryProject(project: any) {
  ensureLibrary(project);
  if (project.id === PM.proj.id) PM.bus.emit('library');
  else PM.Projects?.put?.(project, projectMeta(project)?.thumb);
}
function resolveSection(id: any, sourceProjectId: any = PM.proj.id) {
  const project: any = sourceProjectId === PM.proj.id ? PM.proj : PM.Projects?.get?.(sourceProjectId);
  if (!project) return null;
  const entry: any = ensureLibrary(project).sections.find((section: any) => section.id === id);
  return entry ? { entry, project } : null;
}
function allProjects() {
  const projects: any = [PM.proj];
  (PM.Projects?.list?.() || []).forEach((meta: any) => {
    if (meta.id === PM.proj.id) return;
    const project: any = PM.Projects.get(meta.id);
    if (project) projects.push(project);
  });
  return projects;
}
function catalog(scope: any = 'project', includeDeleted: any = false) {
  const projects: any = scope === 'global' ? allProjects() : [PM.proj];
  return projects.flatMap((project: any) => ensureLibrary(project).sections
    .filter((section: any) => includeDeleted || !section.deletedAt)
    .map((section: any) => ({
      ...section,
      sourceProjectId: project.id,
      sourceProjectName: project.name || section.sourceProjectName || 'Untitled',
      isCurrentProject: project.id === PM.proj.id,
    })));
}
function lookCatalog(scope: any = 'project', includeDeleted: any = false) {
  const projects: any = scope === 'global' ? allProjects() : [PM.proj];
  return projects.flatMap((project: any) => ensureLibrary(project).looks
    .filter((look: any) => includeDeleted || !look.deletedAt)
    .map((look: any) => ({ ...look, sourceProjectId: project.id, sourceProjectName: project.name || look.sourceProjectName || 'Untitled' })));
}
function resolveLook(id: any, sourceProjectId: any = PM.proj.id) {
  const project: any = sourceProjectId === PM.proj.id ? PM.proj : PM.Projects?.get?.(sourceProjectId);
  if (!project) return null;
  const entry: any = ensureLibrary(project).looks.find((look: any) => look.id === id);
  return entry ? { entry, project } : null;
}

function thumb() {
  try { return PM.Export.snapshot(PM.time, 340); } catch (e: any) { return null; }
}
function trim(kind: any) {
  const list: any = lib()[kind];
  while (list.length > MAX_PER_KIND) {
    const deleted: any = list.findLastIndex?.((item: any) => item.deletedAt) ?? -1;
    list.splice(deleted >= 0 ? deleted : list.length - 1, 1);
  }
}

/** Save the selected layers (or the whole composition) as a reusable section.
    The thumbnail is captured at the current playhead. */
function saveSection(name: any, ids: any) {
  const layers: any = PM.proj.layers.filter((l: any) => !ids || ids.includes(l.id));
  if (!layers.length) { PM.toast('Nothing to save — no layers'); return null; }
  const at: any = Date.now(), preview: any = thumb(), snapshot: any = clone(layers);
  const entry: any = {
    schemaVersion: SCHEMA_VERSION,
    id: PM.uid('S'),
    name: name || ('Section ' + (lib().sections.length + 1)),
    at,
    thumb: preview,
    sourceProjectId: PM.proj.id,
    sourceProjectName: PM.proj.name,
    tags: [...new Set(snapshot.map((layer: any) => layer.type))],
    deletedAt: null,
    comments: [],
    layers: snapshot,
    versions: [{ id: PM.uid('SV'), schemaVersion: SCHEMA_VERSION, layers: clone(snapshot), thumb: preview, at }],
  };
  const command: any = { type: 'create_section', section: entry };
  if (PM.Edit) {
    const result: any = PM.Edit.apply(command, { label: 'Save section', origin: 'library' });
    if (!result.ok) { PM.toast(result.message); return null; }
    return lib().sections.find((section: any) => section.id === entry.id) || entry;
  }
  lib().sections.unshift(entry); trim('sections'); PM.bus.emit('library');
  return entry;
}

/** Save the current selection (or full composition) as the newest version. */
function saveVersion(id: any, ids: any, options: any = {}) {
  const found: any = resolveSection(id, options.sourceProjectId);
  if (!found) return null;
  const { entry: s, project }: any = found;
  const layers: any = PM.proj.layers.filter((l: any) => !ids || ids.includes(l.id));
  if (!layers.length) { PM.toast('Nothing to save — no layers'); return null; }
  const at: any = Date.now(), preview: any = thumb(), snapshot: any = clone(layers);
  const version: any = { id: PM.uid('SV'), schemaVersion: SCHEMA_VERSION, layers: clone(snapshot), thumb: preview, at };
  if (project.id === PM.proj.id && PM.Edit) {
    const result: any = PM.Edit.apply({ type: 'update_section', sectionId: id, layers: snapshot, thumb: preview, at, version }, {
      label: 'Update section', origin: 'library',
    });
    if (!result.ok) { PM.toast(result.message); return null; }
  } else {
    s.layers = snapshot; s.thumb = preview; s.at = at;
    s.tags = [...new Set(snapshot.map((layer: any) => layer.type))];
    s.versions.push(version);
    if (s.versions.length > MAX_VERSIONS) s.versions.splice(0, s.versions.length - MAX_VERSIONS);
    persistLibraryProject(project);
  }
  PM.toast(`Saved version ${s.versions.length} of "${s.name}"`);
  return s;
}

/** Save a shader layer's code + uniform values as a Look. */
function saveLook(name: any, L: any) {
  L = L || PM.firstSel();
  if (!L || L.type !== 'shader') { PM.toast('Select a shader layer to save a look'); return null; }
  const uniforms: any = {};
  for (const k in L.d.uniforms) uniforms[k] = JSON.parse(JSON.stringify(L.d.uniforms[k]));
  const entry: any = {
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
function insertSection(id: any, options: any = {}) {
  const found: any = resolveSection(id, options.sourceProjectId);
  if (!found || found.entry.deletedAt || !found.entry.layers.length) return null;
  const s: any = found.entry;
  const start: any = PM.snapF(PM.clamp(PM.time, 0, PM.proj.dur), PM.proj.fps);
  let base: any = Infinity;
  s.layers.forEach((l: any) => { base = Math.min(base, l.from); });
  if (!Number.isFinite(base)) base = 0;
  const clones: any = s.layers.map((src: any) => {
    const c: any = PM.cloneLayer(JSON.parse(JSON.stringify(src)));
    if (c.type === 'audio' && PM.Audio) PM.Audio.normalizeLayer(c);
    c.from = Math.max(0, c.from + (start - base));
    return c;
  });
  const instanceId: any = PM.uid('SI');
  const map: any = new Map(s.layers.map((src: any, i: any) => [src.id, clones[i]]));
  clones.forEach((c: any, i: any) => {
    const src: any = s.layers[i];
    c.parent = PM.TYPE_META[c.type] && PM.TYPE_META[c.type].transform === false
      ? null : src.parent && map.has(src.parent) ? map.get(src.parent).id : null;
    c.sectionRef = {
      schemaVersion: SCHEMA_VERSION, sectionId: s.id, sourceProjectId: found.project.id,
      sourceVersionId: s.versions.at(-1)?.id || null, instanceId, sourceLayerId: src.id,
    };
  });
  PM.hist.do('Insert section', () => {
    for (let i: any = clones.length - 1; i >= 0; i--) PM.addLayer(clones[i], 0);
    PM.selectLayers(clones.map((c: any) => c.id));
  });
  PM.toast('Inserted "' + s.name + '" · ' + clones.length + (clones.length === 1 ? ' layer' : ' layers'));
  return clones;
}

let editSession: any = null;
function openSection(id: any, options: any = {}) {
  const clones: any = insertSection(id, options);
  if (!clones) return null;
  editSession = { sectionId: id, sourceProjectId: options.sourceProjectId || PM.proj.id, layerIds: clones.map((layer: any) => layer.id) };
  PM.toast('Section opened as editable layers. Edit normally, then save from Library.');
  return editSession;
}
function saveEditSession(asNew: any = false, name: any) {
  if (!editSession) return null;
  const ids: any = editSession.layerIds.filter((id: any) => PM.L(id));
  if (!ids.length) { PM.toast('The editable section layers are no longer in this composition'); return null; }
  if (asNew) {
    const made: any = saveSection(name || 'Section copy', ids);
    editSession = made ? { sectionId: made.id, sourceProjectId: PM.proj.id, layerIds: ids } : editSession;
    return made;
  }
  return saveVersion(editSession.sectionId, ids, { sourceProjectId: editSession.sourceProjectId });
}
function currentEdit() { return editSession ? { ...editSession } : null; }

function usage(id: any, sourceProjectId: any = PM.proj.id) {
  return allProjects().reduce((count: any, project: any) => count + (project.layers || []).filter((layer: any) =>
    layer.sectionRef?.sectionId === id && layer.sectionRef?.sourceProjectId === sourceProjectId).length, 0);
}

function renameSection(id: any, name: any, sourceProjectId: any) {
  const found: any = resolveSection(id, sourceProjectId);
  if (!found) return false;
  found.entry.name = String(name || '').trim() || found.entry.name;
  found.entry.at = Date.now(); persistLibraryProject(found.project); return true;
}
function duplicateSection(id: any, sourceProjectId: any, targetProjectId: any = PM.proj.id) {
  const found: any = resolveSection(id, sourceProjectId); if (!found) return null;
  const target: any = targetProjectId === PM.proj.id ? PM.proj : PM.Projects?.get?.(targetProjectId);
  if (!target) return null;
  const copyEntry: any = clone(found.entry);
  copyEntry.id = PM.uid('S'); copyEntry.name += ' copy'; copyEntry.at = Date.now(); copyEntry.deletedAt = null;
  copyEntry.sourceProjectId = target.id; copyEntry.sourceProjectName = target.name;
  copyEntry.versions = copyEntry.versions.map((version: any) => ({ ...version, id: PM.uid('SV') }));
  ensureLibrary(target).sections.unshift(copyEntry); persistLibraryProject(target); return copyEntry;
}
function trashSection(id: any, sourceProjectId: any) {
  const found: any = resolveSection(id, sourceProjectId); if (!found) return false;
  if (found.entry.deletedAt) return true;
  const mutate: any = () => { found.entry.deletedAt = Date.now(); found.entry.at = Date.now(); };
  if (found.project.id === PM.proj.id && PM.hist?.do) PM.hist.do('Delete section', mutate);
  else mutate();
  persistLibraryProject(found.project);
  PM.toast('Section moved to Library Trash');
  return true;
}
function restoreSection(id: any, sourceProjectId: any) {
  const found: any = resolveSection(id, sourceProjectId); if (!found) return false;
  const mutate: any = () => { found.entry.deletedAt = null; found.entry.at = Date.now(); };
  if (found.project.id === PM.proj.id && PM.hist?.do) PM.hist.do('Restore section', mutate);
  else mutate();
  persistLibraryProject(found.project);
  PM.toast('Section restored');
  return true;
}

/** Create a shader layer from a saved Look. */
function applyLook(id: any, options: any = {}) {
  const found: any = resolveLook(id, options.sourceProjectId);
  const k: any = found?.entry;
  if (!k || k.deletedAt) return null;
  PM.hist.do('Apply look', () => {
    const L: any = PM.mkLayer('shader', { name: k.name });
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

function drop(kind: any, id: any) {
  if (kind === 'sections') { trashSection(id, PM.proj.id); PM.toast('Moved to Library Trash'); return; }
  if (kind === 'looks') {
    const found: any = resolveLook(id, PM.proj.id);
    if (found) { found.entry.deletedAt = Date.now(); persistLibraryProject(found.project); PM.toast('Moved to Library Trash'); }
    return;
  }
  const list: any = lib()[kind];
  const before: any = list.length;
  PM.hist.do('Delete from library', () => {
    PM.proj.library[kind] = list.filter((x: any) => x.id !== id);
  });
  if (PM.proj.library[kind].length !== before) { PM.bus.emit('library'); PM.toast('Removed'); }
}
function renameLook(id: any, name: any, sourceProjectId: any) {
  const found: any = resolveLook(id, sourceProjectId); if (!found) return false;
  found.entry.name = String(name || '').trim() || found.entry.name;
  found.entry.at = Date.now(); persistLibraryProject(found.project); return true;
}
function trashLook(id: any, sourceProjectId: any) {
  const found: any = resolveLook(id, sourceProjectId); if (!found) return false;
  found.entry.deletedAt = Date.now(); persistLibraryProject(found.project); return true;
}
function restoreLook(id: any, sourceProjectId: any) {
  const found: any = resolveLook(id, sourceProjectId); if (!found) return false;
  found.entry.deletedAt = null; found.entry.at = Date.now(); persistLibraryProject(found.project); return true;
}

/** Attach a note/comment to a library entry. */
function comment(kind: any, id: any, text: any) {
  const e: any = lib()[kind].find((x: any) => x.id === id);
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
}
