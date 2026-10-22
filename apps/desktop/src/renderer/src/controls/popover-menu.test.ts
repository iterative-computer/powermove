// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { openPopoverMenu } from './popover-menu';

beforeEach(() => {
  vi.stubGlobal('matchMedia', () => ({ matches: true }));
});
afterEach(() => {
  window.dispatchEvent(new Event('blur'));
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});
function trigger(label: string) {
  const button = document.createElement('button');
  const icon = document.createElement('span');
  button.append(icon);
  document.body.append(button);
  const run = vi.fn();
  const onClose = vi.fn();
  button.onclick = () => { openPopoverMenu({ anchor: button, label, items: [{ label: 'Action', run }], onClose }); };
  return { button, icon, run, onClose };
}
function press(node: HTMLElement) {
  node.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
  node.click();
}

describe('popover trigger toggling', () => {
  it('closes on a second trigger press, including its icon, and reopens on the next press', () => {
    const { button, icon, onClose, run } = trigger('Extension actions');
    press(icon);
    expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
    expect(button.getAttribute('aria-expanded')).toBe('true');
    press(icon);
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(button);
    expect(onClose).toHaveBeenCalledOnce();
    expect(run).not.toHaveBeenCalled();
    press(button);
    expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
  });
  it('switches to another trigger and dismisses on outside press', () => {
    const a = trigger('First'), b = trigger('Second');
    press(a.button); press(b.button);
    expect(document.querySelector('[role="menu"]')?.getAttribute('aria-label')).toBe('Second');
    expect(a.onClose).toHaveBeenCalledOnce();
    expect(a.button.getAttribute('aria-expanded')).toBe('false');
    press(document.body);
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(b.onClose).toHaveBeenCalledOnce();
  });
  it('supports keyboard activation, Escape, and menu actions', () => {
    const { button, run } = trigger('Actions');
    button.click(); button.click(); // Keyboard-generated clicks have no pointerdown.
    expect(document.querySelector('[role="menu"]')).toBeNull();
    button.click();
    document.querySelector('[role="menu"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(document.querySelector('[role="menu"]')).toBeNull();
    button.click();
    document.querySelector<HTMLElement>('[role="menuitem"]')!.click();
    expect(run).toHaveBeenCalledOnce();
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });
});

describe('popover sections and check items', () => {
  function open(items: Parameters<typeof openPopoverMenu>[0]['items']) {
    const anchor = document.createElement('button');
    document.body.append(anchor);
    return openPopoverMenu({ anchor, label: 'Preview quality', items });
  }
  it('groups each section under its heading, one radio group per section', () => {
    const pick = vi.fn();
    const menu = open([
      { heading: 'Resolution' },
      { label: 'Automatic', checked: true, run: pick },
      { label: 'Full', checked: false, run: pick },
      { heading: 'Media' },
      { label: 'Automatic', checked: false, run: pick },
      { label: 'Proxies', checked: true, run: pick },
      '-',
      { label: 'Play Every Frame', check: true, checked: true, run: pick }
    ]);
    const groups = [...menu.el.querySelectorAll<HTMLElement>('[role="group"]')];
    expect(groups.map((group) => document.getElementById(group.getAttribute('aria-labelledby')!)?.textContent)).toEqual(['Resolution', 'Media']);
    expect(groups.map((group) => [...group.querySelectorAll('[role="menuitemradio"]')].map((row) => row.getAttribute('aria-checked'))))
      .toEqual([['true', 'false'], ['false', 'true']]);
    // The check item stays out of both groups, after the separator.
    const check = menu.el.querySelector<HTMLElement>('[role="menuitemcheckbox"]')!;
    expect(check.parentElement).toBe(menu.el);
    expect(check.getAttribute('aria-checked')).toBe('true');
    expect(check.previousElementSibling?.getAttribute('role')).toBe('separator');
    // Headings are not rows: arrow keys walk the five rows only.
    expect(menu.el.querySelectorAll('.pm-menu-item')).toHaveLength(5);
    for (let i = 0; i < 5; i++) menu.el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    menu.el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(pick).toHaveBeenCalledOnce();
  });
  it('registers the popover service for built-ins', async () => {
    const { installPopoverMenuService, POPOVER_MENU_SERVICE } = await import('./popover-menu');
    const register = vi.fn();
    installPopoverMenuService({ Kernel: { services: { register } } });
    expect(register).toHaveBeenCalledWith(POPOVER_MENU_SERVICE, expect.objectContaining({ open: openPopoverMenu }));
  });
});
