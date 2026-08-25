/* Ported from js/ui/library.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const h: any = PM.h;
const state: any = {
  overlay: null, root: null, nav: null, content: null, title: null, count: null,
  search: null, action: null, view: 'sections', scope: 'project', query: '',
  originProjectId: null, lastFocus: null, navButtons: {},
};

const VIEWS: any = {
  sections: { label: 'Sections', icon: 'layers', copy: 'Editable layer groups you can drop into any composition.' },
  looks: { label: 'Looks', icon: 'wand', copy: 'Shader styles you can apply in one click.' },
  workspaces: { label: 'Workspaces', icon: 'panel', copy: 'Panel layouts for different ways of working.' },
  trash: { label: 'Trash', icon: 'trash', copy: 'Restore anything you removed.' },
};

const button: any = (label?: any, run?: any, pri: any = false, icon: any = null, attrs: any = {}) => h('button.btn' + (pri ? '.pri' : ''), { ...attrs, onclick: run }, icon ? PM.icon(icon) : null, label);
const selectedLayers: any = () => (PM.sel?.layers || []).map((id: any) => PM.L?.(id)).filter(Boolean);
const scopeName: any = () => state.scope === 'project' ? 'this project' : 'all projects';
const matches: any = (...values: any[]) => !state.query || values.some((value: any) => String(value || '').toLowerCase().includes(state.query));

function ensure() {
  if (state.root) return;
  state.search = h('input', { type: 'search', placeholder: 'Search library', 'aria-label': 'Search library' });
  state.search.addEventListener('input', () => {
    state.query = state.search.value.trim().toLowerCase();
    paintMain();
  });
  state.nav = h('nav.library-nav', { 'aria-label': 'Library categories' });
  state.nav.append(
    h('span.library-nav-label', 'Reusable'), navButton('sections'), navButton('looks'),
    h('span.library-nav-label', 'Layout'), navButton('workspaces'),
    h('div.library-nav-spacer'), navButton('trash'));
  const scope: any = h('div.library-browse', h('span', 'Browse'),
    h('div.library-scope', { role: 'group', 'aria-label': 'Library scope' },
      h('button', { data: { scope: 'project' }, onclick: () => setScope('project') }, 'This project'),
      h('button', { data: { scope: 'global' }, onclick: () => setScope('global') }, 'All projects')));
  const sidebar: any = h('aside.library-sidebar',
    h('label.library-search', PM.icon('search'), state.search),
    scope,
    state.nav,
    h('p.library-sidefoot', 'Everything stays editable: real layers, shader code, keyframes, and panel layouts.'));

  state.title = h('div.library-page-title', h('b'), h('span'));
  state.count = h('span.library-count');
  state.action = h('div.library-top-action');
  const top: any = h('div.library-top', state.title, state.count, state.action);
  state.content = h('div.library-content');
  const main: any = h('main.library-main', top, state.content);
  const head: any = h('header.library-head',
    h('div.library-title', h('b', 'Library'), h('span', 'Reuse your best work. Keep editing it.')),
    h('button.iconbtn', { title: 'Close Library', 'aria-label': 'Close Library', onclick: close }, PM.icon('x')));
  state.root = h('section#library-screen', { role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Library', tabindex: '-1' }, head, h('div.library-shell', sidebar, main));
  state.overlay = h('div#library-overlay', state.root);
  state.overlay.addEventListener('pointerdown', (event: any) => { if (event.target === state.overlay) close(); });
  state.overlay.addEventListener('keydown', trapKeys);
  window.document.body.appendChild(state.overlay);
}

function open(view: any = state.view) {
  ensure();
  if (VIEWS[view]) state.view = view;
  state.originProjectId = PM.proj.id;
  state.lastFocus = window.document.activeElement;
  paint();
  (window.document.getElementById('app') as any).inert = true;
  state.overlay.classList.add('on'); state.root.classList.add('on');
  window.requestAnimationFrame(() => state.search.focus());
}
function close() {
  if (!state.overlay?.classList.contains('on')) return;
  PM.closeMenus?.();
  state.overlay.classList.remove('on'); state.root.classList.remove('on');
  (window.document.getElementById('app') as any).inert = false;
  const restore: any = state.lastFocus; state.lastFocus = null; state.originProjectId = null;
  if (restore?.isConnected) window.requestAnimationFrame(() => restore.focus());
}
function trapKeys(event?: any) {
  if (event.key === 'Escape') { event.preventDefault(); close(); return; }
  if ((event.key === '/' || ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f')) && window.document.activeElement !== state.search) {
    event.preventDefault(); state.search.focus(); state.search.select(); return;
  }
  if (event.key !== 'Tab') return;
  const focusable: any = [...state.root.querySelectorAll('button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])')]
    .filter((node: any) => node.offsetParent !== null);
  if (!focusable.length) { event.preventDefault(); state.root.focus(); return; }
  const first: any = focusable[0], last: any = focusable[focusable.length - 1];
  if (event.shiftKey && window.document.activeElement === first) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && window.document.activeElement === last) { event.preventDefault(); first.focus(); }
}
function setScope(scope?: any) { state.scope = scope; paint(); }
function setView(view?: any) {
  if (!VIEWS[view]) return;
  state.view = view;
  paint();
}

function catalogs() {
  return {
    sections: PM.Library.catalog(state.scope, true),
    looks: PM.Library.lookCatalog(state.scope, true),
    workspaces: workspaceList(false),
    trashedWorkspaces: workspaceList(true),
  };
}
function workspaceList(trashed?: any) {
  const source: any = trashed ? PM.WS.trashList() : PM.WS.list();
  return source.filter((workspace: any) => {
    if (trashed !== !!workspace.deletedAt) return false;
    if (state.scope === 'global') return true;
    if (trashed) return workspace.scope === 'project' && workspace.projectId === PM.proj.id;
    return workspace.id === PM.WS.current.id || (workspace.scope === 'project' && workspace.projectId === PM.proj.id);
  });
}
function counts(catalog?: any) {
  return {
    sections: catalog.sections.filter((item: any) => !item.deletedAt).length,
    looks: catalog.looks.filter((item: any) => !item.deletedAt).length,
    workspaces: catalog.workspaces.length,
    trash: catalog.sections.filter((item: any) => item.deletedAt).length + catalog.looks.filter((item: any) => item.deletedAt).length + catalog.trashedWorkspaces.length,
  };
}
function navButton(view?: any) {
  const count: any = h('span.count', '0');
  const element: any = h('button.library-navbtn', {
    type: 'button',
    onclick: () => setView(view),
  }, PM.icon(VIEWS[view].icon), h('span', VIEWS[view].label), count);
  state.navButtons[view] = { element, count };
  return element;
}
function paint() {
  if (!state.root) return;
  const catalog: any = catalogs(), total: any = counts(catalog);
  Object.entries(state.navButtons).forEach(([view, item]: any) => {
    const active: any = state.view === view;
    item.element.classList.toggle('on', active);
    if (active) item.element.setAttribute('aria-current', 'page');
    else item.element.removeAttribute('aria-current');
    item.count.textContent = String(total[view]);
  });
  state.root.querySelectorAll('.library-scope button').forEach((item: any) => item.classList.toggle('on', item.dataset.scope === state.scope));
  paintMain(catalog);
}
function paintMain(catalog: any = catalogs()) {
  const def: any = VIEWS[state.view];
  state.title.querySelector('b').textContent = def.label;
  state.title.querySelector('span').textContent = def.copy;
  state.action.textContent = '';
  state.content.textContent = '';
  let items: any = [];
  if (state.view === 'sections') items = catalog.sections.filter((item: any) => !item.deletedAt && matches(item.name, item.sourceProjectName, ...(item.tags || [])));
  if (state.view === 'looks') items = catalog.looks.filter((item: any) => !item.deletedAt && matches(item.name, item.sourceProjectName, 'shader look'));
  if (state.view === 'workspaces') items = catalog.workspaces.filter((item: any) => matches(item.name, workspacePanelNames(item)));
  if (state.view === 'trash') items = [
    ...catalog.sections.filter((item: any) => item.deletedAt && matches(item.name, item.sourceProjectName, 'section')).map((item: any) => ({ kind: 'section', item })),
    ...catalog.looks.filter((item: any) => item.deletedAt && matches(item.name, item.sourceProjectName, 'look')).map((item: any) => ({ kind: 'look', item })),
    ...catalog.trashedWorkspaces.filter((item: any) => matches(item.name, 'workspace')).map((item: any) => ({ kind: 'workspace', item })),
  ];
  state.count.textContent = resultLabel(items.length);
  paintTopAction();
  const editing: any = state.view === 'sections' && PM.Library.currentEdit();
  if (editing) state.content.appendChild(editingBanner(editing));
  if (items.length) {
    const grid: any = h('div.library-grid');
    if (state.view === 'sections') items.forEach((item: any) => grid.appendChild(sectionCard(item)));
    if (state.view === 'looks') items.forEach((item: any) => grid.appendChild(lookCard(item)));
    if (state.view === 'workspaces') items.forEach((item: any) => grid.appendChild(workspaceCard(item)));
    if (state.view === 'trash') items.forEach(({ kind, item }: any) => grid.appendChild(kind === 'section' ? sectionCard(item) : kind === 'look' ? lookCard(item) : workspaceCard(item)));
    state.content.appendChild(grid);
  } else state.content.appendChild(emptyState());
}
function resultLabel(count?: any) {
  if (state.query) return `${count} ${count === 1 ? 'match' : 'matches'}`;
  if (state.view === 'trash') return `${count} ${count === 1 ? 'item' : 'items'}`;
  const noun: any = ({ sections: 'section', looks: 'look', workspaces: 'workspace' } as any)[state.view];
  return `${count} ${noun}${count === 1 ? '' : 's'}`;
}
function paintTopAction() {
  if (state.view === 'trash') return;
  if (state.view === 'sections') {
    const edit: any = PM.Library.currentEdit();
    if (edit) {
      const found: any = PM.Library.resolveSection(edit.sectionId, edit.sourceProjectId);
      state.action.append(button(`Update ${found?.entry?.name || 'section'}`, () => confirmSectionUpdate(PM.Library.usage(edit.sectionId, edit.sourceProjectId)), true));
    } else {
      const count: any = selectedLayers().length;
      state.action.append(button(count ? `Save ${count} selected` : 'Save composition', saveSectionDialog, true, 'plus'));
    }
  } else if (state.view === 'looks') state.action.append(button('Save selected look', saveLookDialog, true, 'plus'));
  else state.action.append(button('Save current layout', saveWorkspaceDialog, true, 'plus'));
}

function saveSectionDialog() {
  const layers: any = selectedLayers();
  const fallback: any = layers.length === 1 ? layers[0].name : layers.length > 1 ? `${layers.length}-layer section` : `${PM.proj.name || 'Composition'} section`;
  const input: any = h('input', { value: fallback });
  PM.modal({ title: layers.length ? 'Save selected layers' : 'Save entire composition', width: 430,
    body: h('div.library-save-dialog', h('p', layers.length
      ? `${layers.length} editable layer${layers.length === 1 ? '' : 's'} will be saved together.`
      : 'Every layer in this composition will be saved as one reusable section.'), h('label.library-field-label', 'Name'), h('div.field', input)),
    actions: [{ label: 'Cancel' }, { label: 'Save section', pri: true, run: () => { PM.Library.saveSection(input.value.trim() || fallback, layers.length ? layers.map((layer: any) => layer.id) : null); paint(); } }],
  });
  window.setTimeout(() => { input.focus(); input.select(); }, 30);
}
function saveLookDialog() {
  const layer: any = PM.firstSel?.();
  if (!layer || layer.type !== 'shader') { PM.toast('Select a shader layer to save its look'); return; }
  const input: any = h('input', { value: layer.name || 'Shader look' });
  PM.modal({ title: 'Save shader look', width: 420,
    body: h('div.library-save-dialog', h('p', 'Saves the shader source and every current control value.'), h('label.library-field-label', 'Name'), h('div.field', input)),
    actions: [{ label: 'Cancel' }, { label: 'Save look', pri: true, run: () => { PM.Library.saveLook(input.value.trim() || layer.name, layer); paint(); } }],
  });
  window.setTimeout(() => { input.focus(); input.select(); }, 30);
}
function saveWorkspaceDialog() {
  const input: any = h('input', { value: `${PM.WS.current.name.replace(' (edited)', '')} copy` });
  PM.modal({ title: 'Save current layout', width: 420,
    body: h('div.library-save-dialog', h('p', `Saves the current panel arrangement to ${scopeName()}. Your project content is not copied.`), h('label.library-field-label', 'Name'), h('div.field', input)),
    actions: [{ label: 'Cancel' }, { label: 'Save workspace', pri: true, run: () => {
      PM.WS.duplicate(PM.WS.current.id, { name: input.value.trim() || 'Workspace', scope: state.scope, projectId: state.scope === 'project' ? PM.proj.id : null }); paint();
    } }],
  });
  window.setTimeout(() => { input.focus(); input.select(); }, 30);
}

function sectionMeta(section?: any) {
  const types: any = (section.tags?.length ? section.tags : [...new Set((section.layers || []).map((layer: any) => layer.type))]).slice(0, 3);
  return `${(section.layers || []).length} layer${(section.layers || []).length === 1 ? '' : 's'}${types.length ? ' · ' + types.join(', ') : ''}`;
}
function sectionPreview(section?: any) {
  return h('div.library-thumb', section.thumb
    ? h('img', { src: section.thumb, alt: '' })
    : h('div.library-section-map', ...(section.layers || []).slice(0, 6).map((layer?: any, index?: any) =>
      h('i', { style: { left: `${10 + index * 8}%`, top: `${12 + (index % 3) * 18}%`, width: `${72 - index * 5}%` } }))));
}
function sectionCard(section?: any) {
  const deleted: any = !!section.deletedAt;
  const more: any = h('button.library-more', { title: 'Section actions', 'aria-label': `Actions for ${section.name}` }, PM.icon('more'));
  more.onclick = (event: any) => { event.stopPropagation(); sectionMenu(more, section); };
  const action: any = deleted
    ? button('Restore', () => { PM.Library.restoreSection(section.id, section.sourceProjectId); paint(); }, true)
    : button('Insert', () => { PM.Library.insertSection(section.id, { sourceProjectId: section.sourceProjectId }); close(); }, true);
  const card: any = h('article.library-card' + (deleted ? '.deleted' : ''),
    sectionPreview(section),
    h('div.library-card-body', h('div.library-card-copy', h('b', section.name), h('span', sectionMeta(section)), h('small', `From ${section.sourceProjectName || 'Untitled'}`)), more),
    h('div.library-card-actions', action));
  card.oncontextmenu = (event: any) => { event.preventDefault(); sectionMenu(card, section); };
  return card;
}
function sectionMenu(anchor?: any, section?: any) {
  if (section.deletedAt) { PM.menu(anchor, [{ label: 'Restore', run: () => { PM.Library.restoreSection(section.id, section.sourceProjectId); paint(); } }]); return; }
  PM.menu(anchor, [
    { label: 'Insert into composition', run: () => { PM.Library.insertSection(section.id, { sourceProjectId: section.sourceProjectId }); close(); } },
    { label: 'Open layers to edit', run: () => { PM.Library.openSection(section.id, { sourceProjectId: section.sourceProjectId }); close(); } },
    { label: 'Duplicate to this project', run: () => { PM.Library.duplicateSection(section.id, section.sourceProjectId, PM.proj.id); paint(); } },
    { label: 'Rename…', run: () => renameSection(section) }, '-',
    { label: 'Move to Trash', run: () => { if (PM.Library.trashSection(section.id, section.sourceProjectId)) paint(); } },
  ]);
}
function renameSection(section?: any) {
  const input: any = h('input', { value: section.name });
  PM.modal({ title: 'Rename section', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => { PM.Library.renameSection(section.id, input.value, section.sourceProjectId); paint(); } },
  ] });
  window.setTimeout(() => { input.focus(); input.select(); }, 30);
}

function lookCard(look?: any) {
  const deleted: any = !!look.deletedAt;
  const more: any = h('button.library-more', { title: 'Look actions', 'aria-label': `Actions for ${look.name}` }, PM.icon('more'));
  more.onclick = (event: any) => { event.stopPropagation(); lookMenu(more, look); };
  const card: any = h('article.library-card.look' + (deleted ? '.deleted' : ''),
    h('div.library-thumb', look.thumb ? h('img', { src: look.thumb, alt: '' }) : h('div.library-look-swatch')),
    h('div.library-card-body', h('div.library-card-copy', h('b', look.name), h('span', 'Shader look'), h('small', `From ${look.sourceProjectName || 'Untitled'}`)), more),
    h('div.library-card-actions', deleted
      ? button('Restore', () => { PM.Library.restoreLook(look.id, look.sourceProjectId); paint(); }, true)
      : button('Apply', () => { PM.Library.applyLook(look.id, { sourceProjectId: look.sourceProjectId }); close(); }, true)));
  card.oncontextmenu = (event: any) => { event.preventDefault(); lookMenu(card, look); };
  return card;
}
function lookMenu(anchor?: any, look?: any) {
  if (look.deletedAt) { PM.menu(anchor, [{ label: 'Restore', run: () => { PM.Library.restoreLook(look.id, look.sourceProjectId); paint(); } }]); return; }
  PM.menu(anchor, [
    { label: 'Apply to new shader layer', run: () => { PM.Library.applyLook(look.id, { sourceProjectId: look.sourceProjectId }); close(); } },
    { label: 'Rename…', run: () => renameLook(look) }, '-',
    { label: 'Move to Trash', run: () => { PM.Library.trashLook(look.id, look.sourceProjectId); paint(); } },
  ]);
}
function renameLook(look?: any) {
  const input: any = h('input', { value: look.name });
  PM.modal({ title: 'Rename look', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => { PM.Library.renameLook(look.id, input.value, look.sourceProjectId); paint(); } },
  ] });
  window.setTimeout(() => { input.focus(); input.select(); }, 30);
}

function workspacePanelNames(workspace?: any) {
  return (workspace.layout?.docks || []).flatMap((dock: any) => dock.panels || []).map((panel: any) => PM.PANELS[panel.id]?.title || panel.id).join(' ');
}
function workspaceMap(workspace?: any) {
  const docks: any = (workspace.layout?.docks || []).map((dock: any) => h('div.workspace-map-dock' + (dock.flex ? '.flex' : ''),
    ...(dock.panels || []).map((panel: any) => h('i', { title: PM.PANELS[panel.id]?.title || panel.id }))));
  return h('div.workspace-map', ...docks);
}
function workspaceCard(workspace?: any) {
  const deleted: any = !!workspace.deletedAt, active: any = !deleted && workspace.id === PM.WS.current.id;
  const more: any = h('button.library-more', { title: 'Workspace actions', 'aria-label': `Actions for ${workspace.name}` }, PM.icon('more'));
  more.onclick = (event: any) => { event.stopPropagation(); workspaceMenu(more, workspace); };
  const panelCount: any = (workspace.layout?.docks || []).flatMap((dock: any) => dock.panels || []).length;
  const action: any = deleted
    ? button('Restore', () => { PM.WS.restore(workspace.id); paint(); }, true)
    : button(active ? 'Active' : 'Apply', () => previewWorkspace(workspace), !active, null, active ? { disabled: true } : {});
  const card: any = h('article.library-card.workspace' + (active ? '.active' : '') + (deleted ? '.deleted' : ''),
    workspaceMap(workspace), active ? h('span.library-card-badge', 'Current') : null,
    h('div.library-card-body', h('div.library-card-copy', h('b', workspace.name), h('span', workspace.builtin ? 'Built-in workspace' : 'Custom workspace'), h('small', `${panelCount} panel${panelCount === 1 ? '' : 's'}`)), more),
    h('div.library-card-actions', action));
  card.oncontextmenu = (event: any) => { event.preventDefault(); workspaceMenu(card, workspace); };
  return card;
}
function workspaceMenu(anchor?: any, workspace?: any) {
  if (workspace.deletedAt) { PM.menu(anchor, [{ label: 'Restore', run: () => { PM.WS.restore(workspace.id); paint(); } }]); return; }
  const items: any = [
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
function previewWorkspace(workspace?: any) {
  PM.modal({ title: 'Apply workspace?', width: 520,
    body: h('div.workspace-preview', workspaceMap(workspace), h('b', workspace.name), h('p', 'Only the panel arrangement changes. Your composition, layers, and timeline stay untouched.')),
    actions: [{ label: 'Cancel' }, { label: 'Apply workspace', pri: true, run: () => { PM.WS.activate(workspace.id); close(); } }],
  });
}
function renameWorkspace(workspace?: any) {
  const input: any = h('input', { value: workspace.name });
  PM.modal({ title: 'Rename workspace', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => { PM.WS.rename(workspace.id, input.value); paint(); } },
  ] });
  window.setTimeout(() => { input.focus(); input.select(); }, 30);
}
function beginWorkspaceEdit(workspace?: any) {
  if (!PM.WS.beginEdit(workspace.id)) return;
  close(); WorkspaceEditor.show();
}

function editingBanner(session?: any) {
  const found: any = PM.Library.resolveSection(session.sectionId, session.sourceProjectId);
  const uses: any = PM.Library.usage(session.sectionId, session.sourceProjectId);
  return h('div.library-editing',
    h('div', h('span.library-editing-kicker', 'Editing section'), h('b', found?.entry?.name || 'Section'),
      h('span', uses ? `${uses} placed layer${uses === 1 ? '' : 's'} will stay unchanged until you replace them.` : 'Save these composition layers back to the reusable section.')),
    button('Save as new', saveSectionCopy), button('Update section', () => confirmSectionUpdate(uses), true));
}
function saveSectionCopy() {
  const input: any = h('input', { value: 'Section copy' });
  PM.modal({ title: 'Save as new section', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Save as new', pri: true, run: () => { PM.Library.saveEditSession(true, input.value); paint(); } },
  ] });
  window.setTimeout(() => { input.focus(); input.select(); }, 30);
}
function confirmSectionUpdate(uses?: any) {
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
    const count: any = selectedLayers().length;
    return empty('layers', 'Build your reusable kit', count
      ? `Save the ${count} selected layer${count === 1 ? '' : 's'}, then insert ${count === 1 ? 'it' : 'them'} into any composition without flattening.`
      : 'Select a few layers to save them together, or save the entire composition as one editable section.',
      count ? `Save ${count} selected` : 'Save composition', saveSectionDialog);
  }
  if (state.view === 'looks') return empty('wand', 'Save a shader look', 'Select a shader layer to keep its source and control values ready to reuse.', 'Save selected look', saveLookDialog);
  return empty('panel', 'Save a workspace', 'Keep a panel arrangement for editing, animation, review, or any setup you use often.', 'Save current layout', saveWorkspaceDialog);
}
function empty(icon?: any, title?: any, copy?: any, actionLabel?: any, run?: any) {
  return h('div.library-empty', PM.icon(icon), h('b', title), h('span', copy), actionLabel ? button(actionLabel, run, false, 'plus') : null);
}

const WorkspaceEditor: any = {
  el: null,
  show() {
    this.el?.remove();
    const add: any = h('button.btn', { onclick: () => {
      PM.menu(add, Object.values(PM.PANELS).filter((panel: any) => panel.id !== 'toolbar' && !PM.Layout.hasPanel(PM.WS.current, panel.id))
        .map((panel: any) => ({ label: panel.title, run: () => PM.WS.mutate((workspace: any) => PM.Layout.addPanel(workspace, panel.id, 'right')) })));
    } }, 'Add panel');
    const remove: any = h('button.btn', { onclick: () => {
      const panels: any = (PM.WS.current.layout?.docks || []).flatMap((dock: any) => dock.panels || []).filter((panel: any) => panel.id !== 'viewer');
      const hidden: any = PM.WS.current.hiddenPanels || [];
      PM.menu(remove, [
        ...panels.map((panel: any) => ({ label: 'Hide ' + (PM.PANELS[panel.id]?.title || panel.id), run: () => PM.WS.mutate((workspace: any) => PM.Layout.hidePanel(workspace, panel.id)) })),
        ...(panels.length && hidden.length ? ['-'] : []),
        ...hidden.filter((item: any) => PM.PANELS[item.id]).map((item: any) => ({ label: 'Restore ' + PM.PANELS[item.id].title, run: () => PM.WS.mutate((workspace: any) => PM.Layout.restorePanel(workspace, item.id)) })),
      ]);
    } }, 'Remove panel');
    this.el = h('div#workspace-editbar',
      h('div', h('b', 'Editing workspace layout'), h('span', 'Drag panel handles and dividers. Composition always stays reachable.')),
      add, remove,
      button('Cancel', () => { PM.WS.cancelEdit(); this.hide(); }),
      button('Save as new', () => this.save(true)),
      button('Save', () => { PM.WS.saveEdit(false); this.hide(); }, true));
    window.document.body.appendChild(this.el);
  },
  save(asNew?: any) {
    const input: any = h('input', { value: PM.WS.current.name.replace(' (edited)', '') + ' copy' });
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
}
