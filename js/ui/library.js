/* Powermove — a simple, searchable home for editable Sections, Looks and Workspaces. */
(() => {
const PM = window.PM, h = PM.h;
const state = {
  overlay: null, root: null, nav: null, content: null, title: null, count: null,
  search: null, action: null, view: 'sections', scope: 'project', query: '',
  originProjectId: null, lastFocus: null,
};

const VIEWS = {
  sections: { label: 'Sections', icon: 'layers', copy: 'Editable layer groups you can drop into any composition.' },
  looks: { label: 'Looks', icon: 'wand', copy: 'Shader styles you can apply in one click.' },
  workspaces: { label: 'Workspaces', icon: 'panel', copy: 'Panel layouts for different ways of working.' },
  trash: { label: 'Trash', icon: 'trash', copy: 'Restore anything you removed.' },
};

const button = (label, run, pri = false, icon = null, attrs = {}) => h('button.btn' + (pri ? '.pri' : ''), { ...attrs, onclick: run }, icon ? PM.icon(icon) : null, label);
const selectedLayers = () => (PM.sel?.layers || []).map(id => PM.L?.(id)).filter(Boolean);
const scopeName = () => state.scope === 'project' ? 'this project' : 'all projects';
const matches = (...values) => !state.query || values.some(value => String(value || '').toLowerCase().includes(state.query));

function ensure() {
  if (state.root) return;
  state.search = h('input', { type: 'search', placeholder: 'Search library', 'aria-label': 'Search library' });
  state.search.addEventListener('input', () => {
    state.query = state.search.value.trim().toLowerCase();
    paintMain();
  });
  state.nav = h('nav.library-nav', { 'aria-label': 'Library categories' });
  const scope = h('div.library-browse', h('span', 'Browse'),
    h('div.library-scope', { role: 'group', 'aria-label': 'Library scope' },
      h('button', { data: { scope: 'project' }, onclick: () => setScope('project') }, 'This project'),
      h('button', { data: { scope: 'global' }, onclick: () => setScope('global') }, 'All projects')));
  const sidebar = h('aside.library-sidebar',
    h('label.library-search', PM.icon('search'), state.search),
    scope,
    state.nav,
    h('p.library-sidefoot', 'Everything stays editable: real layers, shader code, keyframes, and panel layouts.'));

  state.title = h('div.library-page-title', h('b'), h('span'));
  state.count = h('span.library-count');
  state.action = h('div.library-top-action');
  const top = h('div.library-top', state.title, state.count, state.action);
  state.content = h('div.library-content');
  const main = h('main.library-main', top, state.content);
  const head = h('header.library-head',
    h('div.library-title', h('b', 'Library'), h('span', 'Reuse your best work. Keep editing it.')),
    h('button.iconbtn', { title: 'Close Library', 'aria-label': 'Close Library', onclick: close }, PM.icon('x')));
  state.root = h('section#library-screen', { role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Library', tabindex: '-1' }, head, h('div.library-shell', sidebar, main));
  state.overlay = h('div#library-overlay', state.root);
  state.overlay.addEventListener('pointerdown', event => { if (event.target === state.overlay) close(); });
  state.overlay.addEventListener('keydown', trapKeys);
  document.body.appendChild(state.overlay);
}

function open(view = state.view) {
  ensure();
  if (VIEWS[view]) state.view = view;
  state.originProjectId = PM.proj.id;
  state.lastFocus = document.activeElement;
  paint();
  document.getElementById('app').inert = true;
  state.overlay.classList.add('on'); state.root.classList.add('on');
  requestAnimationFrame(() => state.search.focus());
}
function close() {
  if (!state.overlay?.classList.contains('on')) return;
  PM.closeMenus?.();
  state.overlay.classList.remove('on'); state.root.classList.remove('on');
  document.getElementById('app').inert = false;
  const restore = state.lastFocus; state.lastFocus = null; state.originProjectId = null;
  if (restore?.isConnected) requestAnimationFrame(() => restore.focus());
}
function trapKeys(event) {
  if (event.key === 'Escape') { event.preventDefault(); close(); return; }
  if ((event.key === '/' || ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f')) && document.activeElement !== state.search) {
    event.preventDefault(); state.search.focus(); state.search.select(); return;
  }
  if (event.key !== 'Tab') return;
  const focusable = [...state.root.querySelectorAll('button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])')]
    .filter(node => node.offsetParent !== null);
  if (!focusable.length) { event.preventDefault(); state.root.focus(); return; }
  const first = focusable[0], last = focusable[focusable.length - 1];
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
}
function setScope(scope) { state.scope = scope; paint(); }
function setView(view) { state.view = view; paint(); }

function catalogs() {
  return {
    sections: PM.Library.catalog(state.scope, true),
    looks: PM.Library.lookCatalog(state.scope, true),
    workspaces: workspaceList(false),
    trashedWorkspaces: workspaceList(true),
  };
}
function workspaceList(trashed) {
  const source = trashed ? PM.WS.trashList() : PM.WS.list();
  return source.filter(workspace => {
    if (trashed !== !!workspace.deletedAt) return false;
    if (state.scope === 'global') return true;
    if (trashed) return workspace.scope === 'project' && workspace.projectId === PM.proj.id;
    return workspace.id === PM.WS.current.id || (workspace.scope === 'project' && workspace.projectId === PM.proj.id);
  });
}
function counts(catalog) {
  return {
    sections: catalog.sections.filter(item => !item.deletedAt).length,
    looks: catalog.looks.filter(item => !item.deletedAt).length,
    workspaces: catalog.workspaces.length,
    trash: catalog.sections.filter(item => item.deletedAt).length + catalog.looks.filter(item => item.deletedAt).length + catalog.trashedWorkspaces.length,
  };
}
function navButton(view, count) {
  return h('button.library-navbtn' + (state.view === view ? '.on' : ''), {
    'aria-current': state.view === view ? 'page' : null,
    onclick: () => setView(view),
  }, PM.icon(VIEWS[view].icon), h('span', VIEWS[view].label), h('span.count', String(count)));
}
function paint() {
  if (!state.root) return;
  const catalog = catalogs(), total = counts(catalog);
  state.nav.textContent = '';
  state.nav.append(
    h('span.library-nav-label', 'Reusable'), navButton('sections', total.sections), navButton('looks', total.looks),
    h('span.library-nav-label', 'Layout'), navButton('workspaces', total.workspaces),
    h('div.library-nav-spacer'), navButton('trash', total.trash));
  state.root.querySelectorAll('.library-scope button').forEach(item => item.classList.toggle('on', item.dataset.scope === state.scope));
  paintMain(catalog);
}
function paintMain(catalog = catalogs()) {
  const def = VIEWS[state.view];
  state.title.querySelector('b').textContent = def.label;
  state.title.querySelector('span').textContent = def.copy;
  state.action.textContent = '';
  state.content.textContent = '';
  let items = [];
  if (state.view === 'sections') items = catalog.sections.filter(item => !item.deletedAt && matches(item.name, item.sourceProjectName, ...(item.tags || [])));
  if (state.view === 'looks') items = catalog.looks.filter(item => !item.deletedAt && matches(item.name, item.sourceProjectName, 'shader look'));
  if (state.view === 'workspaces') items = catalog.workspaces.filter(item => matches(item.name, workspacePanelNames(item)));
  if (state.view === 'trash') items = [
    ...catalog.sections.filter(item => item.deletedAt && matches(item.name, item.sourceProjectName, 'section')).map(item => ({ kind: 'section', item })),
    ...catalog.looks.filter(item => item.deletedAt && matches(item.name, item.sourceProjectName, 'look')).map(item => ({ kind: 'look', item })),
    ...catalog.trashedWorkspaces.filter(item => matches(item.name, 'workspace')).map(item => ({ kind: 'workspace', item })),
  ];
  state.count.textContent = resultLabel(items.length);
  paintTopAction();
  const editing = state.view === 'sections' && PM.Library.currentEdit();
  if (editing) state.content.appendChild(editingBanner(editing));
  if (items.length) {
    const grid = h('div.library-grid');
    if (state.view === 'sections') items.forEach(item => grid.appendChild(sectionCard(item)));
    if (state.view === 'looks') items.forEach(item => grid.appendChild(lookCard(item)));
    if (state.view === 'workspaces') items.forEach(item => grid.appendChild(workspaceCard(item)));
    if (state.view === 'trash') items.forEach(({ kind, item }) => grid.appendChild(kind === 'section' ? sectionCard(item) : kind === 'look' ? lookCard(item) : workspaceCard(item)));
    state.content.appendChild(grid);
  } else state.content.appendChild(emptyState());
}
function resultLabel(count) {
  if (state.query) return `${count} ${count === 1 ? 'match' : 'matches'}`;
  if (state.view === 'trash') return `${count} ${count === 1 ? 'item' : 'items'}`;
  const noun = { sections: 'section', looks: 'look', workspaces: 'workspace' }[state.view];
  return `${count} ${noun}${count === 1 ? '' : 's'}`;
}
function paintTopAction() {
  if (state.view === 'trash') return;
  if (state.view === 'sections') {
    const edit = PM.Library.currentEdit();
    if (edit) {
      const found = PM.Library.resolveSection(edit.sectionId, edit.sourceProjectId);
      state.action.append(button(`Update ${found?.entry?.name || 'section'}`, () => confirmSectionUpdate(PM.Library.usage(edit.sectionId, edit.sourceProjectId)), true));
    } else {
      const count = selectedLayers().length;
      state.action.append(button(count ? `Save ${count} selected` : 'Save composition', saveSectionDialog, true, 'plus'));
    }
  } else if (state.view === 'looks') state.action.append(button('Save selected look', saveLookDialog, true, 'plus'));
  else state.action.append(button('Save current layout', saveWorkspaceDialog, true, 'plus'));
}

function saveSectionDialog() {
  const layers = selectedLayers();
  const fallback = layers.length === 1 ? layers[0].name : layers.length > 1 ? `${layers.length}-layer section` : `${PM.proj.name || 'Composition'} section`;
  const input = h('input', { value: fallback });
  PM.modal({ title: layers.length ? 'Save selected layers' : 'Save entire composition', width: 430,
    body: h('div.library-save-dialog', h('p', layers.length
      ? `${layers.length} editable layer${layers.length === 1 ? '' : 's'} will be saved together.`
      : 'Every layer in this composition will be saved as one reusable section.'), h('label.library-field-label', 'Name'), h('div.field', input)),
    actions: [{ label: 'Cancel' }, { label: 'Save section', pri: true, run: () => { PM.Library.saveSection(input.value.trim() || fallback, layers.length ? layers.map(layer => layer.id) : null); paint(); } }],
  });
  setTimeout(() => { input.focus(); input.select(); }, 30);
}
function saveLookDialog() {
  const layer = PM.firstSel?.();
  if (!layer || layer.type !== 'shader') { PM.toast('Select a shader layer to save its look'); return; }
  const input = h('input', { value: layer.name || 'Shader look' });
  PM.modal({ title: 'Save shader look', width: 420,
    body: h('div.library-save-dialog', h('p', 'Saves the shader source and every current control value.'), h('label.library-field-label', 'Name'), h('div.field', input)),
    actions: [{ label: 'Cancel' }, { label: 'Save look', pri: true, run: () => { PM.Library.saveLook(input.value.trim() || layer.name, layer); paint(); } }],
  });
  setTimeout(() => { input.focus(); input.select(); }, 30);
}
function saveWorkspaceDialog() {
  const input = h('input', { value: `${PM.WS.current.name.replace(' (edited)', '')} copy` });
  PM.modal({ title: 'Save current layout', width: 420,
    body: h('div.library-save-dialog', h('p', `Saves the current panel arrangement to ${scopeName()}. Your project content is not copied.`), h('label.library-field-label', 'Name'), h('div.field', input)),
    actions: [{ label: 'Cancel' }, { label: 'Save workspace', pri: true, run: () => {
      PM.WS.duplicate(PM.WS.current.id, { name: input.value.trim() || 'Workspace', scope: state.scope, projectId: state.scope === 'project' ? PM.proj.id : null }); paint();
    } }],
  });
  setTimeout(() => { input.focus(); input.select(); }, 30);
}

function sectionMeta(section) {
  const types = (section.tags?.length ? section.tags : [...new Set((section.layers || []).map(layer => layer.type))]).slice(0, 3);
  return `${(section.layers || []).length} layer${(section.layers || []).length === 1 ? '' : 's'}${types.length ? ' · ' + types.join(', ') : ''}`;
}
function sectionPreview(section) {
  return h('div.library-thumb', section.thumb
    ? h('img', { src: section.thumb, alt: '' })
    : h('div.library-section-map', ...(section.layers || []).slice(0, 6).map((layer, index) =>
      h('i', { style: { left: `${10 + index * 8}%`, top: `${12 + (index % 3) * 18}%`, width: `${72 - index * 5}%` } }))));
}
function sectionCard(section) {
  const deleted = !!section.deletedAt;
  const more = h('button.library-more', { title: 'Section actions', 'aria-label': `Actions for ${section.name}` }, PM.icon('more'));
  more.onclick = event => { event.stopPropagation(); sectionMenu(more, section); };
  const action = deleted
    ? button('Restore', () => { PM.Library.restoreSection(section.id, section.sourceProjectId); paint(); }, true)
    : button('Insert', () => { PM.Library.insertSection(section.id, { sourceProjectId: section.sourceProjectId }); close(); }, true);
  const card = h('article.library-card' + (deleted ? '.deleted' : ''),
    sectionPreview(section),
    h('div.library-card-body', h('div.library-card-copy', h('b', section.name), h('span', sectionMeta(section)), h('small', `From ${section.sourceProjectName || 'Untitled'}`)), more),
    h('div.library-card-actions', action));
  card.oncontextmenu = event => { event.preventDefault(); sectionMenu(card, section); };
  return card;
}
function sectionMenu(anchor, section) {
  if (section.deletedAt) { PM.menu(anchor, [{ label: 'Restore', run: () => { PM.Library.restoreSection(section.id, section.sourceProjectId); paint(); } }]); return; }
  PM.menu(anchor, [
    { label: 'Insert into composition', run: () => { PM.Library.insertSection(section.id, { sourceProjectId: section.sourceProjectId }); close(); } },
    { label: 'Open layers to edit', run: () => { PM.Library.openSection(section.id, { sourceProjectId: section.sourceProjectId }); close(); } },
    { label: 'Duplicate to this project', run: () => { PM.Library.duplicateSection(section.id, section.sourceProjectId, PM.proj.id); paint(); } },
    { label: 'Rename…', run: () => renameSection(section) }, '-',
    { label: 'Move to Trash', run: () => { if (PM.Library.trashSection(section.id, section.sourceProjectId)) paint(); } },
  ]);
}
function renameSection(section) {
  const input = h('input', { value: section.name });
  PM.modal({ title: 'Rename section', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => { PM.Library.renameSection(section.id, input.value, section.sourceProjectId); paint(); } },
  ] });
  setTimeout(() => { input.focus(); input.select(); }, 30);
}

function lookCard(look) {
  const deleted = !!look.deletedAt;
  const more = h('button.library-more', { title: 'Look actions', 'aria-label': `Actions for ${look.name}` }, PM.icon('more'));
  more.onclick = event => { event.stopPropagation(); lookMenu(more, look); };
  const card = h('article.library-card.look' + (deleted ? '.deleted' : ''),
    h('div.library-thumb', look.thumb ? h('img', { src: look.thumb, alt: '' }) : h('div.library-look-swatch')),
    h('div.library-card-body', h('div.library-card-copy', h('b', look.name), h('span', 'Shader look'), h('small', `From ${look.sourceProjectName || 'Untitled'}`)), more),
    h('div.library-card-actions', deleted
      ? button('Restore', () => { PM.Library.restoreLook(look.id, look.sourceProjectId); paint(); }, true)
      : button('Apply', () => { PM.Library.applyLook(look.id, { sourceProjectId: look.sourceProjectId }); close(); }, true)));
  card.oncontextmenu = event => { event.preventDefault(); lookMenu(card, look); };
  return card;
}
function lookMenu(anchor, look) {
  if (look.deletedAt) { PM.menu(anchor, [{ label: 'Restore', run: () => { PM.Library.restoreLook(look.id, look.sourceProjectId); paint(); } }]); return; }
  PM.menu(anchor, [
    { label: 'Apply to new shader layer', run: () => { PM.Library.applyLook(look.id, { sourceProjectId: look.sourceProjectId }); close(); } },
    { label: 'Rename…', run: () => renameLook(look) }, '-',
    { label: 'Move to Trash', run: () => { PM.Library.trashLook(look.id, look.sourceProjectId); paint(); } },
  ]);
}
function renameLook(look) {
  const input = h('input', { value: look.name });
  PM.modal({ title: 'Rename look', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => { PM.Library.renameLook(look.id, input.value, look.sourceProjectId); paint(); } },
  ] });
  setTimeout(() => { input.focus(); input.select(); }, 30);
}

function workspacePanelNames(workspace) {
  return (workspace.layout?.docks || []).flatMap(dock => dock.panels || []).map(panel => PM.PANELS[panel.id]?.title || panel.id).join(' ');
}
function workspaceMap(workspace) {
  const docks = (workspace.layout?.docks || []).map(dock => h('div.workspace-map-dock' + (dock.flex ? '.flex' : ''),
    ...(dock.panels || []).map(panel => h('i', { title: PM.PANELS[panel.id]?.title || panel.id }))));
  return h('div.workspace-map', ...docks);
}
function workspaceCard(workspace) {
  const deleted = !!workspace.deletedAt, active = !deleted && workspace.id === PM.WS.current.id;
  const more = h('button.library-more', { title: 'Workspace actions', 'aria-label': `Actions for ${workspace.name}` }, PM.icon('more'));
  more.onclick = event => { event.stopPropagation(); workspaceMenu(more, workspace); };
  const panelCount = (workspace.layout?.docks || []).flatMap(dock => dock.panels || []).length;
  const action = deleted
    ? button('Restore', () => { PM.WS.restore(workspace.id); paint(); }, true)
    : button(active ? 'Active' : 'Apply', () => previewWorkspace(workspace), !active, null, active ? { disabled: true } : {});
  const card = h('article.library-card.workspace' + (active ? '.active' : '') + (deleted ? '.deleted' : ''),
    workspaceMap(workspace), active ? h('span.library-card-badge', 'Current') : null,
    h('div.library-card-body', h('div.library-card-copy', h('b', workspace.name), h('span', workspace.builtin ? 'Built-in workspace' : 'Custom workspace'), h('small', `${panelCount} panel${panelCount === 1 ? '' : 's'}`)), more),
    h('div.library-card-actions', action));
  card.oncontextmenu = event => { event.preventDefault(); workspaceMenu(card, workspace); };
  return card;
}
function workspaceMenu(anchor, workspace) {
  if (workspace.deletedAt) { PM.menu(anchor, [{ label: 'Restore', run: () => { PM.WS.restore(workspace.id); paint(); } }]); return; }
  const items = [
    { label: 'Preview and apply…', run: () => previewWorkspace(workspace) },
    { label: 'Edit panel layout', run: () => beginWorkspaceEdit(workspace) },
    { label: 'Duplicate', run: () => { PM.WS.duplicate(workspace.id, { scope: state.scope, projectId: state.scope === 'project' ? PM.proj.id : null }); paint(); } },
  ];
  items[0].disabled = workspace.id === PM.WS.current.id;
  if (workspace.builtin) items.push('-', { label: 'Reset built-in', run: () => { PM.WS.resetBuiltin(workspace.id); paint(); } });
  else items.push({ label: 'Rename…', run: () => renameWorkspace(workspace) }, '-', { label: 'Move to Trash', run: () => { PM.WS.remove(workspace.id); paint(); } });
  items.push('-', { label: 'Edit validated definition…', run: () => { PM.WS.activate(workspace.id, true); PM.WS.editJSON(); } });
  PM.menu(anchor, items);
}
function previewWorkspace(workspace) {
  PM.modal({ title: 'Apply workspace?', width: 520,
    body: h('div.workspace-preview', workspaceMap(workspace), h('b', workspace.name), h('p', 'Only the panel arrangement changes. Your composition, layers, and timeline stay untouched.')),
    actions: [{ label: 'Cancel' }, { label: 'Apply workspace', pri: true, run: () => { PM.WS.activate(workspace.id); close(); } }],
  });
}
function renameWorkspace(workspace) {
  const input = h('input', { value: workspace.name });
  PM.modal({ title: 'Rename workspace', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => { PM.WS.rename(workspace.id, input.value); paint(); } },
  ] });
  setTimeout(() => { input.focus(); input.select(); }, 30);
}
function beginWorkspaceEdit(workspace) {
  if (!PM.WS.beginEdit(workspace.id)) return;
  close(); WorkspaceEditor.show();
}

function editingBanner(session) {
  const found = PM.Library.resolveSection(session.sectionId, session.sourceProjectId);
  const uses = PM.Library.usage(session.sectionId, session.sourceProjectId);
  return h('div.library-editing',
    h('div', h('span.library-editing-kicker', 'Editing section'), h('b', found?.entry?.name || 'Section'),
      h('span', uses ? `${uses} placed layer${uses === 1 ? '' : 's'} will stay unchanged until you replace them.` : 'Save these composition layers back to the reusable section.')),
    button('Save as new', saveSectionCopy), button('Update section', () => confirmSectionUpdate(uses), true));
}
function saveSectionCopy() {
  const input = h('input', { value: 'Section copy' });
  PM.modal({ title: 'Save as new section', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Save as new', pri: true, run: () => { PM.Library.saveEditSession(true, input.value); paint(); } },
  ] });
  setTimeout(() => { input.focus(); input.select(); }, 30);
}
function confirmSectionUpdate(uses) {
  if (!uses) { PM.Library.saveEditSession(false); paint(); return; }
  PM.modal({ title: 'Update section?', width: 500,
    body: h('div.library-decision', h('b', `${uses} placed layer${uses === 1 ? '' : 's'} use this section.`),
      h('p', 'The reusable section will be updated. Existing placed layers remain exactly as edited; Powermove will not silently overwrite them.')),
    actions: [{ label: 'Cancel' }, { label: 'Update section only', pri: true, run: () => { PM.Library.saveEditSession(false); paint(); } }],
  });
}

function emptyState() {
  if (state.query) return empty('search', 'No matches', `Nothing in ${VIEWS[state.view].label.toLowerCase()} matches “${state.search.value.trim()}”.`);
  if (state.view === 'trash') return empty('trash', 'Trash is empty', `Removed items from ${scopeName()} will appear here.`);
  if (state.view === 'sections') {
    const count = selectedLayers().length;
    return empty('layers', 'Build your reusable kit', count
      ? `Save the ${count} selected layer${count === 1 ? '' : 's'}, then insert ${count === 1 ? 'it' : 'them'} into any composition without flattening.`
      : 'Select a few layers to save them together, or save the entire composition as one editable section.',
      count ? `Save ${count} selected` : 'Save composition', saveSectionDialog);
  }
  if (state.view === 'looks') return empty('wand', 'Save a shader look', 'Select a shader layer to keep its source and control values ready to reuse.', 'Save selected look', saveLookDialog);
  return empty('panel', 'Save a workspace', 'Keep a panel arrangement for editing, animation, review, or any setup you use often.', 'Save current layout', saveWorkspaceDialog);
}
function empty(icon, title, copy, actionLabel, run) {
  return h('div.library-empty', PM.icon(icon), h('b', title), h('span', copy), actionLabel ? button(actionLabel, run, false, 'plus') : null);
}

const WorkspaceEditor = {
  el: null,
  show() {
    this.el?.remove();
    const add = h('button.btn', { onclick: () => {
      PM.menu(add, Object.values(PM.PANELS).filter(panel => panel.id !== 'toolbar' && !PM.Layout.hasPanel(PM.WS.current, panel.id))
        .map(panel => ({ label: panel.title, run: () => PM.WS.mutate(workspace => PM.Layout.addPanel(workspace, panel.id, 'right')) })));
    } }, 'Add panel');
    const remove = h('button.btn', { onclick: () => {
      const panels = (PM.WS.current.layout?.docks || []).flatMap(dock => dock.panels || []).filter(panel => panel.id !== 'viewer');
      const hidden = PM.WS.current.hiddenPanels || [];
      PM.menu(remove, [
        ...panels.map(panel => ({ label: 'Hide ' + (PM.PANELS[panel.id]?.title || panel.id), run: () => PM.WS.mutate(workspace => PM.Layout.hidePanel(workspace, panel.id)) })),
        ...(panels.length && hidden.length ? ['-'] : []),
        ...hidden.filter(item => PM.PANELS[item.id]).map(item => ({ label: 'Restore ' + PM.PANELS[item.id].title, run: () => PM.WS.mutate(workspace => PM.Layout.restorePanel(workspace, item.id)) })),
      ]);
    } }, 'Remove panel');
    this.el = h('div#workspace-editbar',
      h('div', h('b', 'Editing workspace layout'), h('span', 'Drag panel handles and dividers. Composition always stays reachable.')),
      add, remove,
      button('Cancel', () => { PM.WS.cancelEdit(); this.hide(); }),
      button('Save as new', () => this.save(true)),
      button('Save', () => { PM.WS.saveEdit(false); this.hide(); }, true));
    document.body.appendChild(this.el);
  },
  save(asNew) {
    const input = h('input', { value: PM.WS.current.name.replace(' (edited)', '') + ' copy' });
    PM.modal({ title: 'Save workspace as new', body: h('div.field', input), actions: [
      { label: 'Cancel' }, { label: 'Save as new', pri: true, run: () => { PM.WS.saveEdit(asNew, input.value); this.hide(); } },
    ] });
  },
  hide() { this.el?.remove(); this.el = null; },
};

PM.LibraryUI = { open, close, get isOpen() { return !!state.root?.classList.contains('on'); } };
PM.WorkspaceEditor = WorkspaceEditor;
PM.bus.on('library', () => { if (state.root?.classList.contains('on')) paint(); });
PM.bus.on('workspaces', () => { if (state.root?.classList.contains('on')) paint(); });
PM.bus.on('project', () => {
  if (state.overlay?.classList.contains('on') && state.originProjectId !== PM.proj.id) close();
});
})();
