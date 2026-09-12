import type { PMRegistry } from '../legacy/registry';
import { consumeMenuTriggerPress, markMenuDismissal } from '../overlays/dismissal';
import { positionMenuAtCursor } from '../overlays/position';
import {
  hidePanel,
  type DockSpec,
  type PanelSpec,
  type Workspace
} from './model';
import { canPopoutPanel } from './popout';

type MenuItem =
  | { kind: 'header'; label: string }
  | { kind: 'separator' }
  | { kind: 'action'; label: string; disabled?: boolean; run: () => void };

const menuCleanups = new WeakMap<PMRegistry, () => void>();

/** `menus.contribute('panel:context', …)` entries, translated to menu rows. */
function panelContextContributions(PM: PMRegistry, panelId: string, dockId: string): MenuItem[] {
  const contributions = PM.Kernel?.collectMenu?.('panel:context', { panelId, dockId }) ?? [];
  const items: MenuItem[] = [];
  for (const contribution of contributions as Array<any>) {
    if (contribution === '-') items.push({ kind: 'separator' });
    else if (contribution && typeof contribution.header === 'string') items.push({ kind: 'header', label: contribution.header });
    else if (contribution && typeof contribution.label === 'string') {
      items.push({
        kind: 'action',
        label: contribution.label,
        disabled: contribution.disabled === true,
        run: () => void contribution.run?.()
      });
    }
  }
  return items.length ? [{ kind: 'separator' }, ...items] : [];
}

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
  const items: MenuItem[] = [
    { kind: 'header', label: def.title },
    ...(canPopoutPanel(spec.id) ? [{ kind: 'action' as const, label: 'Pop out to window', run: () => PM.Popout?.open?.(spec.id) }] : []),
    ...(spec.id === 'viewer' ? [] : [{ kind: 'action' as const, label: 'Close panel', run: () => PM.WS.mutate((w: Workspace) => hidePanel(w, spec.id)) }]),
    /* Extension contributions land at the end so they never shift the
       positions a user has learned for the built-in rows. */
    ...panelContextContributions(PM, spec.id, dock.id)
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
  const placement = positionMenuAtCursor(
    event.clientX, event.clientY, width, height, window.innerWidth, window.innerHeight,
  );
  menu.style.left = `${placement.left}px`;
  menu.style.top = `${placement.top}px`;
  menu.dataset.side = placement.side;
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
    consumeMenuTriggerPress(outsideEvent, trigger);
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
