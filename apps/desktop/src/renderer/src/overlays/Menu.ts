import { flushSync, mount, unmount } from 'svelte';

import Menu from './Menu.svelte';
import type { MenuAction, MenuItem, MenuOptions, OverlayPM } from './types';
import { consumeMenuTriggerPress, markMenuDismissal } from './dismissal';
import { tidyMenu } from './menu-rules';
import { prefersReducedMotion } from '../controls/menu-blink';

type MenuInstance = ReturnType<typeof mount> & { element(): HTMLElement; contains(node: Node | null): boolean };

const MODIFIER_KEYS = new Set(['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'Fn']);

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
    const rect = anchor.getBoundingClientRect();
    const cursorOrigin = options.x != null || options.y != null;
    items = tidyMenu(items);
    if (!items.some((item) => item !== '-' && !('header' in item))) return document.createElement('div');
    this.trigger = focusTarget(anchor);
    /* Pointer menus and button menus share one design, Powermove's own
       (Menu.svelte). Anchored menus pop from the anchor's bottom-left, as
       NSPopUpButton does. */
    const x = options.x ?? (options.right ? rect.right : rect.left);
    const y = options.y ?? rect.bottom + 4;
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
          const ghost = this.ghost();
          this.close(true);
          if (ghost) document.body.append(ghost);
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
      if (!this.menu || this.instance?.contains(event.target as Node) || this.menu.contains(event.target as Node)) return;
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

  /** A still copy of the open menu that blinks its chosen row and fades,
      the way an AppKit menu acknowledges a pick; the command never waits on it. */
  private ghost(): HTMLElement | null {
    const host = this.menu?.parentElement;
    if (!host || prefersReducedMotion()) return null;
    const ghost = host.cloneNode(true) as HTMLElement;
    ghost.className = 'menu-ghost';
    ghost.removeAttribute('data-svelte-overlay-menu');
    ghost.setAttribute('aria-hidden', 'true');
    ghost.querySelectorAll('[role]').forEach((element) => element.removeAttribute('role'));
    ghost.querySelectorAll('.drop').forEach((element) => element.classList.remove('drop'));
    ghost.addEventListener('animationend', (event) => { if (event.target === ghost) ghost.remove(); });
    window.setTimeout(() => ghost.remove(), 600);
    return ghost;
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
