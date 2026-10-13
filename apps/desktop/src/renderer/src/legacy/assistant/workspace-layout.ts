import type { PMRegistry } from '../registry';
import type { DockSpec } from '../../layout/model';

/* `bottom` spans under every column but `right`; its size is a height. */
const DOCK_IDS = ['left', 'center', 'bottom', 'right'];

/** Validate the whole arrangement before touching the saved workspace. */
export function setPanelLayout(PM: PMRegistry, args: Record<string, unknown>) {
  if (PM.WS?.editing) throw new Error('Finish editing your workspace before arranging panels.');
  if (typeof args.name !== 'string' || !args.name.trim() || args.name.length > 80) throw new Error('Name the new workspace (up to 80 characters).');
  if (args.sourceApp !== undefined && args.sourceApp !== 'after-effects') throw new Error('Choose a supported workspace source.');
  if (!Array.isArray(args.docks) || args.docks.length < 1 || args.docks.length > DOCK_IDS.length) throw new Error('Supply one to four docks.');
  const dockIds = new Set<string>();
  const panelIds = new Set<string>();
  const docks: DockSpec[] = args.docks.map(raw => {
    if (!raw || !DOCK_IDS.includes(raw.id) || dockIds.has(raw.id) || !Array.isArray(raw.panels)) throw new Error('Use each left, center, bottom or right dock once.');
    dockIds.add(raw.id);
    if (raw.panels.length > 32) throw new Error('A dock may contain at most 32 panels.');
    const dock: DockSpec = { id: raw.id, panels: [], ...(raw.id === 'center' ? { flex: true } : {}) };
    if (raw.size !== undefined) {
      if (!Number.isFinite(raw.size) || raw.size < 160 || raw.size > 720) throw new Error('Dock sizes must be between 160 and 720.');
      if (raw.id !== 'center') dock.size = raw.size;
    }
    dock.panels = raw.panels.map((panel: any) => {
      if (!panel || typeof panel.id !== 'string' || !Object.hasOwn(PM.PANELS ?? {}, panel.id) || panelIds.has(panel.id)) throw new Error('Use registered panel IDs once. Load new extensions before arranging their panels.');
      if (panel.id === 'toolbar') throw new Error('The toolbar lives in the titlebar; leave it out of the docks.');
      panelIds.add(panel.id);
      if (panel.size !== undefined && (!Number.isFinite(panel.size) || panel.size < 72 || panel.size > 1200)) throw new Error('Panel heights must be between 72 and 1200.');
      if (panel.flex !== undefined && typeof panel.flex !== 'boolean') throw new Error('Panel flex must be true or false.');
      return { id: panel.id, ...(panel.size !== undefined ? { size: panel.size } : {}), ...(panel.flex === true ? { flex: true } : {}) };
    });
    return dock;
  });
  if (!docks.find(dock => dock.id === 'center')?.panels.some(panel => panel.id === 'viewer') || !panelIds.has('timeline')) throw new Error('Keep viewer in the center dock and include timeline.');
  // Layer Effects belongs below Media by default, as in the built-in workspaces.
  const media = docks.find(dock => dock.panels.some(panel => panel.id === 'assets'));
  if (media && Object.hasOwn(PM.PANELS ?? {}, 'layer-effects') && !panelIds.has('layer-effects') && media.panels.length < 32) {
    media.panels.splice(media.panels.findIndex(panel => panel.id === 'assets') + 1, 0, { id: 'layer-effects', size: 350 });
  }
  docks.sort((a, b) => DOCK_IDS.indexOf(a.id) - DOCK_IDS.indexOf(b.id));
  const before = PM.WS.historySnapshot();
  const previousDefault = PM.store.get('defaultWorkspace', 'design');
  try {
    const created = PM.WS.create({ name: args.name.trim(), scope: 'global', projectId: null, hiddenPanels: [], sourceApp: args.sourceApp ?? null, layout: { docks } });
    PM.store.set('defaultWorkspace', created.id);
    const after = PM.WS.historySnapshot();
    const historyId = PM.hist.external(`Agent · ${created.name}`, () => { PM.store.set('defaultWorkspace', previousDefault); return PM.WS.restoreHistorySnapshot(before); }, () => { PM.store.set('defaultWorkspace', created.id); return PM.WS.restoreHistorySnapshot(after); });
    return { workspaceId: created.id, name: created.name, historyId, note: 'Saved as a new workspace. New projects use this workspace. Previous workspaces remain available; normal Undo restores the previous arrangement.' };
  } catch (error) { PM.store.set('defaultWorkspace', previousDefault); PM.WS.restoreHistorySnapshot(before); throw error; }
}
