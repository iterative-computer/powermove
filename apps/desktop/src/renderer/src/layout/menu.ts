import type { MenuContribution } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';
import {
  dockSlots,
  findSlot,
  moveTabBy,
  stackPanel,
  unstackPanel,
  type DockSpec,
  type PanelSpec,
  type Workspace
} from './model';
import { canPopoutPanel } from './popout';
import { canStack, stepTab } from './tabs';

/** Tab entries: cycle, reorder or separate a tab, or stack an untabbed panel
 * with the slot above or below it. (Ctrl+Tab belongs to project tabs; a
 * focused tab strip cycles with the arrow keys.) */
function tabItems(PM: PMRegistry, ws: Workspace, id: string): MenuContribution[] {
  const found = findSlot(ws, id);
  if (!found || !canStack(PM, id)) return [];
  const mutate = (fn: (workspace: Workspace) => unknown) => () => PM.WS.mutate((workspace: Workspace) => { fn(workspace); });
  const { slot } = found;
  if (slot.length > 1) {
    const index = slot.indexOf(found.spec);
    return [
      { label: 'Next tab', run: () => stepTab(PM, id, 1) },
      ...(index > 0 ? [{ label: 'Move tab left', run: mutate((workspace) => moveTabBy(workspace, id, -1)) }] : []),
      ...(index < slot.length - 1 ? [{ label: 'Move tab right', run: mutate((workspace) => moveTabBy(workspace, id, 1)) }] : []),
      { label: 'Separate tab', icon: 'panel', run: mutate((workspace) => unstackPanel(workspace, id)) }
    ];
  }
  const slots = dockSlots(found.dock.panels);
  const at = slots.findIndex((members) => members.includes(found.spec));
  const visible = (members: PanelSpec[] | undefined) => members?.find((member) => PM.PANELS?.[member.id] && PM.Popout?.isOpen?.(member.id) !== true);
  return [slots[at - 1], slots[at + 1]].flatMap((members) => {
    const target = visible(members);
    if (!target || !canStack(PM, target.id)) return [];
    return [{
      label: `Stack with ${PM.PANELS[target.id]?.title || target.id}`,
      icon: 'stack',
      run: mutate((workspace) => stackPanel(workspace, id, target.id, null, (panel) => canStack(PM, panel)))
    }];
  });
}

/** Opens the native panel menu at an explicit screen position. */
export function openPanelMenu(
  PM: PMRegistry,
  spec: PanelSpec,
  dock: DockSpec,
  trigger: HTMLElement,
  position: { x: number; y: number }
): void {
  const ws = PM.Layout.ws as Workspace | null;
  const def = PM.PANELS[spec.id];
  if (!ws || !def) return;

  const contributions = (PM.Kernel?.gatherMenu?.('panel:context', {
    panelId: spec.id,
    dockId: dock.id
  }) ?? []) as MenuContribution[] | Promise<MenuContribution[]>;
  const items = (contributed: MenuContribution[]): MenuContribution[] => [
    { header: def.title },
    ...(canPopoutPanel(spec.id) ? [{
      label: 'Pop out to window',
      icon: 'export',
      run: () => PM.Popout?.open?.(spec.id)
    }] : []),
    ...tabItems(PM, ws, spec.id),
    ...(spec.id === 'viewer' ? [] : [{
      label: (findSlot(ws, spec.id)?.slot.length ?? 0) > 1 ? 'Close tab' : 'Close panel',
      icon: 'x',
      run: () => PM.WS.mutate((workspace: Workspace) => PM.Layout.closePanel(workspace, spec.id))
    }]),
    ...(contributed.length ? ['-' as const, ...contributed] : [])
  ];

  /* Sandboxed contributions answer over a port; the kernel bounds the wait. */
  PM.menu(trigger, Array.isArray(contributions) ? items(contributions) : contributions.then(items), position);
}
