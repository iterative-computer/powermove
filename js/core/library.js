/* Powermove — generative library. Sections (saved layer groups) and Looks (saved
   shader presets) with live thumbnails, so AI-generated work is browsable and
   reusable instead of living only in the undo stack. Stored inside the project,
   so it saves, autosaves, and round-trips through .pmv files. */
(() => {
const PM = window.PM;
const MAX_PER_KIND = 24;
const MAX_VERSIONS = 12;
const clone = (value) => JSON.parse(JSON.stringify(value));

const lib = () => {
  if (!PM.proj.library || typeof PM.proj.library !== 'object') PM.proj.library = { sections: [], looks: [] };
  if (!Array.isArray(PM.proj.library.sections)) PM.proj.library.sections = [];
  if (!Array.isArray(PM.proj.library.looks)) PM.proj.library.looks = [];
  PM.proj.library.sections.forEach(s => {
    if (!Array.isArray(s.versions)) s.versions = [];
    if (!s.versions.length && Array.isArray(s.layers)) {
      s.versions.push({ layers: clone(s.layers), thumb: s.thumb || null, at: s.at || Date.now() });
    }
  });
  return PM.proj.library;
};

function thumb() {
  try { return PM.Export.snapshot(PM.time, 340); } catch (e) { return null; }
}
function trim(kind) {
  const list = lib()[kind];
  while (list.length > MAX_PER_KIND) list.pop();
}

/** Save the selected layers (or the whole composition) as a reusable section.
    The thumbnail is captured at the current playhead. */
function saveSection(name, ids) {
  const layers = PM.proj.layers.filter(l => !ids || ids.includes(l.id));
  if (!layers.length) { PM.toast('Nothing to save — no layers'); return null; }
  const at = Date.now(), preview = thumb(), snapshot = clone(layers);
  const entry = {
    id: PM.uid('S'),
    name: name || ('Section ' + (lib().sections.length + 1)),
    at,
    thumb: preview,
    comments: [],
    layers: snapshot,
    versions: [{ layers: clone(snapshot), thumb: preview, at }],
  };
  lib().sections.unshift(entry);
  trim('sections');
  PM.bus.emit('library');
  return entry;
}

/** Save the current selection (or full composition) as the newest version. */
function saveVersion(id, ids) {
  const s = lib().sections.find(x => x.id === id);
  if (!s) return null;
  const layers = PM.proj.layers.filter(l => !ids || ids.includes(l.id));
  if (!layers.length) { PM.toast('Nothing to save — no layers'); return null; }
  const at = Date.now(), preview = thumb(), snapshot = clone(layers);
  s.layers = snapshot; s.thumb = preview; s.at = at;
  s.versions.push({ layers: clone(snapshot), thumb: preview, at });
  if (s.versions.length > MAX_VERSIONS) s.versions.splice(0, s.versions.length - MAX_VERSIONS);
  PM.bus.emit('library');
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
    id: PM.uid('K'),
    name: name || L.name,
    at: Date.now(),
    thumb: thumb(),
    comments: [],
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
function insertSection(id) {
  const s = lib().sections.find(x => x.id === id);
  if (!s || !s.layers.length) return null;
  const start = PM.snapF(PM.clamp(PM.time, 0, PM.proj.dur), PM.proj.fps);
  let base = Infinity;
  s.layers.forEach(l => { base = Math.min(base, l.from); });
  if (!Number.isFinite(base)) base = 0;
  const clones = s.layers.map(src => {
    const c = PM.cloneLayer(JSON.parse(JSON.stringify(src)));
    c.from = Math.max(0, c.from + (start - base));
    return c;
  });
  const map = new Map(s.layers.map((src, i) => [src.id, clones[i]]));
  clones.forEach((c, i) => {
    const src = s.layers[i];
    c.parent = src.parent && map.has(src.parent) ? map.get(src.parent).id : null;
  });
  PM.hist.do('Insert section', () => {
    for (let i = clones.length - 1; i >= 0; i--) PM.addLayer(clones[i], 0);
    PM.selectLayers(clones.map(c => c.id));
  });
  PM.toast('Inserted "' + s.name + '" · ' + clones.length + (clones.length === 1 ? ' layer' : ' layers'));
  return clones;
}

/** Create a shader layer from a saved Look. */
function applyLook(id) {
  const k = lib().looks.find(x => x.id === id);
  if (!k) return null;
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
  const list = lib()[kind];
  const before = list.length;
  PM.hist.do('Delete from library', () => {
    PM.proj.library[kind] = list.filter(x => x.id !== id);
  });
  if (PM.proj.library[kind].length !== before) { PM.bus.emit('library'); PM.toast('Removed'); }
}

/** Attach a note/comment to a library entry. */
function comment(kind, id, text) {
  const e = lib()[kind].find(x => x.id === id);
  if (!e || !text || !text.trim()) return null;
  (e.comments = e.comments || []).push({ at: Date.now(), text: text.trim() });
  PM.bus.emit('library');
  return e.comments;
}

PM.Library = { all: lib, saveSection, saveVersion, saveLook, insertSection, applyLook, drop, comment };
})();
