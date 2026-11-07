// @vitest-environment happy-dom
import { workspaceStatus } from '../../shell/workspace-actions';
import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
function editor(saved = new Map<string, any>()) {
  const PM = makePM('core/workspace');
  PM.store = { separateHistory: true, get: (k: string, f: any) => saved.has(k) ? structuredClone(saved.get(k)) : f,
    set: (k: string, v: any) => saved.set(k, structuredClone(v)) };
  PM.Layout = { apply() {} }; PM.perf = {}; PM.proj = {id: 'project'}; PM.WS.init(); return PM;
}
describe('workspace preferences', () => {
  it('offers Reset from the first edit of a built-in, and reads Unchanged after it', () => {
    const PM = editor();
    PM.WS.reset('design');
    PM.WS.mutate((w: any) => { w.layout.docks[0].size = 377; });
    expect(workspaceStatus(PM)).toMatchObject({ builtin: false, changed: true, canReset: true });
    PM.WS.reset();
    expect(workspaceStatus(PM)).toMatchObject({ changed: false, canReset: false });
  });

  it('resets the first built-in edit to its pre-edit baseline across restart', () => {
    const saved = new Map<string, any>(), PM = editor(saved);
    PM.WS.reset('design');
    const original = structuredClone(PM.WS.current.layout);
    PM.WS.mutate((w: any) => { w.layout.docks[0].size = 377; });
    const id = PM.WS.current.id;
    expect(PM.WS.current.builtin).toBe(false);
    expect(PM.WS.current.savedState.layout).toEqual(original);
    const restarted = editor(saved);
    expect(restarted.WS.reset(id)).toBe(true);
    expect(restarted.WS.current.layout).toEqual(original);
    restarted.WS.mutate((w: any) => { w.layout.docks[0].size = 388; });
    expect(restarted.WS.saveLayout()).toBe(true);
    restarted.WS.mutate((w: any) => { w.layout.docks[0].size = 399; });
    restarted.WS.reset();
    expect(restarted.WS.current.layout.docks[0].size).toBe(388);
  });

  it('uses the last arrangement for a new project after restart', () => {
    const saved = new Map<string, any>(), PM = editor(saved);
    PM.WS.mutate((w: any) => { w.layout.docks[0].size = 377; });
    const expected = PM.WS.snapshot(), restarted = editor(saved);
    restarted.WS.openForProject(); expect(restarted.WS.snapshot()).toEqual(expected);
  });
  it('saves a global workspace and resets changes to the saved arrangement after restart', () => {
    const saved = new Map<string, any>(), PM = editor(saved), w = PM.WS.saveCurrent('My layout');
    expect(w).toMatchObject({name: 'My layout', scope: 'global', projectId: null, builtin: false});
    const original = structuredClone(w.layout);
    PM.WS.mutate((draft: any) => { draft.layout.docks[0].size = 377; });
    expect(PM.WS.current.layout).not.toEqual(original);
    const restarted = editor(saved);
    expect(restarted.WS.reset(w.id)).toBe(true); expect(restarted.WS.current.layout).toEqual(original);
    expect(restarted.WS.rename(w.id, 'Editing')).toBe(true); restarted.WS.reset(w.id);
    expect(restarted.WS.current.name).toBe('Editing');
  });
  it('opens new projects in the default and keeps existing project arrangements', () => {
    const saved = new Map<string, any>(), PM = editor(saved), w = PM.WS.saveCurrent('My default');
    expect(PM.WS.setDefault(w.id)).toBe(true); PM.WS.activate('animate', true);
    const previous = PM.WS.snapshot(); PM.WS.openForProject(); expect(PM.WS.current.id).toBe(w.id);
    PM.WS.openForProject(previous); expect(PM.WS.current.id).toBe('animate');
    expect(editor(saved).WS.defaultId()).toBe(w.id);
  });
  it('clears a deleted default and preserves its baseline in trash', () => {
    const PM = editor(), w = PM.WS.saveCurrent('Saved'); PM.WS.setDefault(w.id); PM.WS.remove(w.id);
    expect(PM.WS.defaultId()).toBe(null); expect(PM.WS.current.id).toBe('design');
    expect(PM.WS.restore(w.id).savedState).toBeDefined();
    expect(PM.WS.setDefault('missing')).toBe(false); expect(PM.WS.reset('missing')).toBe(false);
    expect(PM.WS.rename('design', 'No')).toBe(false);
  });
  it('does not persist uncommitted edits as last used', () => {
    const saved = new Map<string, any>(), PM = editor(saved), before = PM.WS.snapshot();
    PM.WS.beginEdit(); PM.WS.mutate((w: any) => { w.layout.docks[0].size = 377; });
    expect(editor(saved).WS.snapshot()).toEqual(before); PM.WS.cancelEdit();
  });
  it('updates cross-window library records without detaching the mounted active layout', () => {
    const saved = new Map<string, any>(), PM = editor(saved), w = PM.WS.saveCurrent('Saved');
    const incoming = structuredClone(saved.get('workspaces'));
    incoming.find((item: any) => item.id === w.id).name = 'Renamed elsewhere';
    incoming.push({...structuredClone(w), id:'other-window', name:'Another workspace'});
    saved.set('workspaces', incoming); PM.bus.emit('store:external', ['workspaces']);
    expect(PM.WS.current).toBe(PM.WS.get(w.id));
    PM.WS.current.layout.docks[0].size = 388; PM.WS.save();
    const restarted = editor(saved);
    expect(restarted.WS.current.layout.docks[0].size).toBe(388);
    expect(restarted.WS.current.name).toBe('Renamed elsewhere');
    expect(restarted.WS.get('other-window')).toBeDefined();
  });
  it('does not resurrect a deleted workspace from an older project snapshot', () => {
    const PM = editor(), w = PM.WS.saveCurrent('Saved'), snapshot = PM.WS.snapshot();
    PM.WS.remove(w.id); PM.WS.restoreSnapshot(snapshot);
    expect(PM.WS.get(w.id)).toBeUndefined(); expect(PM.WS.current.id).toBe('design');
  });
  it('persists built-in reset to the project snapshot and validates names', () => {
    const saved = new Map<string, any>(), PM = editor(saved);
    PM.WS.current.layout.docks[0].size = 444; PM.WS.save();
    expect(PM.WS.reset('design')).toBe(true);
    expect(saved.get('projectWorkspace.project').layout).toEqual(PM.WS.current.layout);
    const w = PM.WS.saveCurrent('User');
    expect(PM.WS.rename(w.id,'')).toBe(false); expect(PM.WS.rename(w.id,'x'.repeat(81))).toBe(false);
  });
  it('persists the activated workspace when restoring history or saving an edit as new', () => {
    const saved = new Map<string, any>(), PM = editor(saved), before = PM.WS.historySnapshot();
    PM.WS.saveCurrent('Another'); PM.WS.restoreHistorySnapshot(before);
    expect(saved.get('projectWorkspace.project').id).toBe('design');
    PM.WS.beginEdit(); PM.WS.mutate((w: any) => { w.layout.docks[0].size = 388; });
    const created = PM.WS.saveEdit(true,'Edited workspace');
    expect(saved.get('projectWorkspace.project').id).toBe(created.id);
    expect(saved.get('projectWorkspace.project').layout.docks[0].size).toBe(388);
  });
  it('saves an updated baseline explicitly and ignores stale project baselines', () => {
    const PM = editor(), w = PM.WS.saveCurrent('Saved'), stale = PM.WS.snapshot();
    PM.WS.mutate((draft: any) => { draft.layout.docks[0].size = 377; });
    PM.WS.saveLayout(); PM.WS.restoreSnapshot(stale); PM.WS.reset(w.id);
    expect(PM.WS.current.layout.docks[0].size).toBe(377);
  });
});
