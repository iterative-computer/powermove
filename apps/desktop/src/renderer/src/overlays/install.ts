import { flushSync, mount, unmount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import { MenuController } from './Menu';
import { ModalController } from './modal';
import { PaletteController } from './palette';
import Toasts from './Toasts.svelte';
import type { MenuItem, MenuOptions, ModalOptions, ToastOptions } from './types';

type ToastInstance = ReturnType<typeof mount> & {
  push(message: unknown, milliseconds?: number, options?: ToastOptions): void;
  clear(): void;
};

type InstalledOverlays = {
  PM: PMRegistry;
  target: HTMLElement;
  toasts: ToastInstance;
  menu: MenuController;
  modal: ModalController;
  palette: PaletteController;
  previous: OverlayMembers;
  members: OverlayMembers;
  attributes: {
    role: string | null;
    live: string | null;
    atomic: string | null;
    zIndex: string;
  };
};

type OverlayMembers = {
  toast: PMRegistry['toast'];
  menu: PMRegistry['menu'];
  closeMenus: PMRegistry['closeMenus'];
  modal: PMRegistry['modal'];
  palette: PMRegistry['palette'];
  paletteCommand: PMRegistry['commands']['palette']['run'];
};

let installed: InstalledOverlays | null = null;

export function installSvelteOverlays(PM: PMRegistry): void {
  const target = document.getElementById('toasts');
  if (!target) return;

  if (installed) void dispose(installed);
  const previous: OverlayMembers = {
    toast: PM.toast,
    menu: PM.menu,
    closeMenus: PM.closeMenus,
    modal: PM.modal,
    palette: PM.palette,
    paletteCommand: PM.commands?.palette?.run
  };
  const attributes = {
    role: target.getAttribute('role'),
    live: target.getAttribute('aria-live'),
    atomic: target.getAttribute('aria-atomic'),
    zIndex: target.style.zIndex
  };
  target.setAttribute('role', 'status');
  target.setAttribute('aria-live', 'polite');
  target.setAttribute('aria-atomic', 'false');
  target.style.zIndex = '402';
  const toasts = mount(Toasts, { target, props: { PM } }) as ToastInstance;
  const menu = new MenuController(PM);
  const modal = new ModalController(PM);
  const palette = new PaletteController(PM);
  const openPalette = (): void => palette.open();
  const members: OverlayMembers = {
    toast: (message: unknown, milliseconds = 2200, options: ToastOptions = {}): void => {
      flushSync(() => toasts.push(message, milliseconds, options));
    },
    menu: (anchor: HTMLElement, items: MenuItem[], options: MenuOptions = {}): HTMLElement =>
      menu.open(anchor, items, options),
    closeMenus: (): void => menu.close(false),
    modal: (options: ModalOptions) => modal.open(options),
    palette: openPalette,
    paletteCommand: openPalette
  };
  installed = { PM, target, toasts, menu, modal, palette, previous, members, attributes };
  PM.toast = members.toast;
  PM.menu = members.menu;
  PM.closeMenus = members.closeMenus;
  PM.modal = members.modal;
  PM.palette = members.palette;
  if (PM.commands?.palette) PM.commands.palette.run = members.paletteCommand;
  flushSync();
}

async function dispose(current: InstalledOverlays): Promise<void> {
  if (installed === current) installed = null;
  current.menu.close(false);
  current.modal.closeAll();
  current.palette.close();
  current.toasts.clear();
  restoreAttribute(current.target, 'role', current.attributes.role);
  restoreAttribute(current.target, 'aria-live', current.attributes.live);
  restoreAttribute(current.target, 'aria-atomic', current.attributes.atomic);
  current.target.style.zIndex = current.attributes.zIndex;
  if (current.PM.toast === current.members.toast) current.PM.toast = current.previous.toast;
  if (current.PM.menu === current.members.menu) current.PM.menu = current.previous.menu;
  if (current.PM.closeMenus === current.members.closeMenus) current.PM.closeMenus = current.previous.closeMenus;
  if (current.PM.modal === current.members.modal) current.PM.modal = current.previous.modal;
  if (current.PM.palette === current.members.palette) current.PM.palette = current.previous.palette;
  if (current.PM.commands?.palette?.run === current.members.paletteCommand) {
    current.PM.commands.palette.run = current.previous.paletteCommand;
  }
  await unmount(current.toasts);
}

function restoreAttribute(target: HTMLElement, name: string, value: string | null): void {
  if (value == null) target.removeAttribute(name);
  else target.setAttribute(name, value);
}

/** Test/HMR seam. */
export async function unmountSvelteOverlays(): Promise<void> {
  if (installed) await dispose(installed);
}
