// @vitest-environment happy-dom
import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { installSvelteOverlays, unmountSvelteOverlays } from './install';
import { paletteEntries, scorePaletteMatch } from './palette';

let PM: Record<string, any>;

function registry(): Record<string, any> {
  const commands = {
    palette: { id: 'palette', label: 'Command palette', kb: '⌘K', cat: 'View', run: vi.fn() },
    newSolid: { id: 'newSolid', label: 'New solid', kb: '⌘Y', cat: 'Create', run: vi.fn() },
    newText: { id: 'newText', label: 'New text layer', kb: '⌘T', cat: 'Create', run: vi.fn() }
  };
  const current: Record<string, any> = {
    clamp: (value: number, minimum: number, maximum: number) => Math.min(Math.max(value, minimum), maximum),
    commands,
    proj: { layers: [{ id: 'layer-1', name: 'Hero title' }] },
    ICONS: {
      undo: '<path/>', redo: '<path/>', x: '<path/>', trash: '<path/>', export: '<path/>',
      warning: '<path/>', caution: '<path/>', chevD: '<path/>',
      project: '<path/>', layers: '<path/>', plus: '<path/>', hand: '<path/>', cursor: '<path/>',
      eye: '<path/>', music: '<path/>', film: '<path/>', image: '<path/>', panel: '<path/>',
      sparkle: '<path/>', note: '<path/>', missing: '<path/>'
    },
    WS: {
      list: vi.fn(() => [{ id: 'edit', name: 'Editing' }]),
      activate: vi.fn()
    },
    FX: { blur: { label: 'Gaussian blur' } },
    Edit: { apply: vi.fn() },
    firstSel: vi.fn(() => ({ id: 'layer-1' })),
    selectLayers: vi.fn()
  };
  current.cmd = vi.fn((id: string) => current.commands[id]?.run());
  return current;
}

beforeEach(async () => {
  await unmountSvelteOverlays();
  document.body.innerHTML = `
    <div id="app"><button id="trigger" type="button">Open</button><div id="body"></div></div>
    <div id="projects-screen"></div>
    <div id="library-overlay"><section id="library-screen"></section></div>
    <div id="workspace-editbar"></div>
    <div id="scrim"></div>
    <div class="toastwrap" id="toasts"></div>
  `;
  PM = registry();
  installSvelteOverlays(PM);
  flushSync();
});

afterEach(async () => {
  vi.useRealTimers();
  await unmountSvelteOverlays();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe('installSvelteOverlays', () => {
  it('renders a keyboard-accessible easing grid with actual Bézier paths', () => {
    const run = vi.fn();
    const curve = [0, 0, 1, 1];
    const menu = PM.menu(document.body, [
      { header: 'Keyframe' },
      ...Array.from({ length: 8 }, (_, i) => ({ label: `Curve ${i}`, curve, run })),
      '-', { label: 'Delete keyframe', run }
    ]);
    flushSync();
    expect(menu.classList.contains('curve-grid')).toBe(true);
    expect(menu.querySelectorAll('svg.curve-preview')).toHaveLength(8);
    expect(menu.querySelector('path.curve-line')?.getAttribute('d')).toBe('M6 34C6 34 46 6 46 6');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement?.textContent?.trim()).toBe('Curve 4');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    expect(document.activeElement?.textContent?.trim()).toBe('Curve 5');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(run).toHaveBeenCalledOnce();
  });
  it('keeps the shared toast live region intact across consecutive installs', async () => {
    installSvelteOverlays(PM);
    flushSync();
    await Promise.resolve();

    const target = document.querySelector<HTMLElement>('#toasts')!;
    expect(target.getAttribute('role')).toBe('status');
    expect(target.getAttribute('aria-live')).toBe('polite');
    expect(target.getAttribute('aria-atomic')).toBe('false');
  });

  it('restores the previous PM overlay members when unmounted', async () => {
    await unmountSvelteOverlays();
    const legacy = {
      toast: vi.fn(),
      menu: vi.fn(),
      closeMenus: vi.fn(),
      modal: vi.fn(),
      palette: vi.fn(),
      paletteCommand: vi.fn()
    };
    PM.toast = legacy.toast;
    PM.menu = legacy.menu;
    PM.closeMenus = legacy.closeMenus;
    PM.modal = legacy.modal;
    PM.palette = legacy.palette;
    PM.commands.palette.run = legacy.paletteCommand;

    installSvelteOverlays(PM);
    expect(PM.toast).not.toBe(legacy.toast);
    expect(PM.commands.palette.run).toBe(PM.palette);
    await unmountSvelteOverlays();

    expect(PM.toast).toBe(legacy.toast);
    expect(PM.menu).toBe(legacy.menu);
    expect(PM.closeMenus).toBe(legacy.closeMenus);
    expect(PM.modal).toBe(legacy.modal);
    expect(PM.palette).toBe(legacy.palette);
    expect(PM.commands.palette.run).toBe(legacy.paletteCommand);
  });

  it('renders menu actions, separators, and reachable disabled items, then restores focus', () => {
    const trigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    trigger.focus();
    const enabled = vi.fn();
    const disabled = vi.fn();

    const returned = PM.menu(trigger, [
      { header: 'Layer' },
      { label: 'Selected', icon: 'hand', on: true, kb: '⌘1', run: enabled },
      '-',
      { label: 'Unavailable', disabled: true, run: disabled }
    ]);
    flushSync();

    const menu = document.querySelector<HTMLElement>('.drop[role="menu"]')!;
    const items = [...menu.querySelectorAll<HTMLButtonElement>('.di')];
    expect(returned).toBe(menu);
    expect(menu.querySelector('.hd')?.textContent).toBe('Layer');
    expect(menu.querySelector('.sep[role="separator"]')).toBeTruthy();
    expect(items[0]?.classList.contains('on')).toBe(true);
    expect(items[0]?.querySelector('.menu-icon svg')?.getAttribute('data-icon')).toBe('hand');
    expect(items[1]?.querySelector('.menu-icon')).toBeNull();
    expect(items[1]?.getAttribute('aria-disabled')).toBe('true');
    // Pointer-opened: focus parks on the menu, no row is painted; arrows enter the list.
    expect(document.activeElement).toBe(menu);

    menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(items[0]);
    menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(items[1]);
    items[1]?.click();
    expect(disabled).not.toHaveBeenCalled();
    expect(menu.isConnected).toBe(true);

    items[0]?.click();
    expect(enabled).toHaveBeenCalledOnce();
    expect(document.querySelector('.drop')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it('keeps closeMenus compatible with legacy-owned drops and outside listeners', () => {
    const foreign = document.createElement('div');
    foreign.className = 'drop';
    document.body.append(foreign);
    const outside = vi.fn();
    document.addEventListener('pointerdown', outside);
    PM._menuOutside = outside;

    PM.closeMenus();
    document.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));

    expect(foreign.isConnected).toBe(false);
    expect(outside).not.toHaveBeenCalled();
    expect(PM._menuOutside).toBeNull();
  });

  it('captures handled menu keys before global shortcuts and activates with Space or Enter', () => {
    const trigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    const first = vi.fn();
    const second = vi.fn();
    const globalShortcut = vi.fn();
    window.addEventListener('keydown', globalShortcut);

    const open = () => {
      trigger.focus();
      PM.menu(trigger, [
        { label: 'First', run: first },
        { label: 'Second', run: second }
      ]);
      flushSync();
      return [...document.querySelectorAll<HTMLButtonElement>('.drop [role="menuitem"]')];
    };

    try {
      let items = open();
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', cancelable: true }));
      expect(document.activeElement).toBe(items[1]);
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', cancelable: true }));
      expect(document.activeElement).toBe(items[0]);
      window.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', cancelable: true }));
      expect(first).toHaveBeenCalledOnce();
      expect(document.querySelector('.drop')).toBeNull();

      items = open();
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', cancelable: true }));
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));
      expect(second).toHaveBeenCalledOnce();

      open();
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }));
      expect(document.querySelector('.drop')).toBeNull();
      expect(document.activeElement).toBe(trigger);
      expect(globalShortcut).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener('keydown', globalShortcut);
    }
  });

  it('toggles closed on a second trigger press, including nested trigger content', () => {
    vi.useFakeTimers();
    const trigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    trigger.innerHTML = '<span>Open</span>';
    trigger.addEventListener('click', () => PM.menu(trigger, [{ label: 'Action' }]));
    const press = () => {
      const target = trigger.firstElementChild!;
      target.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
      target.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
      target.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      vi.advanceTimersByTime(0);
    };
    press();
    expect(document.querySelector('.drop')).not.toBeNull();
    press();
    expect(document.querySelector('.drop')).toBeNull();
    press();
    expect(document.querySelector('.drop')).not.toBeNull();
  });

  it('switches directly to a different trigger and preserves item activation', () => {
    vi.useFakeTimers();
    const first = document.querySelector<HTMLButtonElement>('#trigger')!;
    const second = document.createElement('button');
    document.body.append(second);
    const run = vi.fn();
    second.onclick = () => PM.menu(second, [{ label: 'Second action', run }]);
    PM.menu(first, [{ label: 'First action' }]);
    vi.advanceTimersByTime(0);
    second.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    second.click();
    expect(document.querySelectorAll('.drop')).toHaveLength(1);
    document.querySelector<HTMLButtonElement>('.drop .di')!.click();
    expect(run).toHaveBeenCalledOnce();
    expect(document.querySelector('.drop')).toBeNull();
  });

  it('closes pointerdown-opened legacy controls without reopening or blocking the next press', () => {
    vi.useFakeTimers();
    const trigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    trigger.onpointerdown = () => PM.menu(trigger, [{ label: 'Action' }]);
    const press = () => {
      trigger.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
      vi.advanceTimersByTime(0);
    };
    press();
    expect(document.querySelector('.drop')).not.toBeNull();
    press();
    expect(document.querySelector('.drop')).toBeNull();
    trigger.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true }));
    press();
    expect(document.querySelector('.drop')).not.toBeNull();
  });

  it('closes a menu on an outside pointer and restores its trigger', () => {
    vi.useFakeTimers();
    const trigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    trigger.focus();
    PM.menu(trigger, [{ label: 'Action', run: vi.fn() }]);
    vi.advanceTimersByTime(0);
    document.querySelector<HTMLElement>('#body')?.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true })
    );

    expect(document.querySelector('.drop')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it('adopts a live modal body, preserves the run false contract, and toggles the scrim', () => {
    const input = document.createElement('input');
    input.value = 'Before';
    const keepOpen = vi.fn(() => false);
    const close = vi.fn();
    const onClose = vi.fn();
    const handle = PM.modal({
      title: 'Edit value',
      body: input,
      width: 390,
      actions: [
        { label: 'Validate', pri: true, run: keepOpen },
        { label: 'Done', run: close }
      ],
      onClose
    });
    flushSync();
    const modalElement = handle.el as HTMLElement;
    const modalBody = handle.body as HTMLElement;

    expect(handle).toEqual({ el: expect.any(HTMLElement), body: expect.any(HTMLElement), close: expect.any(Function) });
    expect(modalElement.classList.contains('modal')).toBe(true);
    expect(modalElement.querySelector('h3')?.textContent).toBe('Edit value');
    expect(modalBody.classList.contains('mb')).toBe(true);
    expect(modalElement.querySelector('.mf')).toBeTruthy();
    expect(modalBody.firstChild).toBe(input);
    input.value = 'After';
    expect(modalBody.querySelector('input')).toBe(input);
    expect(document.querySelector('#scrim')?.classList.contains('on')).toBe(true);

    const buttons = [...modalElement.querySelectorAll<HTMLButtonElement>('button')];
    buttons[0]?.click();
    expect(keepOpen).toHaveBeenCalledOnce();
    expect(handle.el.isConnected).toBe(true);
    buttons[1]?.click();
    expect(close).toHaveBeenCalledOnce();
    expect(onClose).toHaveBeenCalledOnce();
    expect(handle.el.isConnected).toBe(false);
    expect(document.querySelector('#scrim')?.classList.contains('on')).toBe(false);
  });

  it('traps modal focus, closes on Escape, restores inert state, and restores focus', async () => {
    const trigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    trigger.focus();
    const liveBody = document.createElement('div');
    const input = document.createElement('input');
    liveBody.append(input);
    const app = document.querySelector<HTMLElement>('#app')!;
    const hosts = ['projects-screen', 'library-overlay', 'library-screen', 'workspace-editbar']
      .map((id) => document.getElementById(id)!);
    hosts[2]!.inert = true;
    const handle = PM.modal({
      title: 'Focus dialog',
      body: liveBody,
      actions: [{ label: 'Cancel' }, { label: 'Save', pri: true }]
    });
    await Promise.resolve();
    flushSync();

    const modalElement = handle.el as HTMLElement;
    const buttons = [...modalElement.querySelectorAll<HTMLButtonElement>('button')];
    expect(app.inert).toBe(true);
    expect(hosts.every((host) => host.inert)).toBe(true);
    expect(document.activeElement).toBe(input);
    buttons[1]?.focus();
    buttons[1]?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(input);
    input.focus();
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(buttons[1]);

    handle.el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    expect(handle.el.isConnected).toBe(false);
    expect(app.inert).toBe(false);
    expect(hosts.map((host) => host.inert)).toEqual([false, false, true, false]);
    expect(document.activeElement).toBe(trigger);
  });

  it('moves Tab through the middle of a modal while skipping hidden controls', async () => {
    const body = document.createElement('div');
    const first = document.createElement('input');
    const hidden = document.createElement('button');
    const middle = document.createElement('input');
    hidden.hidden = true;
    body.append(first, hidden, middle);
    PM.modal({ title: 'Tab order', body, actions: [{ label: 'Done' }] });
    await Promise.resolve();
    flushSync();

    first.focus();
    first.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(middle);
    middle.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(first);
  });

  it('closes from Escape even when a modal textarea stops bubbling key events', () => {
    const textarea = document.createElement('textarea');
    const stopped = vi.fn((event: KeyboardEvent) => event.stopPropagation());
    textarea.addEventListener('keydown', stopped);
    const handle = PM.modal({ title: 'Edit JSON', body: textarea });
    flushSync();

    textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));

    expect(handle.el.isConnected).toBe(false);
    expect(stopped).not.toHaveBeenCalled();
  });

  it('falls back to #app when an action detaches the modal trigger', () => {
    const trigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    trigger.focus();
    const handle = PM.modal({
      title: 'Delete project',
      actions: [{ label: 'Delete', run: () => trigger.remove() }]
    });
    flushSync();

    (handle.el as HTMLElement).querySelector<HTMLButtonElement>('button')?.click();

    expect(handle.el.isConnected).toBe(false);
    expect(document.activeElement).toBe(document.querySelector('#app'));
    expect(document.activeElement).not.toBe(document.body);
  });

  it('keeps the scrim active and restores the remaining modal when nested modals close', async () => {
    const outerTrigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    outerTrigger.focus();
    const nestedTrigger = document.createElement('button');
    nestedTrigger.textContent = 'Open nested';
    const outer = PM.modal({ title: 'Outer', body: nestedTrigger, actions: [{ label: 'Close outer' }] });
    await Promise.resolve();
    nestedTrigger.focus();
    const inner = PM.modal({ title: 'Inner', actions: [{ label: 'Close inner' }] });
    await Promise.resolve();
    flushSync();

    expect(document.querySelectorAll('[data-svelte-overlay-modal]')).toHaveLength(2);
    expect(document.querySelector('#scrim')?.classList.contains('on')).toBe(true);
    document.querySelector<HTMLElement>('#scrim')?.click();
    expect(inner.el.isConnected).toBe(false);
    expect(outer.el.isConnected).toBe(true);
    expect(document.querySelector('#scrim')?.classList.contains('on')).toBe(true);
    expect(document.activeElement).toBe(nestedTrigger);

    document.querySelector<HTMLElement>('#scrim')?.click();
    expect(outer.el.isConnected).toBe(false);
    expect(document.querySelector('#scrim')?.classList.contains('on')).toBe(false);
    expect(document.activeElement).toBe(outerTrigger);
  });

  it('accepts a string body and closes from the scrim click contract', () => {
    const trigger = document.querySelector<HTMLButtonElement>('#trigger')!;
    trigger.focus();
    const onClose = vi.fn();
    const handle = PM.modal({ title: 'Message', body: 'Plain text body', onClose });
    flushSync();

    expect((handle.body as HTMLElement).textContent).toBe('Plain text body');
    document.querySelector<HTMLElement>('#scrim')?.click();
    expect(handle.el.isConnected).toBe(false);
    expect(onClose).toHaveBeenCalledOnce();
    expect(document.activeElement).toBe(trigger);
  });

  it('replaces the visible toast, uses matching icons, expires immediately, and permits dismissing errors', () => {
    vi.useFakeTimers();
    PM.toast('Undo · Position X', 100);
    flushSync();
    expect(document.querySelectorAll('.toast')).toHaveLength(1);
    expect(document.querySelector('.toast [data-icon="undo"]')).toBeTruthy();

    PM.toast('Redo · Position X', 100);
    flushSync();
    expect(document.querySelectorAll('.toast')).toHaveLength(1);
    expect(document.querySelector('.toast [data-icon="redo"]')).toBeTruthy();

    PM.toast('Save failed', 5000);
    flushSync();

    const target = document.querySelector('#toasts')!;
    expect(target.getAttribute('role')).toBe('status');
    expect(target.getAttribute('aria-live')).toBe('polite');
    expect((target as HTMLElement).style.zIndex).toBe('402');
    expect(document.querySelectorAll('.toast')).toHaveLength(1);
    expect(document.querySelector('.toast[role="alert"]')).toBeTruthy();
    expect(document.querySelector<HTMLElement>('.toast')?.classList.contains('leaving')).toBe(false);
    document.querySelector<HTMLButtonElement>('.toast[data-toast-error="true"] > button')?.click();
    flushSync();
    expect(document.querySelector('.toast')).toBeNull();

    PM.toast('Saved project', 100);
    flushSync();
    expect(document.querySelector('.toast [data-icon="export"]')).toBeTruthy();
    vi.advanceTimersByTime(100);
    flushSync();
    expect(document.querySelector('.toast')).toBeNull();
  });

  it('skips nullish toasts and accepts explicit sticky and dismissible behavior', () => {
    vi.useFakeTimers();
    PM.toast(undefined);
    PM.toast(null);
    PM.toast('Needs attention', 100, { sticky: true, dismissible: true });
    flushSync();

    expect(document.querySelectorAll('.toast')).toHaveLength(1);
    expect(document.querySelector('.toast')?.textContent).not.toContain('undefined');
    vi.advanceTimersByTime(10_000);
    flushSync();
    expect(document.querySelectorAll('.toast')).toHaveLength(1);
    document.querySelector<HTMLButtonElement>('.toast button')?.click();
    flushSync();
    expect(document.querySelector('.toast')).toBeNull();
  });

  it('lets a caller state the outcome instead of sniffing a user-chosen name', () => {
    vi.useFakeTimers();
    // A success that quotes the user's file name must not become a sticky error
    // just because the file is called "error.png".
    PM.toast('Imported error.png', 100, { error: false });
    flushSync();

    expect(document.querySelector('.toast[data-toast-error]')).toBeNull();
    expect(document.querySelector('.toast [data-icon="plus"]')).toBeTruthy();
    expect(document.querySelector('.toast button')).toBeNull();
    vi.advanceTimersByTime(200);
    flushSync();
    expect(document.querySelector('.toast')).toBeNull();

    // A genuine failure whose wording the sniffer does not recognize must still
    // read as one.
    PM.toast('The optimized video did not produce a playable frame', 100, { error: true });
    flushSync();
    expect(document.querySelector('.toast[data-toast-error="true"]')).toBeTruthy();
    vi.advanceTimersByTime(10_000);
    flushSync();
    expect(document.querySelector('.toast[data-toast-error="true"]')).toBeTruthy();
  });

  it('tells an extension alert apart from an editor error', () => {
    vi.useFakeTimers();
    // The same wording from an extension is that extension's alert: the editor
    // is intact, so it gets the alert marker and the extension's name, not the
    // error card and its diagnostics.
    PM.toast('Timeline update failed', 100, { source: { id: 'timeline', name: 'Timeline' } });
    flushSync();

    const alert = document.querySelector<HTMLElement>('.toast[data-toast-kind="alert"]')!;
    expect(alert).toBeTruthy();
    expect(alert.getAttribute('data-toast-error')).toBeNull();
    expect(alert.textContent).toContain('Timeline');
    expect(alert.textContent).toContain('Timeline update failed');
    expect(alert.querySelector('[data-icon="caution"]')).toBeTruthy();
    expect(alert.querySelector('.disclosure')).toBeNull();

    // It stays put like an error, because it still needs reading.
    vi.advanceTimersByTime(10_000);
    flushSync();
    expect(document.querySelector('.toast[data-toast-kind="alert"]')).toBeTruthy();
    document.querySelector<HTMLButtonElement>('.toast[data-toast-kind="alert"] > button')?.click();
    flushSync();
    expect(document.querySelector('.toast')).toBeNull();

    // The editor failing the same operation is still an error.
    PM.toast('Timeline update failed', 100);
    flushSync();
    expect(document.querySelector('.toast[data-toast-error="true"]')).toBeTruthy();
    expect(document.querySelector('.toast[data-toast-kind="error"]')).toBeTruthy();
  });

  it('keeps an extension notice out of the error family even when it states a failure', () => {
    vi.useFakeTimers();
    PM.toast('Could not import scene.obj', 100, { error: true, source: { id: 'layers-3d', name: '3D layers' } });
    flushSync();

    expect(document.querySelector('.toast[data-toast-error]')).toBeNull();
    expect(document.querySelector('.toast[data-toast-kind="alert"]')?.textContent).toContain('3D layers');
  });

  it('lets a routine extension notice pass on its own', () => {
    vi.useFakeTimers();
    PM.toast('Copied 2 effects', 100, { source: { id: 'inspector', name: 'Inspector' } });
    flushSync();

    expect(document.querySelector('.toast[data-toast-kind="status"]')).toBeTruthy();
    expect(document.querySelector('.toast')?.textContent).not.toContain('Inspector');
    vi.advanceTimersByTime(200);
    flushSync();
    expect(document.querySelector('.toast')).toBeNull();
  });

  it('keeps errors available until dismissed', () => {
    vi.useFakeTimers();
    PM.toast('Save failed', 100);
    flushSync();

    expect(document.querySelector('.toast button')).toBeTruthy();
    vi.advanceTimersByTime(10_000);
    flushSync();
    expect(document.querySelector('.toast')).not.toBeNull();
  });

  it('routes the installed palette command through keyboard-style navigation and PM.cmd', () => {
    PM.commands.palette.run();
    flushSync();
    const input = document.querySelector<HTMLInputElement>('#palette input[role="combobox"]')!;
    expect(input.getAttribute('aria-label')).toContain('Search commands');
    expect(document.querySelector<HTMLElement>('#projects-screen')?.inert).toBe(true);
    input.value = 'new solid';
    input.dispatchEvent(new InputEvent('input', { bubbles: true }));
    flushSync();
    expect(document.querySelectorAll('#palette [role="option"]')).toHaveLength(1);

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(PM.cmd).toHaveBeenCalledWith('newSolid');
    expect(document.querySelector('#palette')).toBeNull();
    expect(document.querySelector('#scrim')?.classList.contains('on')).toBe(false);
    expect(document.querySelector<HTMLElement>('#projects-screen')?.inert).toBe(false);
  });
});

describe('scorePaletteMatch legacy parity', () => {
  it.each([
    ['New solid', '', 0],
    ['New solid', 'new', 0],
    ['New solid', 'SOL', 4],
    ['Workspace · Editing', ' editing ', 12],
    ['Gaussian blur', 'blur', 9],
    ['New solid', 'nsl', null]
  ])('scores %j against %j as %j', (value, query, expected) => {
    expect(scorePaletteMatch(value, query)).toBe(expected);
  });
});

describe('paletteEntries legacy grouping and cap', () => {
  it.each([
    ['', ['Command', 'Workspace']],
    ['match', ['Command', 'Layer', 'Workspace', 'Effect']]
  ])('keeps group order for query %j', (query, expected) => {
    const model = registry();
    model.commands = {
      match: { id: 'duplicate', label: 'Match command', cat: 'Command' }
    };
    model.proj.layers = [{ id: 'duplicate', name: 'Match layer' }];
    model.WS.list = () => [{ id: 'duplicate', name: 'Match workspace' }];
    model.FX = { duplicate: { label: 'Match effect' } };

    expect(paletteEntries(model, query).map((entry) => entry.cat)).toEqual(expected);
  });

  it.each([
    [58, 5, 58, 2],
    [65, 5, 60, 0]
  ])('caps %i commands plus %i workspaces at 60', (commandCount, workspaceCount, commandsKept, workspacesKept) => {
    const model = registry();
    model.commands = Object.fromEntries(Array.from({ length: commandCount }, (_, index) => [
      `command-${index}`,
      { id: `command-${index}`, label: `Command ${index}`, cat: 'Command' }
    ]));
    model.WS.list = () => Array.from({ length: workspaceCount }, (_, index) => ({
      id: `workspace-${index}`,
      name: `Workspace ${index}`
    }));

    const entries = paletteEntries(model, '');
    expect(entries).toHaveLength(60);
    expect(entries.filter((entry) => entry.cat === 'Command')).toHaveLength(commandsKept);
    expect(entries.filter((entry) => entry.cat === 'Workspace')).toHaveLength(workspacesKept);
  });
});

it('preserves distinct failures across success notifications and deduplicates repeated errors', () => {
  PM.toast('Could not import first.mov');
  PM.toast('Could not import second.mov');
  PM.toast('Could not import first.mov');
  PM.toast('Saved project');
  flushSync();
  expect(document.querySelectorAll('[data-toast-error]')).toHaveLength(2);
  expect(document.querySelectorAll('.toast')).toHaveLength(3);
  expect(document.querySelector('#toasts')?.textContent).toContain('first.mov');
  expect(document.querySelector('#toasts')?.textContent).toContain('second.mov');
});

it('waits for asynchronous modal validation without closing or submitting twice', async () => {
  let finish!: (result: boolean) => void;
  const run = vi.fn(() => new Promise<boolean>(resolve => { finish = resolve; }));
  const handle = PM.modal({ title: 'Save project', actions: [{ label: 'Save', run }] });
  flushSync();
  const button = handle.el.querySelector('button') as HTMLButtonElement;
  button.click();
  flushSync();
  expect(handle.el.isConnected).toBe(true);
  expect(button.disabled).toBe(true);
  button.click();
  expect(run).toHaveBeenCalledOnce();
  finish(false);
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
  expect(handle.el.isConnected).toBe(true);
  expect(button.disabled).toBe(false);
});

it('keeps failed modal actions open with useful feedback', async () => {
  const run = vi.fn().mockRejectedValueOnce(new Error('ENOSPC: no space left on device')).mockResolvedValueOnce(true);
  const handle = PM.modal({ title: 'Save project', actions: [{ label: 'Save', run }] });
  flushSync();
  const button = handle.el.querySelector('.mf button') as HTMLButtonElement;
  button.click();
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
  expect(handle.el.isConnected).toBe(true);
  expect(handle.el.querySelector('[role="alert"]')?.textContent).toContain('Your disk is full');
  expect(button.disabled).toBe(false);
  button.click();
  await Promise.resolve();
  await Promise.resolve();
  expect(handle.el.isConnected).toBe(false);
});
