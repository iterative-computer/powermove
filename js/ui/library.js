/* Powermove — one compact Library for reusable Sections and Workspaces. */
(() => {
const PM = window.PM, h = PM.h;
const state = { root: null, tab: 'sections', scope: 'project', trash: false };

const button = (label, run, pri = false) => h('button.btn' + (pri ? '.pri' : ''), { onclick: run }, label);
const scopeName = () => state.scope === 'project' ? 'This project' : 'My library';

function open(tab = state.tab) {
  state.tab = tab;
  if (!state.root) {
    state.root = h('section#library-screen', { role: 'dialog', 'aria-label': 'Library' });
    document.body.appendChild(state.root);
  }
  paint(); state.root.classList.add('on');
}
function close() { state.root?.classList.remove('on'); }

function header() {
  const tabs = h('div.library-tabs',
    h('button' + (state.tab === 'sections' ? '.on' : ''), { onclick: () => { state.tab = 'sections'; state.trash = false; paint(); } }, 'Sections'),
    h('button' + (state.tab === 'workspaces' ? '.on' : ''), { onclick: () => { state.tab = 'workspaces'; state.trash = false; paint(); } }, 'Workspaces'));
  const scope = h('div.library-scope',
    h('button' + (state.scope === 'project' ? '.on' : ''), { onclick: () => { state.scope = 'project'; paint(); } }, 'This project'),
    h('button' + (state.scope === 'global' ? '.on' : ''), { onclick: () => { state.scope = 'global'; paint(); } }, 'My library'));
  return h('header.library-head',
    h('div.library-title', h('b', 'Library'), h('span', 'Reusable, editable building blocks')),
    tabs, scope,
    h('button.iconbtn', { title: state.trash ? 'Show Library' : 'Show recoverable Trash', onclick: () => { state.trash = !state.trash; paint(); } }, PM.icon('trash')),
    h('button.iconbtn', { title: 'Close Library', onclick: close }, PM.icon('x')));
}

function sectionMeta(section) {
  const types = (section.tags?.length ? section.tags : [...new Set((section.layers || []).map(layer => layer.type))]).slice(0, 3);
  return [
    `${(section.layers || []).length} layer${(section.layers || []).length === 1 ? '' : 's'}`,
    types.join(' · ') || 'section',
    section.sourceProjectName || 'Untitled',
  ];
}
function sectionMenu(anchor, section) {
  if (section.deletedAt) {
    PM.menu(anchor, [{ label: 'Restore', run: () => { PM.Library.restoreSection(section.id, section.sourceProjectId); paint(); } }]);
    return;
  }
  PM.menu(anchor, [
    { label: 'Open as editable layers', run: () => { close(); PM.Library.openSection(section.id, { sourceProjectId: section.sourceProjectId }); } },
    { label: 'Duplicate to this project', run: () => { PM.Library.duplicateSection(section.id, section.sourceProjectId, PM.proj.id); paint(); } },
    { label: 'Rename…', run: () => renameSection(section) },
    '-',
    { label: 'Move to Trash', run: () => { PM.Library.trashSection(section.id, section.sourceProjectId); paint(); } },
  ]);
}
function renameSection(section) {
  const input = h('input', { value: section.name });
  PM.modal({ title: 'Rename section', body: h('div.field', input), actions: [
    { label: 'Cancel' },
    { label: 'Rename', pri: true, run: () => { PM.Library.renameSection(section.id, input.value, section.sourceProjectId); paint(); } },
  ] });
  setTimeout(() => { input.focus(); input.select(); }, 30);
}
function sectionCard(section) {
  const more = h('button.library-more', { title: 'Section actions' }, PM.icon('more'));
  more.onclick = e => { e.stopPropagation(); sectionMenu(more, section); };
  const thumb = h('div.library-thumb', section.thumb
    ? h('img', { src: section.thumb, alt: '' })
    : h('div.library-section-map', ...(section.layers || []).slice(0, 6).map((layer, index) =>
      h('i', { style: { left: `${10 + index * 8}%`, top: `${12 + (index % 3) * 18}%`, width: `${72 - index * 5}%` } }))));
  const meta = sectionMeta(section);
  const card = h('article.library-card' + (section.deletedAt ? '.deleted' : ''),
    thumb,
    h('div.library-card-copy', h('b', section.name), h('span', meta[0] + ' · ' + meta[1]), h('small', 'From ' + meta[2])),
    more,
    section.deletedAt ? button('Restore', () => { PM.Library.restoreSection(section.id, section.sourceProjectId); paint(); })
      : h('div.library-card-actions',
          button('Insert', () => { PM.Library.insertSection(section.id, { sourceProjectId: section.sourceProjectId }); close(); }, true),
          button('Edit', () => { PM.Library.openSection(section.id, { sourceProjectId: section.sourceProjectId }); close(); })));
  return card;
}

function saveSectionBar() {
  const session = PM.Library.currentEdit();
  if (!session) return h('div.library-create',
    h('div', h('b', 'Save the current selection'), h('span', 'Keeps real layers, properties, and keyframes editable.')),
    button('Save shader look', () => { PM.Library.saveLook(); paint(); }),
    button('Save section', () => {
      const ids = PM.sel?.layers?.length ? PM.sel.layers : null;
      PM.Library.saveSection(null, ids); paint();
    }, true));
  const found = PM.Library.resolveSection(session.sectionId, session.sourceProjectId);
  const uses = PM.Library.usage(session.sectionId, session.sourceProjectId);
  return h('div.library-create.editing',
    h('div', h('b', `Editing ${found?.entry?.name || 'section'}`),
      h('span', uses ? `${uses} placed layer${uses === 1 ? '' : 's'} stay unchanged unless you replace them deliberately.` : 'Save these real composition layers back to the section.')),
    button('Save as new', () => saveSectionCopy()),
    button('Update section', () => confirmSectionUpdate(uses), true));
}

function lookMenu(anchor, look) {
  if (look.deletedAt) {
    PM.menu(anchor, [{ label: 'Restore', run: () => { PM.Library.restoreLook(look.id, look.sourceProjectId); paint(); } }]); return;
  }
  PM.menu(anchor, [
    { label: 'Apply look', run: () => { PM.Library.applyLook(look.id, { sourceProjectId: look.sourceProjectId }); close(); } },
    { label: 'Rename…', run: () => {
      const input = h('input', { value: look.name });
      PM.modal({ title: 'Rename look', body: h('div.field', input), actions: [
        { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => { PM.Library.renameLook(look.id, input.value, look.sourceProjectId); paint(); } },
      ] }); setTimeout(() => { input.focus(); input.select(); }, 30);
    } },
    '-', { label: 'Move to Trash', run: () => { PM.Library.trashLook(look.id, look.sourceProjectId); paint(); } },
  ]);
}
function lookCard(look) {
  const more = h('button.library-more', { title: 'Look actions' }, PM.icon('more'));
  more.onclick = event => { event.stopPropagation(); lookMenu(more, look); };
  return h('article.library-card.look' + (look.deletedAt ? '.deleted' : ''),
    h('div.library-thumb', look.thumb ? h('img', { src: look.thumb, alt: '' }) : h('div.library-look-swatch')),
    h('div.library-card-copy', h('b', look.name), h('span', 'Shader look'), h('small', 'From ' + (look.sourceProjectName || 'Untitled'))),
    more,
    look.deletedAt ? button('Restore', () => { PM.Library.restoreLook(look.id, look.sourceProjectId); paint(); })
      : h('div.library-card-actions', button('Apply', () => { PM.Library.applyLook(look.id, { sourceProjectId: look.sourceProjectId }); close(); }, true)));
}
function saveSectionCopy() {
  const input = h('input', { value: 'Section copy' });
  PM.modal({ title: 'Save as new section', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Save as new', pri: true, run: () => { PM.Library.saveEditSession(true, input.value); paint(); } },
  ] });
}
function confirmSectionUpdate(uses) {
  if (!uses) { PM.Library.saveEditSession(false); paint(); return; }
  PM.modal({
    title: 'Update section?', width: 500,
    body: h('div.library-decision',
      h('b', `${uses} placed layer${uses === 1 ? '' : 's'} use this section.`),
      h('p', 'The reusable section will be updated. Existing placed layers remain exactly as edited; Powermove will not silently overwrite them.')),
    actions: [{ label: 'Cancel' }, { label: 'Update section only', pri: true, run: () => { PM.Library.saveEditSession(false); paint(); } }],
  });
}

function sectionsView() {
  const sections = PM.Library.catalog(state.scope, true).filter(section => state.trash ? !!section.deletedAt : !section.deletedAt);
  const looks = PM.Library.lookCatalog(state.scope, true).filter(look => state.trash ? !!look.deletedAt : !look.deletedAt);
  return h('div.library-view',
    state.trash ? null : saveSectionBar(),
    sections.length ? h('section.library-group', h('h2', 'Sections'), h('div.library-grid', ...sections.map(sectionCard))) : looks.length ? null : empty(
      state.trash ? 'Trash is empty' : `No sections in ${scopeName().toLowerCase()}`,
      state.trash ? 'Deleted items remain recoverable here.' : 'Select layers and save them as a reusable editable section.'),
    looks.length ? h('section.library-group', h('h2', 'Looks'), h('div.library-grid', ...looks.map(lookCard))) : null);
}

function workspaceMap(workspace) {
  const docks = (workspace.layout?.docks || []).map(dock => {
    const panels = (dock.panels || []).map(panel =>
      h('i', { title: PM.PANELS[panel.id]?.title || panel.id }));
    return h('div.workspace-map-dock' + (dock.flex ? '.flex' : ''), ...panels);
  });
  return h('div.workspace-map', ...docks);
}
function workspaceList() {
  if (state.trash) return PM.WS.trashList();
  return PM.WS.list().filter(workspace => state.scope === 'global'
    ? workspace.builtin || workspace.scope !== 'project'
    : workspace.id === PM.WS.current.id || (workspace.scope === 'project' && workspace.projectId === PM.proj.id));
}
function workspaceMenu(anchor, workspace) {
  if (workspace.deletedAt) {
    PM.menu(anchor, [{ label: 'Restore', run: () => { PM.WS.restore(workspace.id); paint(); } }]); return;
  }
  const items = [
    { label: 'Apply with preview…', run: () => previewWorkspace(workspace) },
    { label: 'Edit layout', run: () => beginWorkspaceEdit(workspace) },
    { label: 'Duplicate', run: () => { PM.WS.duplicate(workspace.id, { scope: state.scope, projectId: state.scope === 'project' ? PM.proj.id : null }); paint(); } },
  ];
  if (workspace.builtin) items.push('-', { label: 'Reset built-in', run: () => { PM.WS.resetBuiltin(workspace.id); paint(); } });
  else items.push({ label: 'Rename…', run: () => renameWorkspace(workspace) }, '-', { label: 'Move to Trash', run: () => { PM.WS.remove(workspace.id); paint(); } });
  items.push('-', { label: 'Edit validated definition…', run: () => { PM.WS.activate(workspace.id, true); PM.WS.editJSON(); } });
  items[0].disabled = workspace.id === PM.WS.current.id;
  PM.menu(anchor, items);
}
function workspaceCard(workspace) {
  const more = h('button.library-more', { title: 'Workspace actions' }, PM.icon('more'));
  more.onclick = e => { e.stopPropagation(); workspaceMenu(more, workspace); };
  return h('article.library-card.workspace' + (workspace.id === PM.WS.current.id ? '.active' : '') + (workspace.deletedAt ? '.deleted' : ''),
    workspaceMap(workspace),
    h('div.library-card-copy', h('b', workspace.name), h('span', workspace.builtin ? 'Built-in workspace' : 'Custom workspace'),
      h('small', `${workspace.layout?.docks?.length || 0} docks · ${(workspace.layout?.docks || []).flatMap(d => d.panels || []).length} panels`)),
    more,
    workspace.deletedAt ? button('Restore', () => { PM.WS.restore(workspace.id); paint(); })
      : h('div.library-card-actions',
          button(workspace.id === PM.WS.current.id ? 'Active' : 'Apply', () => previewWorkspace(workspace), workspace.id !== PM.WS.current.id),
          button('Edit layout', () => beginWorkspaceEdit(workspace))));
}
function workspacesView() {
  const workspaces = workspaceList();
  return h('div.library-view',
    h('div.library-create', h('div', h('b', state.scope === 'project' ? 'Project workspaces' : 'Workspace library'),
      h('span', 'Structured layouts stay editable, validated, and recoverable.')),
      state.trash ? null : button('Save current as new', () => { PM.WS.duplicate(PM.WS.current.id, { scope: state.scope, projectId: state.scope === 'project' ? PM.proj.id : null }); paint(); }, true)),
    workspaces.length ? h('div.library-grid', ...workspaces.map(workspaceCard)) : empty(state.trash ? 'Trash is empty' : 'No workspaces here', 'Save the current layout as a new workspace.'));
}
function previewWorkspace(workspace) {
  PM.modal({ title: 'Apply workspace?', width: 520,
    body: h('div.workspace-preview', workspaceMap(workspace), h('b', workspace.name), h('p', 'This changes panel arrangement only. Your project, layers, and timeline remain untouched.')),
    actions: [{ label: 'Cancel' }, { label: 'Apply', pri: true, run: () => { PM.WS.activate(workspace.id); close(); } }],
  });
}
function renameWorkspace(workspace) {
  const input = h('input', { value: workspace.name });
  PM.modal({ title: 'Rename workspace', body: h('div.field', input), actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => { PM.WS.rename(workspace.id, input.value); paint(); } },
  ] }); setTimeout(() => { input.focus(); input.select(); }, 30);
}
function beginWorkspaceEdit(workspace) {
  if (!PM.WS.beginEdit(workspace.id)) return;
  close(); WorkspaceEditor.show();
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

function empty(title, copy) { return h('div.library-empty', PM.icon(state.trash ? 'trash' : state.tab === 'sections' ? 'layers' : 'panel'), h('b', title), h('span', copy)); }
function paint() {
  if (!state.root) return;
  state.root.textContent = '';
  state.root.append(header(), h('main', state.tab === 'sections' ? sectionsView() : workspacesView()));
}

PM.LibraryUI = { open, close, get isOpen() { return !!state.root?.classList.contains('on'); } };
PM.WorkspaceEditor = WorkspaceEditor;
PM.bus.on('library', () => { if (state.root?.classList.contains('on')) paint(); });
PM.bus.on('workspaces', () => { if (state.root?.classList.contains('on')) paint(); });
})();
