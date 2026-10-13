import { describe, expect, it, vi } from 'vitest';
import { setPanelLayout } from './workspace-layout';
import type { PMRegistry } from '../registry';

function host() {
  let current = { id: 'original', name: 'Original' };
  const checkpoints: any[] = [];
  const settings: Record<string, unknown> = {};
  const PM = { store: { get: (key: string, fallback: unknown) => settings[key] ?? fallback, set: (key: string, value: unknown) => { settings[key] = value; } }, PANELS: { viewer: {}, timeline: {}, inspector: {}, agent: {}, toolbar: {}, assets: {}, 'layer-effects': {}, 'timing.panel': {} },
    WS: { historySnapshot: () => structuredClone(current), restoreHistorySnapshot: vi.fn(value => { current = value; }),
      create: vi.fn(value => { current = { ...value, id: 'imported' }; return current; }) },
    hist: { external: vi.fn((label, undo, redo) => { checkpoints.push({ label, undo, redo }); return 'layout-history'; }) } };
  return { PM: PM as unknown as PMRegistry, checkpoints, getCurrent: () => current };
}
const arrangement = { name: 'From After Effects', docks: [
  { id: 'center', panels: [{ id: 'viewer', flex: true }, { id: 'timeline', size: 220 }] },
  { id: 'right', size: 320, panels: [{ id: 'timing.panel', size: 200 }, { id: 'inspector', size: 250 }] }
] };

describe('agent workspace arrangement', () => {
  it('retains import provenance through Undo and Redo and clears it on unrelated layouts', () => {
    const { PM, checkpoints } = host();
    setPanelLayout(PM, { ...arrangement, sourceApp: 'after-effects' });
    expect(PM.WS.create).toHaveBeenLastCalledWith(expect.objectContaining({ sourceApp: 'after-effects' }));
    checkpoints[0].undo();
    expect(PM.WS.restoreHistorySnapshot).toHaveBeenLastCalledWith(expect.not.objectContaining({ sourceApp: 'after-effects' }));
    checkpoints[0].redo();
    expect(PM.WS.restoreHistorySnapshot).toHaveBeenLastCalledWith(expect.objectContaining({ sourceApp: 'after-effects' }));
    setPanelLayout(PM, arrangement);
    expect(PM.WS.create).toHaveBeenLastCalledWith(expect.objectContaining({ sourceApp: null }));
    expect(() => setPanelLayout(PM, { ...arrangement, sourceApp: 'unknown' })).toThrow('supported workspace source');
  });
  it('preserves panel order and sizes in a new global workspace with working Undo/Redo', () => {
    const { PM, checkpoints, getCurrent } = host();
    expect(setPanelLayout(PM, arrangement)).toMatchObject({ workspaceId: 'imported', historyId: 'layout-history' });
    expect(PM.WS.create).toHaveBeenCalledWith(expect.objectContaining({ scope: 'global', projectId: null, layout: { docks: [
      { id: 'center', flex: true, panels: arrangement.docks[0]!.panels },
      { id: 'right', size: 320, panels: arrangement.docks[1]!.panels }
    ] } }));
    expect(PM.store.get('defaultWorkspace')).toBe('imported');
    checkpoints[0].undo(); expect(getCurrent().id).toBe('original'); expect(PM.store.get('defaultWorkspace')).toBe('design');
    checkpoints[0].redo(); expect(getCurrent().id).toBe('imported');
  });
  it('places Layer Effects below Media unless the arrangement already includes it', () => {
    const { PM } = host();
    setPanelLayout(PM, { name: 'With media', docks: [{ id: 'left', panels: [{ id: 'assets', flex: true }] }, ...arrangement.docks] });
    expect(PM.WS.create).toHaveBeenLastCalledWith(expect.objectContaining({ layout: { docks: expect.arrayContaining([
      { id: 'left', panels: [{ id: 'assets', flex: true }, { id: 'layer-effects', size: 350 }] }
    ]) } }));
    setPanelLayout(PM, { name: 'Own place', docks: [{ id: 'left', panels: [{ id: 'assets', flex: true }] }, { ...arrangement.docks[1]!, panels: [{ id: 'layer-effects' }] }, arrangement.docks[0]!] });
    const docks = (PM.WS.create as any).mock.lastCall[0].layout.docks;
    expect(docks.find((dock: any) => dock.id === 'left').panels).toEqual([{ id: 'assets', flex: true }]);
  });
  it('accepts a bottom dock for a full-width timeline and orders it before the right column', () => {
    const { PM } = host();
    setPanelLayout(PM, { name: 'Wide timeline', docks: [
      { id: 'right', size: 320, panels: [{ id: 'inspector', flex: true }] },
      { id: 'bottom', size: 340, panels: [{ id: 'timeline', flex: true }] },
      { id: 'center', panels: [{ id: 'viewer', flex: true }] },
      { id: 'left', panels: [{ id: 'assets', flex: true }, { id: 'layer-effects', size: 350 }] }
    ] });
    const docks = (PM.WS.create as any).mock.lastCall[0].layout.docks;
    expect(docks.map((dock: any) => dock.id)).toEqual(['left', 'center', 'bottom', 'right']);
    expect(docks[2]).toEqual({ id: 'bottom', size: 340, panels: [{ id: 'timeline', flex: true }] });
  });
  it.each([
    { name: '', docks: arrangement.docks },
    { ...arrangement, docks: [...arrangement.docks, { id: 'top', panels: [] }] },
    { ...arrangement, docks: [{ id: 'center', panels: [{ id: 'missing' }] }] },
    { ...arrangement, docks: [{ id: 'center', panels: [{ id: 'viewer' }, { id: 'viewer' }, { id: 'timeline' }] }] },
    { ...arrangement, docks: [{ id: 'center', panels: [{ id: 'timeline' }] }] },
    { ...arrangement, docks: [{ id: 'center', panels: [{ id: 'viewer' }, { id: 'timeline', size: -20 }] }] },
    { ...arrangement, docks: [{ id: 'center', panels: [{ id: 'viewer' }, { id: 'timeline' }, { id: 'toolbar' }] }] }
  ])('rejects an invalid complete layout before any mutation', args => {
    const { PM, getCurrent } = host();
    expect(() => setPanelLayout(PM, args)).toThrow(); expect(PM.WS.create).not.toHaveBeenCalled(); expect(getCurrent().id).toBe('original');
  });
});
