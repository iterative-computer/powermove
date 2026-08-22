/* Powermove — production project library. Persistent project chrome stays
   visible while this Home surface owns search, layouts and recoverable Trash. */
(() => {
const PM = window.PM, h = PM.h;
const S = {
  el: null, nav: null, grid: null, title: null, count: null, search: null,
  section: PM.store.get('projectsSection', 'recents'),
  view: PM.store.get('projectsView', 'grid'),
  sort: PM.store.get('projectsSort', 'recent'),
};
if (!['recents', 'projects', 'trash'].includes(S.section)) S.section = 'recents';

PM.ProjectsScreen = {
  get isOpen() { return !!S.el && S.el.classList.contains('on'); },
  show(section) {
    ensure();
    if (section && ['recents', 'projects', 'trash'].includes(section)) S.section = section;
    paint(); S.el.classList.add('on'); PM.bus.emit('projects:screen');
  },
  hide() { if (S.el) S.el.classList.remove('on'); PM.bus.emit('projects:screen'); },
  toggle() { this.isOpen ? this.hide() : this.show(); },
};

function ensure() {
  if (S.el) return;
  S.search = h('input', { type: 'search', placeholder: 'Search projects', 'aria-label': 'Search projects' });
  S.search.addEventListener('input', paint);
  S.nav = h('div.ps-nav');
  const sidebar = h('aside.ps-sidebar', h('label.ps-search', PM.icon('search'), S.search), S.nav,
    h('div.ps-sidefoot', 'Projects autosave locally. Deleted work stays in Trash until you remove it forever.'));

  S.title = h('b'); S.count = h('span');
  const view = h('div.ps-view', { role: 'group', 'aria-label': 'Project layout' },
    viewButton('grid', 'Grid view', 'grid'), viewButton('list', 'List view', 'list'));
  const sort = h('select.ps-sort', { 'aria-label': 'Sort projects' },
    h('option', { value: 'recent' }, 'Recently edited'), h('option', { value: 'name' }, 'Name'));
  sort.value = S.sort;
  sort.onchange = () => { S.sort = sort.value; PM.store.set('projectsSort', S.sort); paint(); };
  const top = h('div.ps-top', h('div.ps-title', S.title, S.count), view, sort,
    h('button.chip', { onclick: () => pickPmv().click() }, 'Import…'),
    h('button.btn.pri', { onclick: () => { PM.ProjectsScreen.hide(); PM.newProject(); } }, PM.icon('plus'), 'New Project'));
  S.grid = h('div.ps-grid');
  S.el = h('div#projects-screen', sidebar, h('main.ps-main', top, h('div.ps-content', S.grid)));
  document.body.appendChild(S.el);
  S.el.addEventListener('keydown', e => {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); PM.ProjectsScreen.hide(); }
  });
  S.el.addEventListener('dragover', e => { if ([...e.dataTransfer.types].includes('Files')) e.preventDefault(); });
  S.el.addEventListener('drop', async e => {
    const file = [...e.dataTransfer.files].find(f => /\.(pmv|json)$/i.test(f.name));
    if (!file) return;
    e.preventDefault(); await importFile(file);
  });
}

function viewButton(value, label, icon) {
  const b = h('button', { title: label, 'aria-label': label, onclick: () => {
    S.view = value; PM.store.set('projectsView', value); paint();
  } }, PM.icon(icon));
  b.dataset.view = value;
  return b;
}
function navButton(section, label, icon, count) {
  return h('button.ps-navbtn' + (S.section === section ? '.on' : ''), {
    onclick: () => { S.section = section; PM.store.set('projectsSection', section); paint(); },
  }, PM.icon(icon), h('span', label), h('span.count', String(count)));
}

function paint() {
  if (!S.el) return;
  const live = [...PM.Projects.list()], trash = [...PM.Projects.trashList()];
  S.nav.textContent = '';
  S.nav.append(navButton('recents', 'Recents', 'clock', Math.min(12, live.length)),
    navButton('projects', 'All Projects', 'project', live.length), navButton('trash', 'Trash', 'trash', trash.length));
  const labels = { recents: 'Recents', projects: 'Projects', trash: 'Trash' };
  let metas = S.section === 'trash' ? trash : live;
  if (S.sort === 'name') metas.sort((a, b) => String(a.name).localeCompare(String(b.name)));
  else metas.sort((a, b) => (b.at || b.deletedAt || 0) - (a.at || a.deletedAt || 0));
  if (S.section === 'recents') metas = metas.slice(0, 12);
  const q = S.search.value.trim().toLowerCase();
  if (q) metas = metas.filter(m => String(m.name || '').toLowerCase().includes(q));
  S.title.textContent = labels[S.section];
  S.count.textContent = `${metas.length} ${metas.length === 1 ? 'project' : 'projects'}`;
  S.el.querySelectorAll('.ps-view button').forEach(b => b.classList.toggle('on', b.dataset.view === S.view));
  S.grid.className = 'ps-grid' + (S.view === 'list' ? ' list' : '') + (!metas.length ? ' empty' : '');
  S.grid.textContent = '';
  metas.forEach(m => S.grid.appendChild(card(m, S.section === 'trash')));
  if (!metas.length) S.grid.appendChild(emptyState(q));
}

function card(m, trashed) {
  const active = !trashed && m.id === PM.proj.id;
  const raw = PM.Projects.get(m.id);
  const inner = h('div.ps-thumb-inner');
  if (m.thumb) inner.appendChild(h('img', { src: m.thumb, alt: '' }));
  const thumb = h('div.ps-thumb', inner, active ? h('span.ps-card-badge', 'Open') : null);
  const more = h('button.ps-more', { title: 'Project actions', 'aria-label': 'Project actions' }, PM.icon('more'));
  const meta = h('div.ps-meta', h('div.ps-meta-copy', h('div.ps-name', m.name || 'Untitled'),
    h('div.ps-sub', projectSub(m, raw, trashed))), more);
  const c = h('article.ps-card' + (active ? '.active' : ''), thumb, meta);
  c.onclick = () => {
    if (trashed) return;
    PM.ProjectsScreen.hide(); window.dispatchEvent(new CustomEvent('pm-open-project', { detail: raw }));
  };
  c.oncontextmenu = e => { e.preventDefault(); projectMenu(c, m, trashed, e.clientX, e.clientY); };
  more.onclick = e => { e.stopPropagation(); projectMenu(more, m, trashed); };
  return c;
}
function projectSub(m, raw, trashed) {
  const dims = raw && raw.w && raw.h ? `${raw.w}×${raw.h}` : '';
  const date = trashed ? `Deleted ${ago(m.deletedAt)}` : `Edited ${ago(m.at)}`;
  return dims ? `${date} · ${dims}` : date;
}
function projectMenu(anchor, m, trashed, x, y) {
  const items = trashed ? [
    { label: 'Restore', run: () => { PM.Projects.restore(m.id); paint(); PM.bus.emit('projects:tabs'); PM.toast('Project restored'); } },
    '-', { label: 'Delete Forever…', run: () => destroyDialog(m) },
  ] : [
    { label: 'Open', disabled: m.id === PM.proj.id, run: () => { PM.ProjectsScreen.hide(); window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.Projects.get(m.id) })); } },
    { label: 'Rename…', run: () => renameDialog(m) }, { label: 'Duplicate', run: () => duplicate(m) },
    '-', { label: 'Move to Trash…', run: () => trashDialog(m) },
  ];
  PM.menu(anchor, items, x == null ? {} : { x, y });
}

function emptyState(searching) {
  const trash = S.section === 'trash';
  const title = searching ? 'No matching projects' : trash ? 'Trash is empty' : 'No projects yet';
  const body = searching ? 'Try a different name.' : trash
    ? 'Projects moved to Trash stay here until you restore or permanently delete them.'
    : 'Projects keep the composition, layers, reusable sections and exports together.';
  return h('div.ps-empty', PM.icon(trash ? 'trash' : searching ? 'search' : 'project'), h('b', title), h('span', body),
    !searching && !trash ? h('button.btn', { onclick: () => { PM.ProjectsScreen.hide(); PM.newProject(); } }, PM.icon('plus'), 'Create a project') : null);
}

function pickPmv() {
  const inp = h('input', { type: 'file', accept: '.pmv,.json,application/json' });
  inp.onchange = async () => { if (inp.files[0]) await importFile(inp.files[0]); };
  return inp;
}
async function importFile(file) {
  try {
    const o = JSON.parse(await file.text()), p = o.proj || o;
    if (!p || !Array.isArray(p.layers)) throw new Error('Not a Powermove project');
    PM.ProjectsScreen.hide(); window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p }));
    PM.toast('Imported ' + file.name);
  } catch (e) { PM.toast('Could not import: ' + e.message, 4500); }
}

function renameDialog(m) {
  const raw = PM.Projects.get(m.id), name = h('input', { value: raw && raw.name || m.name });
  PM.modal({ title: 'Rename project', body: h('div.field', name), width: 400, actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => {
      const v = name.value.trim() || m.name;
      if (raw) { raw.name = v; PM.Projects.put(raw); } else PM.Projects.upsertMeta({ id: m.id, name: v, at: Date.now() });
      if (m.id === PM.proj.id) PM.proj.name = v;
      paint(); PM.bus.emit('projects:tabs'); PM.bus.emit('project');
    } },
  ] });
  setTimeout(() => { name.focus(); name.select(); }, 30);
}
function duplicate(m) {
  const source = PM.Projects.get(m.id);
  if (!source) return PM.toast('Project data missing');
  const raw = JSON.parse(JSON.stringify(source));
  raw.id = PM.uid('P'); raw.name = (m.name || 'Untitled') + ' copy';
  PM.Projects.put(raw, m.thumb); paint(); PM.bus.emit('projects:tabs'); PM.toast('Duplicated “' + m.name + '”');
}
function trashDialog(m) {
  PM.modal({ title: 'Move “' + m.name + '” to Trash?', body: h('div', { style: { color: 'var(--tx-2)', fontSize: '12.5px', lineHeight: 1.6 } },
    'You can restore this project from Trash.'), width: 420, actions: [
    { label: 'Cancel' }, { label: 'Move to Trash', pri: true, run: () => {
      PM.Projects.trash(m.id); if (m.id === PM.proj.id) switchUnderlying(); paint(); PM.bus.emit('projects:tabs');
    } },
  ] });
}
function destroyDialog(m) {
  PM.modal({ title: 'Delete “' + m.name + '” forever?', body: h('div', { style: { color: 'var(--tx-2)', fontSize: '12.5px', lineHeight: 1.6 } },
    'This permanently removes the local project. This cannot be undone.'), width: 420, actions: [
    { label: 'Cancel' }, { label: 'Delete Forever', pri: true, run: () => { PM.Projects.destroy(m.id); paint(); } },
  ] });
}
function switchUnderlying() {
  const id = PM.Projects.tabs()[0] || (PM.Projects.list()[0] || {}).id;
  const p = id && PM.Projects.get(id);
  window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p || PM.mkProject({ name: 'Untitled', dur: 10, w: 1920, h: 1080, fps: 30, bg: '#09090A' }) }));
}
function ago(t) {
  const s = Math.max(1, (Date.now() - (t || 0)) / 1000);
  if (s < 90) return 'just now'; if (s < 3600) return Math.round(s / 60) + ' min ago';
  if (s < 86400) return Math.round(s / 3600) + ' h ago'; if (s < 604800) return Math.round(s / 86400) + ' d ago';
  return new Date(t).toLocaleDateString();
}
})();
