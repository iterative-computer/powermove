/* The Library is the live panel catalog and workspace gallery. Saved project
   content is left intact. The screen itself is Svelte (LibraryScreen); this
   installer keeps the legacy PM.LibraryUI contract and defers any DOM work
   until the library is first opened. */
import { mount } from 'svelte';

import type { PMRegistry } from '../registry';
import LibraryScreen from '../../library/LibraryScreen.svelte';

export function install(PM: PMRegistry): void {
const h: any = PM.h;
let screen: any = null;
const button: any = (label: string, run: () => void, primary = false) => h('button.btn' + (primary ? '.pri' : ''), { onclick: run }, label);
function ensure() {
  if (!screen) screen = mount(LibraryScreen, { target: document.body, props: { PM } });
  return screen;
}
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
function open(view?: 'panels' | 'workspaces') {
  ensure().open(view);
}
function close() {
  screen?.close();
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


PM.LibraryUI = { open, close, reveal, create, get isOpen() { return !!screen?.isOpen(); } };
PM.WorkspaceEditor = WorkspaceEditor;
PM.bus.on('layout:applied', () => { if (PM.LibraryUI.isOpen) screen?.refresh(); });
PM.bus.on('workspaces', () => { if (PM.LibraryUI.isOpen) screen?.refresh(); });
PM.bus.on('project', close);
PM.Kernel?.events?.on?.('extensions:changed', () => { if (PM.LibraryUI.isOpen) screen?.refresh(); });
}
