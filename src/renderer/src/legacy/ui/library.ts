/* The Library is the live panel catalog. Saved project content is left intact. */
import type { PMRegistry } from '../registry';
import { panelIcons } from '../../panels/panel-icons';

export function install(PM: PMRegistry): void {
const h: any = PM.h;
const state: any = { root: null, overlay: null, content: null, search: null, lastFocus: null, query: '' };
const button: any = (label: string, run: () => void, primary = false) => h('button.btn' + (primary ? '.pri' : ''), { onclick: run }, label);
function reveal(id: string) {
  if (!PM.PANELS[id]) return;
  close();
  if (id === 'toolbar') { document.getElementById('toolbar-strip')?.scrollIntoView({ block: 'nearest' }); return; }
  PM.WS.mutate((workspace: any) => {
    if (workspace.hiddenPanels?.some((panel: any) => panel.id === id)) PM.Layout.restorePanel(workspace, id);
    else if (!PM.Layout.hasPanel(workspace, id)) PM.Layout.addPanel(workspace, id, id === 'viewer' ? 'center' : 'right');
    const found = PM.Layout.findPanel(workspace, id);
    if (found) { found.dock.hidden = false; found.spec.collapsed = false; }
  });
  PM.panelInst[id]?.el?.scrollIntoView({ block: 'nearest' });
}
function create() {
  close();
  PM.SpatialAssistant?.open?.();
  PM.AgentUI?.setScope('workspace');
  PM.AgentUI?.setDraft('Create a new panel that ', true);
}
function paint() {
  if (!state.content) return;
  state.content.replaceChildren();
  const panels = Object.entries(PM.PANELS || {}).map(([id, def]: any) => ({ ...def, id }))
    .sort((a: any, b: any) => String(a.title || a.id).localeCompare(String(b.title || b.id)));
  const matches = panels.filter((panel: any) => !state.query || (panel.title + ' ' + panel.id).toLowerCase().includes(state.query));
  const icons = panelIcons(panels, PM.ICONS || {});
  for (const panel of matches) {
    const location = PM.Layout?.findPanel?.(PM.WS?.current, panel.id);
    const visible = panel.id === 'toolbar' || (location && !location.dock.hidden && !location.spec.collapsed);
    const title = panel.title || panel.id;
    const open = h('button.panel-library-open', { type: 'button', onclick: () => reveal(panel.id), 'aria-label': 'Open ' + title }, PM.icon(icons[panel.id]), h('span', title));
    state.content.append(h('div.panel-library-row', { data: { panelId: panel.id } }, open, h('span.panel-library-status', visible ? 'Open' : '')));
  }
  if (!matches.length) state.content.append(h('div.panel-library-empty', state.query ? 'No panels match your search.' : 'No panels are available.'));
}
function ensure() {
  if (state.root) return;
  state.search = h('input', { type: 'search', placeholder: 'Search panels', 'aria-label': 'Search panels', oninput: () => {
    state.query = state.search.value.trim().toLowerCase(); paint();
  } });
  state.content = h('div.panel-library-list', { 'aria-label': 'Available panels' });
  state.root = h('section#library-screen.panel-library', { role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Panel library', tabindex: '-1' },
    h('header.panel-library-head', h('b', 'Panels'), h('span.sp'),
      h('button.btn', { type: 'button', onclick: create }, PM.icon('plus'), 'New panel'),
      h('button.iconbtn', { type: 'button', 'aria-label': 'Close Library', onclick: close }, PM.icon('x'))),
    h('label.panel-library-search', PM.icon('search'), state.search), state.content);
  state.overlay = h('div#library-overlay', state.root);
  state.overlay.addEventListener('pointerdown', (event: any) => { if (event.target === state.overlay) close(); });
  state.overlay.addEventListener('keydown', (event: KeyboardEvent) => {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); close(); }
    if (event.key !== 'Tab') return;
    const items = [...state.root.querySelectorAll('button,input')] as HTMLElement[];
    const first = items[0], last = items.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  });
  document.body.appendChild(state.overlay);
}
function open() {
  ensure(); state.lastFocus = document.activeElement; paint();
  const app = document.getElementById('app'); if (app) app.inert = true;
  state.overlay.classList.add('on'); state.root.classList.add('on');
  window.requestAnimationFrame(() => state.search.focus());
}
function close() {
  if (!state.overlay?.classList.contains('on')) return;
  state.overlay.classList.remove('on'); state.root.classList.remove('on');
  const app = document.getElementById('app'); if (app) app.inert = false;
  const focus = state.lastFocus; state.lastFocus = null;
  if (focus?.isConnected) focus.focus();
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


PM.LibraryUI = { open, close, reveal, create, get isOpen() { return !!state.root?.classList.contains('on'); } };
PM.WorkspaceEditor = WorkspaceEditor;
PM.bus.on('layout:applied', () => { if (PM.LibraryUI.isOpen) paint(); });
PM.bus.on('workspaces', () => { if (PM.LibraryUI.isOpen) paint(); });
PM.bus.on('project', close);
PM.Kernel?.events?.on?.('extensions:changed', () => { if (PM.LibraryUI.isOpen) paint(); });
}
