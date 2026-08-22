/* Powermove — projects screen. A home surface listing every project in the
   registry with live thumbnails; the entry point for creating, importing,
   renaming, duplicating, and deleting work. */
(() => {
const PM = window.PM, h = PM.h;
const S = { el: null, grid: null };

PM.ProjectsScreen = {
  get isOpen() { return !!S.el && S.el.isConnected; },
  show() { ensure(); paint(); S.el.classList.add('on'); },
  hide() { if (S.el) S.el.classList.remove('on'); },
  toggle() { this.isOpen ? this.hide() : this.show(); },
};

function ensure() {
  if (S.el) return;
  const close = () => PM.ProjectsScreen.hide();
  S.grid = h('div.ps-grid');
  S.el = h('div#projects-screen',
    h('div.ps-top',
      h('div.ps-title', h('b', 'Projects'), h('span', 'Everything is autosaved on this Mac')),
      h('span.sp'),
      h('button.chip', { onclick: () => { const inp = pickPmv(); inp.click(); } }, PM.icon('folder'), 'Import…'),
      h('button.btn.pri', { onclick: () => { PM.ProjectsScreen.hide(); PM.newProject(); } }, PM.icon('plus'), 'New Project'),
      h('button.iconbtn', { title: 'Back to editor (Esc)', onclick: close }, PM.icon('x'))),
    S.grid);
  document.body.appendChild(S.el);
  S.el.addEventListener('keydown', e => e.stopPropagation());
}

function pickPmv() {
  const inp = h('input', { type: 'file', accept: '.pmv,.json,application/json' });
  inp.onchange = async () => {
    const f = inp.files[0];
    if (!f) return;
    try {
      const o = JSON.parse(await f.text());
      PM.ProjectsScreen.hide();
      // reuse the app-level importer so registry + tabs stay consistent
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: o.proj || o }));
    } catch (e) { PM.toast('Could not import: ' + e.message); }
  };
  return inp;
}

function paint() {
  S.grid.textContent = '';
  const metas = [...PM.Projects.list()].sort((a, b) => b.at - a.at);
  metas.forEach(m => S.grid.appendChild(card(m)));
  if (!metas.length) {
    S.grid.appendChild(h('div.empty', { style: { gridColumn: '1 / -1' } },
      'No projects yet.\nCreate one with “New Project” or drop a .pmv file here.'));
  }
}

function card(m) {
  const active = m.id === PM.proj.id;
  const img = h('div.ps-thumb');
  if (m.thumb) img.appendChild(h('img', { src: m.thumb, alt: '' }));
  const c = h('div.ps-card' + (active ? '.active' : ''),
    img,
    h('div.ps-meta',
      h('div.ps-name', m.name),
      h('div.ps-sub', active ? 'Open now' : 'Edited ' + ago(m.at))),
    h('div.ps-acts',
      h('button.chip', { onclick: (e) => { e.stopPropagation(); renameDialog(m); } }, 'Rename'),
      h('button.chip', { onclick: (e) => { e.stopPropagation(); duplicate(m); } }, 'Duplicate'),
      h('button.chip', { onclick: (e) => { e.stopPropagation(); removeDialog(m); } }, 'Delete')));
  c.onclick = () => { PM.ProjectsScreen.hide(); window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.Projects.get(m.id) })); };
  return c;
}

function ago(t) {
  const s = Math.max(1, (Date.now() - t) / 1000);
  if (s < 90) return 'just now';
  if (s < 3600) return Math.round(s / 60) + ' min ago';
  if (s < 86400) return Math.round(s / 3600) + ' h ago';
  return new Date(t).toLocaleDateString();
}

function renameDialog(m) {
  const raw = PM.Projects.get(m.id);
  const name = h('input', { value: raw && raw.name || m.name });
  PM.modal({ title: 'Rename project', body: h('div.field', name), width: 400, actions: [
    { label: 'Cancel' },
    { label: 'Rename', pri: true, run: () => {
      const v = name.value.trim() || m.name;
      if (raw) { raw.name = v; try { PM.store.set(PM.Projects.SLOT + m.id, JSON.stringify(raw)); } catch (e) { } }
      PM.Projects.upsertMeta({ id: m.id, name: v, at: Date.now() });
      if (m.id === PM.proj.id) PM.proj.name = v;
      paint(); PM.bus.emit('projects:tabs'); PM.bus.emit('project');
    } },
  ] });
  setTimeout(() => { name.focus(); name.select(); }, 30);
}

function duplicate(m) {
  const raw = PM.Projects.get(m.id);
  if (!raw) return PM.toast('Project data missing');
  raw.id = PM.uid('P');
  raw.name = (m.name || 'Untitled') + ' copy';
  try {
    PM.store.set(PM.Projects.SLOT + raw.id, JSON.stringify(raw));
    PM.Projects.upsertMeta({ id: raw.id, name: raw.name, at: Date.now(), thumb: m.thumb });
    paint(); PM.bus.emit('projects:tabs');
    PM.toast('Duplicated "' + m.name + '"');
  } catch (e) { PM.toast('Duplicate failed: ' + e.message); }
}

function removeDialog(m) {
  PM.modal({ title: 'Delete "' + m.name + '"?', body: h('div', { style: { color: 'var(--tx-2)', fontSize: '12.5px', lineHeight: 1.6 } },
    'This removes the project from Powermove permanently. Exported .pmv files are not affected.'), width: 420, actions: [
    { label: 'Cancel' },
    { label: 'Delete', pri: true, run: () => {
      PM.Projects.remove(m.id);
      if (m.id === PM.proj.id) {
        const rest = PM.Projects.tabs();
        if (rest.length) window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.Projects.get(rest[0]) }));
        else switchOrBlank();
      }
      paint(); PM.bus.emit('projects:tabs');
    } },
  ] });
}
function switchOrBlank() {
  window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Untitled', dur: 10, w: 1920, h: 1080, fps: 30, bg: '#09090A' }) }));
}
})();
