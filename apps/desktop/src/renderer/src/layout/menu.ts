import type { MenuContribution } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';
import {
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

  const contributions = (PM.Kernel?.collectMenu?.('panel:context', {
    panelId: spec.id,
    dockId: dock.id
  }) ?? []) as MenuContribution[];
  const items: MenuContribution[] = [
    { header: def.title },
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
    ...(contributions.length ? ['-' as const, ...contributions] : [])
  ];

  PM.menu(trigger, items, position);
}
