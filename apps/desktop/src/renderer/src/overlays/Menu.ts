import { flushSync, mount, unmount } from 'svelte';

import Menu from './Menu.svelte';
import type { MenuAction, MenuItem, MenuOptions, OverlayPM } from './types';
import { consumeMenuTriggerPress, markMenuDismissal } from './dismissal';
import { canRenderNatively, iconRasterizer, planNativeMenu } from './native-menu';
import { bridge } from '../kernel/bridge';
import type { PowermoveBridge } from '../../../shared/ipc';

type MenuInstance = ReturnType<typeof mount> & { element(): HTMLElement };

const MODIFIER_KEYS = new Set(['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'Fn']);

function nativeMenuBridge(): NonNullable<PowermoveBridge['menu']> | null {
  try {
    const menu = bridge()?.menu;
    return menu && typeof menu.popup === 'function' ? menu : null;
  } catch {
    return null;
  }
}

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
  /* Bumped by every open and close: a menu whose items are still arriving
     opens only if nothing opened or closed menus since it was asked for. */
  private generation = 0;

  constructor(private readonly PM: OverlayPM) {}

  /** `items` may be a Promise (contributions that answer asynchronously); the
      menu then opens when it settles with any, unless another open or a close
      came first, or the person pressed, typed or changed the selection
      meanwhile: its items act on what was true when it was asked. */
  open(anchor: HTMLElement, items: MenuItem[] | PromiseLike<MenuItem[]>, options: MenuOptions = {}): HTMLElement {
    this.close(false);
    if (!Array.isArray(items)) {
      const asked = this.generation;
      const stale = (): void => { if (asked === this.generation) this.generation += 1; };
      const typed = (event: KeyboardEvent): void => { if (!MODIFIER_KEYS.has(event.key)) stale(); };
      let offSelection: (() => void) | undefined;
      // From the next task on, so the press that asked for this menu is not one.
      const listening = window.setTimeout(() => {
        document.addEventListener('pointerdown', stale, true);
        document.addEventListener('keydown', typed, true);
        const off = this.PM.bus?.on?.('sel', stale);
        offSelection = typeof off === 'function' ? off : undefined;
      }, 0);
      const settled = (): void => {
        window.clearTimeout(listening);
        document.removeEventListener('pointerdown', stale, true);
        document.removeEventListener('keydown', typed, true);
        offSelection?.();
      };
      void Promise.resolve(items).then((ready) => {
        settled();
        if (asked === this.generation && ready.length) this.open(anchor, ready, options);
      }, settled);
      return document.createElement('div');
    }
    this.removeForeignMenus();
    this.trigger = focusTarget(anchor);
    const rect = anchor.getBoundingClientRect();
    const cursorOrigin = options.x != null || options.y != null;
    /* Every menu is a real NSMenu when the desktop bridge is present, so
       pointer menus and button dropdowns share one design. Anchored menus pop
       from the anchor's bottom-left, as NSPopUpButton does. Only curve pickers
       stay in the DOM: they draw bezier previews. */
    const native = nativeMenuBridge();
    if (native && canRenderNatively(items)) {
      const x = options.x ?? rect.left;
      const y = options.y ?? rect.bottom + 4;
      /* Icons rasterize once per name and are cached, so the await is a
         microtask after the first menu. */
      void planNativeMenu(items, iconRasterizer(this.PM.ICONS as Record<string, string> | undefined))
        .then((plan) => native.popup({ items: plan.items, x, y })
          .then((id) => { if (id != null) plan.actions.get(id)?.run?.(); }))
        .catch(() => undefined);
      return document.createElement('div');
    }
    const x = options.x ?? (options.right ? rect.right : rect.left);
    const y = options.y ?? rect.bottom + 5;
    let instance: MenuInstance;
    instance = mount(Menu, {
      target: document.body,
      props: {
        PM: this.PM,
        items,
        x,
        y,
        cursorOrigin,
        label: this.menuLabel(items),
        onrun: (item: MenuAction) => {
          /* Without the native bridge a submenu opens in place, with a way back. */
          if (item.submenu) {
            const parent = items;
            this.open(anchor, [
              { label: '‹ Back', run: () => this.open(anchor, parent, options) },
              { header: item.label },
              ...item.submenu
            ], options);
            return;
          }
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
      if (!cursorOrigin) consumeMenuTriggerPress(event, anchor);
      this.close(true);
    };
    this.PM._menuOutside = this.outside;
    this.outsideTimer = window.setTimeout(() => {
      if (this.outside && this.menu?.isConnected) document.addEventListener('pointerdown', this.outside, true);
    }, 0);
    return this.menu;
  }

  close(restoreFocus = false): void {
    this.generation += 1;
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
