import type { PMRegistry } from '../legacy/registry';
import { markMenuDismissal } from '../overlays/dismissal';
import {
  addPanel,
  dockLabel,
  hasPanel,
  hidePanel,
  movePanel,
  movePanelBy,
  restorePanel,
  type DockSpec,
  type PanelSpec,
  type Workspace
} from './model';

type MenuItem =
  | { kind: 'header'; label: string }
  | { kind: 'separator' }
  | { kind: 'action'; label: string; disabled?: boolean; run: () => void };

const menuCleanups = new WeakMap<PMRegistry, () => void>();

export function openPanelMenu(
  PM: PMRegistry,
  event: MouseEvent,
  spec: PanelSpec,
  dock: DockSpec,
  trigger: HTMLElement
): HTMLElement | null {
  const ws = PM.Layout.ws as Workspace | null;
  const def = PM.PANELS[spec.id];
  if (!ws || !def) return null;
  menuCleanups.get(PM)?.();
  PM.closeMenus?.();
  const position = dock.panels.findIndex((item) => item.id === spec.id);
  const items: MenuItem[] = [
    { kind: 'header', label: def.title },
    ...(spec.id === 'viewer' ? [] : [{ kind: 'action' as const, label: 'Hide panel', run: () => PM.WS.mutate((w: Workspace) => hidePanel(w, spec.id)) }]),
    { kind: 'separator' },
    { kind: 'header', label: 'Move to' },
    ...(['left', 'center', 'right'] as const).map((dockId) => ({
      kind: 'action' as const,
      label: dockLabel(dockId),
      disabled: dock.id === dockId,
      run: () => PM.WS.mutate((w: Workspace) => movePanel(w, spec.id, dockId))
    })),
    {
      kind: 'action', label: 'Move up', disabled: position <= 0,
      run: () => PM.WS.mutate((w: Workspace) => movePanelBy(w, spec.id, -1))
    },
    {
      kind: 'action', label: 'Move down', disabled: position < 0 || position >= dock.panels.length - 1,
      run: () => PM.WS.mutate((w: Workspace) => movePanelBy(w, spec.id, 1))
    },
    { kind: 'separator' },
    { kind: 'header', label: 'Add panel' },
    ...Object.values(PM.PANELS)
      .filter((panel: any) => !hasPanel(ws, panel.id)
        && !(ws.hiddenPanels ?? []).some((item) => item.id === panel.id)
        && panel.id !== 'toolbar')
      .map((panel: any) => ({
        kind: 'action' as const,
        label: panel.title,
        run: () => PM.WS.mutate((w: Workspace) => addPanel(w, panel.id, dock.id))
      })),
    ...(ws.hiddenPanels ?? [])
      .filter((item) => PM.PANELS[item.id])
      .map((item) => ({
        kind: 'action' as const,
        label: `Restore ${PM.PANELS[item.id].title}`,
        run: () => PM.WS.mutate((w: Workspace) => restorePanel(w, item.id))
      }))
  ];

  const menu = document.createElement('div');
  menu.className = 'drop';
  menu.setAttribute('role', 'menu');
  menu.setAttribute('aria-label', `${def.title} panel options`);
  let dispose = (): void => {};
  const closeMenu = (restoreFocus = true): void => {
    dispose();
    if (menu.isConnected) PM.closeMenus?.();
    if (restoreFocus && trigger.isConnected) trigger.focus({ preventScroll: true });
  };
  for (const item of items) {
    if (item.kind === 'separator') {
      const separator = document.createElement('div');
      separator.className = 'sep';
      separator.setAttribute('role', 'separator');
      menu.appendChild(separator);
      continue;
    }
    if (item.kind === 'header') {
      const header = document.createElement('div');
      header.className = 'hd';
      header.textContent = item.label;
      menu.appendChild(header);
      continue;
    }
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `di${item.disabled ? ' disabled' : ''}`;
    button.setAttribute('role', 'menuitem');
    button.textContent = item.label;
    if (item.disabled) button.setAttribute('aria-disabled', 'true');
    button.tabIndex = -1;
    button.addEventListener('click', (click) => {
      click.stopPropagation();
      if (item.disabled) return;
      closeMenu(false);
      item.run();
    });
    menu.appendChild(button);
  }
  document.body.appendChild(menu);

  const width = menu.offsetWidth;
  const height = menu.offsetHeight;
  menu.style.left = `${PM.clamp(event.clientX, 6, window.innerWidth - width - 6)}px`;
  menu.style.top = `${PM.clamp(event.clientY, 6, window.innerHeight - height - 6)}px`;
  const buttons = [...menu.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]')];
  let active = 0;
  const focusAt = (index: number): void => {
    if (!buttons.length) return;
    active = (index + buttons.length) % buttons.length;
    buttons.forEach((button, buttonIndex) => { button.tabIndex = buttonIndex === active ? 0 : -1; });
    buttons[active]!.focus({ preventScroll: true });
  };
  /* Pointer-opened menus must not paint a row as focused: park focus on the
     menu itself and let the arrow keys move it onto an item. */
  menu.tabIndex = -1;
  menu.focus({ preventScroll: true });

  const onKeyDown = (keyEvent: KeyboardEvent): void => {
    if (!menu.isConnected) return;
    const onItem = buttons.includes(document.activeElement as HTMLButtonElement);
    if (keyEvent.key === 'ArrowDown') focusAt(onItem ? active + 1 : 0);
    else if (keyEvent.key === 'ArrowUp') focusAt(onItem ? active - 1 : buttons.length - 1);
    else if (keyEvent.key === 'Home') focusAt(0);
    else if (keyEvent.key === 'End') focusAt(buttons.length - 1);
    else if (keyEvent.key === 'Escape') closeMenu();
    else if (keyEvent.key === 'Tab') closeMenu(false);
    else return;
    if (keyEvent.key !== 'Tab') keyEvent.preventDefault();
    keyEvent.stopPropagation();
  };
  window.addEventListener('keydown', onKeyDown, true);
  PM._menuOutside = (outsideEvent: PointerEvent) => {
    if (menu.contains(outsideEvent.target as Node)) return;
    markMenuDismissal(outsideEvent);
    closeMenu(false);
  };
  const outside = PM._menuOutside;
  const outsideTimer = window.setTimeout(() => {
    if (menu.isConnected) document.addEventListener('pointerdown', outside, true);
  }, 0);
  dispose = (): void => {
    window.clearTimeout(outsideTimer);
    window.removeEventListener('keydown', onKeyDown, true);
    document.removeEventListener('pointerdown', outside, true);
    if (menuCleanups.get(PM) === dispose) menuCleanups.delete(PM);
  };
  menuCleanups.set(PM, dispose);
  return menu;
}
