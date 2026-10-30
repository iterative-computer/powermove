import type { MenuContribution } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';
import {
  placePanels,
  stackOf,
  type DockSpec,
  type PanelSpec,
  type Workspace
} from './model';
import { canPopoutPanel } from './popout';

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
  const maximized = PM.Layout.maximizedPanel?.();
  const stack = stackOf(ws, spec.id);
  const others = stack.filter((member) => member.id !== spec.id);
  /* Any panel can join this one as a tab: a closed one opens here, an open
     one moves here. Panels without a header cannot show tabs. */
  const tabbable = (item: Record<string, any> | undefined): boolean => !!item && !item.headless && item.id !== 'viewer';
  const addable = !tabbable(def) ? [] : (Object.values(PM.PANELS) as Array<Record<string, any>>)
    .filter((item) => tabbable(item) && item.id !== 'toolbar' && item.library !== false
      && !stack.some((member) => member.id === item.id) && PM.Popout?.isOpen?.(item.id) !== true)
    .sort((a, b) => String(a.title).localeCompare(String(b.title)));
  const items = (contributed: MenuContribution[]): MenuContribution[] => [
    { header: def.title },
    /* After Effects' panel menu: maximize, undock, close, close the rest of the stack. */
    {
      label: maximized && stack.some((member) => member.id === maximized) ? 'Restore panel size' : 'Maximize panel',
      icon: 'frame',
      kb: '`',
      run: () => PM.Layout.toggleMaximize?.(spec.id)
    },
    ...(addable.length ? [{
      label: 'Add panel as tab',
      icon: 'plus',
      submenu: addable.map((item) => ({
        label: String(item.title || item.id),
        run: () => {
          PM.Layout.rememberPanelOpen?.(item.id);
          PM.WS.mutate((workspace: Workspace) => { placePanels(workspace, [{ id: item.id }], { kind: 'group', target: spec.id }); });
        }
      }))
    }] : []),
    ...(canPopoutPanel(spec.id) ? [{
      label: 'Pop out to window',
      icon: 'export',
      run: () => PM.Popout?.open?.(spec.id)
    }] : []),
    ...(spec.id === 'viewer' ? [] : [{
      label: 'Close panel',
      icon: 'x',
      run: () => PM.WS.mutate((workspace: Workspace) => PM.Layout.closePanel(workspace, spec.id))
    }]),
    ...(others.length ? [{
      label: 'Close other panels in group',
      run: () => PM.WS.mutate((workspace: Workspace) => {
        for (const member of others) PM.Layout.closePanel(workspace, member.id);
      })
    }] : []),
    ...(contributed.length ? ['-' as const, ...contributed] : [])
  ];

  /* Sandboxed contributions answer over a port; the kernel bounds the wait. */
  PM.menu(trigger, Array.isArray(contributions) ? items(contributions) : contributions.then(items), position);
}
