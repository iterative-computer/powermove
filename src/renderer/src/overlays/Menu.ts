import { flushSync, mount, unmount } from 'svelte';

import Menu from './Menu.svelte';
import type { MenuAction, MenuItem, MenuOptions, OverlayPM } from './types';
import { markMenuDismissal } from './dismissal';

type MenuInstance = ReturnType<typeof mount> & { element(): HTMLElement };

function focusTarget(anchor: HTMLElement): HTMLElement | null {
  const active = document.activeElement;
  if (active instanceof HTMLElement && active !== document.body) return active;
  return anchor !== document.body ? anchor : null;
}

export class MenuController {
  private instance: MenuInstance | null = null;
  private menu: HTMLElement | null = null;
  private outside: ((event: PointerEvent) => void) | null = null;
  private outsideTimer = 0;
  private trigger: HTMLElement | null = null;

  constructor(private readonly PM: OverlayPM) {}

  open(anchor: HTMLElement, items: MenuItem[], options: MenuOptions = {}): HTMLElement {
    this.close(false);
    this.removeForeignMenus();
    this.trigger = focusTarget(anchor);
    const rect = anchor.getBoundingClientRect();
    const x = options.x ?? (options.right ? rect.right : rect.left);
    const y = options.y ?? rect.bottom + 5;
    let instance: MenuInstance;
    instance = mount(Menu, {
      target: document.body,
      props: {
        items,
        x,
        y,
        label: this.menuLabel(items),
        onrun: (item: MenuAction) => {
          this.close(true);
          item.run?.();
        },
        onclose: (restoreFocus = true) => this.close(restoreFocus)
      }
    }) as MenuInstance;
    this.instance = instance;
    flushSync();
    this.menu = instance.element();
    if (options.right && options.x == null) {
      const width = this.menu.offsetWidth;
      this.menu.style.left = `${this.clamp(rect.right - width, 6, window.innerWidth - width - 6)}px`;
    }
    this.outside = (event: PointerEvent) => {
      if (!this.menu || this.menu.contains(event.target as Node)) return;
      markMenuDismissal(event);
      this.close(true);
    };
    this.PM._menuOutside = this.outside;
    this.outsideTimer = window.setTimeout(() => {
      if (this.outside && this.menu?.isConnected) document.addEventListener('pointerdown', this.outside, true);
    }, 0);
    return this.menu;
  }

  close(restoreFocus = false): void {
    window.clearTimeout(this.outsideTimer);
    if (this.outside) document.removeEventListener('pointerdown', this.outside, true);
    const registeredOutside = this.PM._menuOutside as ((event: PointerEvent) => void) | null | undefined;
    if (registeredOutside && registeredOutside !== this.outside) {
      document.removeEventListener('pointerdown', registeredOutside, true);
    }
    this.PM._menuOutside = null;
    this.outside = null;
    const instance = this.instance;
    this.instance = null;
    const menu = this.menu;
    this.menu = null;
    if (instance) void unmount(instance);
    menu?.remove();
    document.querySelectorAll('.drop').forEach((element) => {
      if (element !== menu) element.remove();
    });
    const trigger = this.trigger;
    this.trigger = null;
    if (restoreFocus && trigger?.isConnected) trigger.focus({ preventScroll: true });
  }

  private removeForeignMenus(): void {
    const outside = this.PM._menuOutside as ((event: PointerEvent) => void) | null | undefined;
    if (outside && outside !== this.outside) document.removeEventListener('pointerdown', outside, true);
    this.PM._menuOutside = null;
    document.querySelectorAll('.drop').forEach((element) => element.remove());
  }

  private clamp(value: number, minimum: number, maximum: number): number {
    return this.PM.clamp?.(value, minimum, maximum) ?? Math.min(Math.max(value, minimum), maximum);
  }

  private menuLabel(items: MenuItem[]): string {
    const header = items.find((item): item is { header: string } => item !== '-' && 'header' in item);
    return header?.header ? `${header.header} menu` : 'Menu';
  }
}
