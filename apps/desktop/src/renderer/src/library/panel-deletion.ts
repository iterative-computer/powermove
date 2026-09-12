const DELETED_PANELS_KEY = 'deletedPanels';

type PMForPanelDeletion = Record<string, any>;

export function deletedPanelIds(PM: PMForPanelDeletion): Set<string> {
  const saved = PM.store?.get?.(DELETED_PANELS_KEY, []);
  return new Set(Array.isArray(saved) ? saved.filter((id): id is string => typeof id === 'string') : []);
}

/** Remove a panel from the catalog and every saved workspace. */
export function deletePanel(PM: PMForPanelDeletion, id: string): boolean {
  if (!id || !PM.PANELS?.[id] || id === 'viewer' || id === 'toolbar') return false;

  const deleted = deletedPanelIds(PM);
  deleted.add(id);
  PM.store?.set?.(DELETED_PANELS_KEY, [...deleted]);

  const workspaces = new Set<any>([...(PM.WS?.all ?? []), PM.WS?.current].filter(Boolean));
  for (const workspace of workspaces) {
    PM.Layout?.removePanel?.(workspace, id);
    workspace.hiddenPanels = (workspace.hiddenPanels ?? []).filter((panel: any) => panel.id !== id);
    workspace.custom = (workspace.custom ?? []).filter((panel: any) => panel.id !== id);
  }
  PM.WS?.save?.();

  delete PM.PANELS[id];
  PM.Layout?.apply?.(PM.WS?.current);
  return true;
}
