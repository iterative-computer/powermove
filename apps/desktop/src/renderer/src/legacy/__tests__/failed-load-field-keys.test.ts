// @vitest-environment happy-dom
// @ts-nocheck -- legacy PM is intentionally a dynamic registry.
/* After a failed strict document load, the keys and Edit-menu items that edit
   a focused text field must behave exactly as without the new engine: they
   never touch the document. Only their document fallback (paste layers, undo,
   ...) shows the engine's named failure. Real kernel key listener, real
   built-in commands and the real default keymap. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import activateKeymap from '../../../../extensions/keymap-default/index';
import { installKernel } from '../../kernel/install';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { install as installShortcuts } from '../ui/shortcuts';
import { runCommandWhenReady } from '../../engine-doc/readiness';

let PM: any;
let installed: any;
let bridge: any;
let menuCommand: (command: string) => void;

function failedDoc() {
  return {
    enabled: true, strict: true, ready: false, whenReady: Promise.resolve(false),
    settled: () => Promise.resolve(false), showFailure: vi.fn(), onReady: () => () => {},
  };
}

beforeEach(() => {
  bridge = {
    nativeEdit: vi.fn(),
    pasteMedia: vi.fn(async () => false),
  };
  installBridgeForTests(bridge);
  PM = {
    h: () => ({ focus() {}, select() {}, value: '' }),
    proj: { w: 1920, h: 1080, fps: 30, dur: 8, layers: [], assets: {}, work: [0, 8] },
    time: 1,
    snapF: (value: number) => value,
    store: { get: (_key: string, fallback: unknown) => fallback, set() {} },
    bus: { on: () => () => {}, emit() {} },
    toast: vi.fn(),
    EngineDoc: failedDoc(),
  };
  installed = installKernel(PM);
  installShortcuts(PM);
  activateKeymap({ keybindings: { bind: (definition: any) => PM.Kernel.bind('keymap-default', definition) } } as any);
  // The native menu runner (legacy/host/electron-shim.ts onMenuCommand).
  menuCommand = command => { runCommandWhenReady(PM, command, () => PM.cmd(command)); };
});

afterEach(() => {
  installed?.uninstall?.();
  resetBridgeForTests();
  document.body.replaceChildren();
});

const settle = () => new Promise(resolve => setTimeout(resolve, 0));
function press(init: KeyboardEventInit, target: EventTarget): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}
function focusedInput(): HTMLInputElement {
  const input = document.createElement('input');
  document.body.append(input);
  input.focus();
  expect(document.activeElement).toBe(input);
  return input;
}

describe('field editing after a failed strict document load', () => {
  it('pastes into a focused input with Cmd+V and blurs it with Escape', async () => {
    const pasteLayers = vi.fn();
    PM.Kernel.commands.register('legacy', { id: 'pasteLayers', label: 'Paste layers', run: pasteLayers });
    const input = focusedInput();

    const paste = press({ key: 'v', metaKey: true }, input);
    expect(bridge.nativeEdit).toHaveBeenCalledWith('paste');
    expect(paste.defaultPrevented).toBe(true); // As on base: the native paste ran instead.

    const escape = press({ key: 'Escape' }, input);
    expect(document.activeElement).not.toBe(input);
    expect(escape.defaultPrevented).toBe(false);

    await settle();
    expect(PM.EngineDoc.showFailure).not.toHaveBeenCalled();
    expect(pasteLayers).not.toHaveBeenCalled();
  });

  it('runs Edit-menu field commands natively but names the failure for their document fallback', async () => {
    const input = focusedInput();
    for (const command of ['contextCopy', 'contextCut', 'contextPaste', 'contextUndo', 'contextRedo', 'contextSelectAll']) {
      menuCommand(command);
    }
    expect(bridge.nativeEdit.mock.calls.map(([action]: [string]) => action))
      .toEqual(['copy', 'cut', 'paste', 'undo', 'redo', 'selectAll']);
    await settle();
    expect(PM.EngineDoc.showFailure).not.toHaveBeenCalled();

    // No field: Cmd+V would paste media or layers into the document, which cannot answer.
    input.blur();
    bridge.nativeEdit.mockClear();
    press({ key: 'v', metaKey: true }, document.body);
    menuCommand('contextUndo');
    await settle();
    expect(bridge.nativeEdit).not.toHaveBeenCalled();
    expect(bridge.pasteMedia).not.toHaveBeenCalled();
    expect(PM.EngineDoc.showFailure).toHaveBeenCalledTimes(2);
  });

  it('saves by shortcut and menu, as from the toolbar', async () => {
    PM.saveProject = vi.fn(async () => true);
    press({ key: 's', metaKey: true }, document.body);
    menuCommand('saveAs');
    expect(PM.saveProject).toHaveBeenCalledTimes(2);
    expect(PM.saveProject).toHaveBeenLastCalledWith({ saveAs: true });
    await settle();
    expect(PM.EngineDoc.showFailure).not.toHaveBeenCalled();
  });

  it('treats an extension override of a field command as a document command', async () => {
    const run = vi.fn();
    PM.Kernel.commands.register('vendor.paste', { id: 'contextPaste', label: 'Paste', run });
    press({ key: 'v', metaKey: true }, focusedInput());
    await settle();
    expect(run).not.toHaveBeenCalled();
    expect(PM.EngineDoc.showFailure).toHaveBeenCalledOnce();
  });
});
