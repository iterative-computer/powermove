import type { PMRegistry } from '../legacy/registry';
import { addPanel, findPanel, hidePanel, restorePanel, type Workspace } from './model';

const STORE_KEY = 'panelVisibility';
const PANEL_ID = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,119}$/;

type VisibilityPreferences = Record<string, boolean>;

function preferences(PM: PMRegistry): VisibilityPreferences {
  const saved = PM.store?.get?.(STORE_KEY, {});
  if (!saved || typeof saved !== 'object' || Array.isArray(saved)) return {};
  return Object.fromEntries(Object.entries(saved)
    .filter(([id, visible]) => PANEL_ID.test(id) && typeof visible === 'boolean')
    .slice(-256)) as VisibilityPreferences;
}

export function preferredPanelVisibility(PM: PMRegistry, id: string): boolean | undefined {
  return preferences(PM)[id];
}

export function rememberPanelVisibility(PM: PMRegistry, id: string, visible: boolean): void {
  if (id === 'viewer' || !PANEL_ID.test(id)) return;
  const saved = preferences(PM);
  if (id === 'agent') {
    if (saved[id] === visible) return;
    saved[id] = visible;
  } else if (visible) {
    if (!(id in saved)) return;
    delete saved[id];
  } else {
    if (saved[id] === false) return;
    saved[id] = false;
  }
  PM.store?.set?.(STORE_KEY, saved);
}

/** Each workspace keeps its dock, order and size. The pinned agent follows
 * the user across projects until explicitly closed. */
export function applyPanelVisibility(PM: PMRegistry, workspace: Workspace): boolean {
  let changed = false;
  const saved = preferences(PM);
  // Adopt an agent already pinned in an older saved workspace.
  const agent = findPanel(workspace, 'agent');
  if (saved.agent === undefined && agent && !agent.dock.hidden) {
    rememberPanelVisibility(PM, 'agent', true);
    saved.agent = true;
  }
  for (const [id, visible] of Object.entries(saved)) {
    if (!visible) changed = hidePanel(workspace, id) || changed;
    else if (id === 'agent') {
      const existing = findPanel(workspace, id);
      if (existing) {
        if (existing.dock.hidden) { existing.dock.hidden = false; changed = true; }
      } else {
        if (!restorePanel(workspace, id)) addPanel(workspace, id, 'right');
        const restored = findPanel(workspace, id);
        if (restored) restored.dock.hidden = false;
        changed = true;
      }
    }
  }
  return changed;
}
