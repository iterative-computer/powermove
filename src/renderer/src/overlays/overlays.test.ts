// @vitest-environment happy-dom
import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { installSvelteOverlays, unmountSvelteOverlays } from './install';
import { paletteEntries, scorePaletteMatch } from './palette';

let PM: Record<string, any>;

function registry(svelte = true): Record<string, any> {
  const commands = {
    palette: { id: 'palette', label: 'Command palette', kb: '⌘K', cat: 'View', run: vi.fn() },
    newSolid: { id: 'newSolid', label: 'New solid', kb: '⌘Y', cat: 'Create', run: vi.fn() },
    newText: { id: 'newText', label: 'New text layer', kb: '⌘T', cat: 'Create', run: vi.fn() }
  };
  const current: Record<string, any> = {
    SvelteShell: svelte,
    clamp: (value: number, minimum: number, maximum: number) => Math.min(Math.max(value, minimum), maximum),
    commands,
    proj: { layers: [{ id: 'layer-1', name: 'Hero title' }] },
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
  it('is a strict no-op while the Svelte shell switch is off', async () => {
    await unmountSvelteOverlays();
    const legacyToast = vi.fn();
    const legacyMenu = vi.fn();
    const legacyCloseMenus = vi.fn();
    const legacyModal = vi.fn();
    const legacyPalette = vi.fn();
    const off = registry(false);
    off.toast = legacyToast;
    off.menu = legacyMenu;
    off.closeMenus = legacyCloseMenus;
    off.modal = legacyModal;
    off.palette = legacyPalette;

    installSvelteOverlays(off);

    expect(off.toast).toBe(legacyToast);
    expect(off.menu).toBe(legacyMenu);
    expect(off.closeMenus).toBe(legacyCloseMenus);
    expect(off.modal).toBe(legacyModal);
    expect(off.palette).toBe(legacyPalette);
    expect(document.querySelector('[data-svelte-overlay-menu]')).toBeNull();
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
      { label: 'Selected', on: true, kb: '⌘1', run: enabled },
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
    expect(items[1]?.getAttribute('aria-disabled')).toBe('true');
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

  it('queues, fades, auto-dismisses, and permits dismissing error toasts', () => {
    vi.useFakeTimers();
    PM.toast('Saved', 100);
    PM.toast('Save failed', 5000);
    flushSync();

    const target = document.querySelector('#toasts')!;
    expect(target.getAttribute('role')).toBe('status');
    expect(target.getAttribute('aria-live')).toBe('polite');
    expect((target as HTMLElement).style.zIndex).toBe('402');
    expect(document.querySelectorAll('.toast')).toHaveLength(2);
    expect(document.querySelector<HTMLElement>('.toast')?.style.display).toBe('flex');
    document.querySelector<HTMLButtonElement>('.toast[data-toast-error="true"] button')?.click();
    flushSync();
    expect(document.querySelectorAll('.toast')).toHaveLength(1);

    vi.advanceTimersByTime(100);
    flushSync();
    expect(document.querySelector<HTMLElement>('.toast')?.style.opacity).toBe('0');
    vi.advanceTimersByTime(260);
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

  it('keeps regex-inferred dismiss buttons backward compatible without making errors sticky', () => {
    vi.useFakeTimers();
    PM.toast('Save failed', 100);
    flushSync();

    expect(document.querySelector('.toast button')).toBeTruthy();
    vi.advanceTimersByTime(360);
    flushSync();
    expect(document.querySelector('.toast')).toBeNull();
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
