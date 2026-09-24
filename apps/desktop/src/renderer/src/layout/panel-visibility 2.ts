import type { PMRegistry } from '../legacy/registry';
import { hidePanel, type Workspace } from './model';

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
  if (visible) {
    if (!(id in saved)) return;
    delete saved[id];
  } else {
    if (saved[id] === false) return;
    saved[id] = false;
  }
  PM.store?.set?.(STORE_KEY, saved);
}

/** Apply only visibility. Each workspace keeps its own dock, order and size. */
export function applyPanelVisibility(PM: PMRegistry, workspace: Workspace): boolean {
  let changed = false;
  for (const [id, visible] of Object.entries(preferences(PM)))
    if (!visible) changed = hidePanel(workspace, id) || changed;
  return changed;
}
