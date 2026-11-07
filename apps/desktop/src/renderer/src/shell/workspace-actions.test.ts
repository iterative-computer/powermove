// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import {
  deleteWorkspace,
  resetWorkspace,
  saveWorkspaceChanges,
  toggleDefaultWorkspace,
  workspaceMenuItems,
  workspaceStatus
} from './workspace-actions';

function fakePM() {
  const layout = { docks: [{ id: 'left', size: 300 }] };
  const custom: any = { id: 'ws-1', name: 'Editing', layout, hiddenPanels: [], savedState: { layout: { docks: [{ id: 'left', size: 300 }] }, hiddenPanels: [] } };
  const builtin: any = { id: 'design', name: 'Default', builtin: true, layout, hiddenPanels: [] };
  let defaultId: string | null = null;
  const all = [builtin, custom];
  const PM: Record<string, any> = {
    toast: vi.fn(),
    confirm: vi.fn(async () => true),
    bus: { emit: vi.fn() },
    WS: {
      all, current: custom, editing: null,
      list: () => all,
      get: (id: string) => all.find((w) => w.id === id) ?? null,
      defaultId: () => defaultId,
      setDefault: vi.fn((id: string | null) => { defaultId = id; return true; }),
      saveLayout: vi.fn((id: string) => { const w = all.find((x) => x.id === id); w.savedState = { layout: structuredClone(w.layout), hiddenPanels: [] }; return true; }),
      reset: vi.fn(() => true),
      activate: vi.fn(),
      remove: vi.fn((id: string) => { all.splice(all.findIndex((w) => w.id === id), 1); }),
      restore: vi.fn((id: string) => { const w = { ...custom, id }; all.push(w); return w; }),
    }
  };
  return { PM, custom, builtin };
}

const row = (items: any[], label: string) => items.find((item) => typeof item === 'object' && item.label === label);

describe('workspace actions', () => {
  it('reads unchanged, changed and built-in states without key-order noise', () => {
    const { PM, custom } = fakePM();
    // Same content, different key order: still unchanged.
    custom.savedState = { hiddenPanels: [], layout: { docks: [{ size: 300, id: 'left' }] } };
    expect(workspaceStatus(PM)).toMatchObject({ changed: false, canSave: false, canReset: false, isDefault: false });
    custom.layout.docks[0].size = 377;
    expect(workspaceStatus(PM)).toMatchObject({ changed: true, canSave: true, canReset: true });
    expect(workspaceStatus(PM, 'design')).toMatchObject({ builtin: true, canSave: false, canReset: true });
  });

  it('lists workspaces with the default marked, then the After Effects workspace commands', () => {
    const { PM } = fakePM();
    PM.WS.setDefault('design');
    const items = workspaceMenuItems(PM);
    expect(items.filter((item: any) => item.label).map((item: any) => item.label)).toEqual([
      'Default', 'Editing',
      'Save Changes to This Workspace', 'Reset to Saved Layout', 'Set as Default Workspace',
      'New Workspace…', 'Rename Workspace…', 'Delete Workspace…', 'Panel Library…'
    ]);
    expect(row(items, 'Default')).toMatchObject({ kb: 'Default', on: false });
    expect(row(items, 'Editing')).toMatchObject({ on: true, kb: null });
    // Nothing changed since the last save: both disabled, each saying why.
    expect(row(items, 'Save Changes to This Workspace')).toMatchObject({ disabled: true, kb: 'Saved' });
    expect(row(items, 'Reset to Saved Layout')).toMatchObject({ disabled: true, kb: 'Unchanged' });
    expect(row(items, 'Set as Default Workspace')).toMatchObject({ on: false, disabled: false });
    expect(row(items, 'Rename Workspace…').disabled).toBe(false);
  });

  it('keeps built-ins from being saved over, renamed or deleted', () => {
    const { PM, builtin } = fakePM();
    PM.WS.current = builtin;
    const items = workspaceMenuItems(PM);
    expect(row(items, 'Save Changes to This Workspace')).toMatchObject({ disabled: true, kb: 'Built-in' });
    expect(row(items, 'Reset to Saved Layout').disabled).toBe(false);
    expect(row(items, 'Rename Workspace…').disabled).toBe(true);
    expect(row(items, 'Delete Workspace…').disabled).toBe(true);
    expect(saveWorkspaceChanges(PM)).toBe(false);
    expect(PM.WS.saveLayout).not.toHaveBeenCalled();
  });

  it('saves, resets and toggles the default with a confirming toast', () => {
    const { PM, custom } = fakePM();
    custom.layout.docks[0].size = 377;
    expect(saveWorkspaceChanges(PM)).toBe(true);
    expect(PM.WS.saveLayout).toHaveBeenCalledWith('ws-1');
    expect(PM.toast.mock.calls.at(-1)[0]).toBe('Saved “Editing”. Reset now returns to this layout.');
    expect(workspaceStatus(PM).changed).toBe(false);

    expect(resetWorkspace(PM)).toBe(true);
    expect(PM.toast.mock.calls.at(-1)[0]).toBe('Reset “Editing” to its saved layout');
    PM.WS.reset.mockReturnValueOnce(false);
    expect(resetWorkspace(PM)).toBe(false);

    expect(toggleDefaultWorkspace(PM)).toBe(true);
    expect(PM.WS.defaultId()).toBe('ws-1');
    expect(row(workspaceMenuItems(PM), 'Set as Default Workspace').on).toBe(true);
    expect(PM.toast.mock.calls.at(-1)[0]).toBe('New windows and projects open in “Editing”');
    expect(toggleDefaultWorkspace(PM)).toBe(true);
    expect(PM.WS.defaultId()).toBe(null);
  });

  it('confirms a delete and offers Undo that restores the default and current workspace', async () => {
    const { PM } = fakePM();
    PM.WS.setDefault('ws-1');
    expect(await deleteWorkspace(PM)).toBe(true);
    expect(PM.WS.remove).toHaveBeenCalledWith('ws-1');
    const [, , options] = PM.toast.mock.calls.at(-1);
    expect(options.action.label).toBe('Undo');
    options.action.run();
    expect(PM.WS.restore).toHaveBeenCalledWith('ws-1');
    expect(PM.WS.setDefault).toHaveBeenLastCalledWith('ws-1');
    expect(PM.WS.activate).toHaveBeenCalledWith('ws-1', true);

    PM.confirm.mockResolvedValueOnce(false);
    PM.WS.current = PM.WS.get('ws-1');
    PM.WS.remove.mockClear();
    expect(await deleteWorkspace(PM)).toBe(false);
    expect(PM.WS.remove).not.toHaveBeenCalled();
  });
});
