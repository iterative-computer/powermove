// @vitest-environment happy-dom
// @ts-nocheck -- legacy PM is intentionally a dynamic registry.
/*
 * Parity table for the keydown handler that used to live in ui/shortcuts.ts.
 *
 * Every branch of the old if-chain and both switch blocks gets a row here. The
 * test fires a real KeyboardEvent at the window (which is where the kernel's
 * single listener lives) and asserts the command that ran, so the migration
 * from a hand-written handler to `kernel.keybindings` is verified against the
 * behaviour it replaced rather than against its own implementation.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import activateKeymap from '../../../../extensions/keymap-default/index';
import { installKernel } from '../../kernel/install';
import { install as installShortcuts } from '../ui/shortcuts';

let PM: any;
let ran: string[];
let activatedBindings: any[];

function makeEditor() {
  PM = {
    h: () => ({ focus() {}, select() {}, value: '' }),
    proj: { w: 1920, h: 1080, fps: 30, dur: 8, layers: [], assets: {}, work: [0, 8] },
    time: 1,
    snapF: (value: number) => value,
    store: { get: (_key: string, fallback: unknown) => fallback, set() {} },
    bus: { on: () => () => {}, emit() {} }
  };
  installKernel(PM);
  installShortcuts(PM);
  activatedBindings = [];
  /* Built-ins are activated through an owner-scoped API by the loader. This
     focused harness supplies the same ownership boundary without booting the
     complete extension system. */
  activateKeymap({
    keybindings: {
      bind: (definition: any) => {
        activatedBindings.push(definition);
        return PM.Kernel.bind('keymap-default', definition);
      }
    }
  } as any);
  ran = [];
  /* Shadow every command with a recorder. Registering over an id is exactly
     what an extension override does, so this also exercises that path. */
  for (const id of PM.Kernel.commands.ids()) {
    /* `blurField` is the fall-through binding under Escape; shadowing it would
       swallow the chord before `deselect` ever sees it. */
    if (id === 'blurField') continue;
    PM.Kernel.commands.register('test', { id, label: id, run: () => void ran.push(id) });
  }
}

function press(init: KeyboardEventInit, target: EventTarget = window): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}

beforeEach(makeEditor);
afterEach(() => {
  PM.Kernel.uninstall();
  document.body.replaceChildren();
});

/* [description, KeyboardEventInit, expected command id] */
const TABLE: Array<[string, KeyboardEventInit, string]> = [
  ['cmd+k opens the palette', { key: 'k', metaKey: true }, 'palette'],
  ['ctrl+k opens the palette', { key: 'k', ctrlKey: true }, 'palette'],
  ['cmd+shift+k opens the agent', { key: 'k', metaKey: true, shiftKey: true }, 'agent'],
  ['cmd+comma opens settings', { key: ',', metaKey: true }, 'settings'],
  ['ctrl+comma opens settings', { key: ',', ctrlKey: true }, 'settings'],
  ['cmd+z undoes', { key: 'z', metaKey: true }, 'undo'],
  ['ctrl+z undoes', { key: 'z', ctrlKey: true }, 'undo'],
  ['cmd+shift+z redoes', { key: 'z', metaKey: true, shiftKey: true }, 'redo'],
  ['cmd+y makes a solid', { key: 'y', metaKey: true }, 'newSolid'],
  ['cmd+shift+y makes a shape', { key: 'y', metaKey: true, shiftKey: true }, 'newShape'],
  ['cmd+t makes text', { key: 't', metaKey: true }, 'newText'],
  ['cmd+shift+g makes a shader layer', { key: 'g', metaKey: true, shiftKey: true }, 'newShader'],
  ['cmd+d duplicates', { key: 'd', metaKey: true }, 'duplicate'],
  ['cmd+shift+d splits', { key: 'd', metaKey: true, shiftKey: true }, 'split'],
  ['cmd+c copies layers', { key: 'c', metaKey: true }, 'copyLayers'],
  ['cmd+shift+c precomposes', { key: 'c', metaKey: true, shiftKey: true }, 'precompose'],
  ['cmd+v pastes layers', { key: 'v', metaKey: true }, 'pasteLayers'],
  ['cmd+a selects all', { key: 'a', metaKey: true }, 'selectAll'],
  ['cmd+i imports', { key: 'i', metaKey: true }, 'import'],
  ['cmd+s saves', { key: 's', metaKey: true }, 'save'],
  ['cmd+shift+s saves a project as', { key: 's', metaKey: true, shiftKey: true }, 'saveAs'],
  ['cmd+o opens', { key: 'o', metaKey: true }, 'open'],
  ['cmd+e exports', { key: 'e', metaKey: true }, 'export'],
  ['cmd+p shows projects', { key: 'p', metaKey: true }, 'projects'],
  ['cmd+n makes a project', { key: 'n', metaKey: true }, 'newProject'],
  ['F9 easy-eases', { key: 'F9' }, 'easeOut'],
  ['shift+F9 applies the power curve', { key: 'F9', shiftKey: true }, 'easePower'],
  ['cmd+F9 linearises', { key: 'F9', metaKey: true }, 'easeLinear'],
  ['cmd+shift+F9 linearises', { key: 'F9', metaKey: true, shiftKey: true }, 'easeLinear'],
  ['space toggles playback', { key: ' ' }, 'play'],
  ['Home goes to the start', { key: 'Home' }, 'gotoStart'],
  ['End goes to the end', { key: 'End' }, 'gotoEnd'],
  ['ArrowRight steps forward', { key: 'ArrowRight' }, 'nextFrame'],
  ['shift+ArrowRight jumps to the next edge', { key: 'ArrowRight', shiftKey: true }, 'nextEdge'],
  ['ArrowLeft steps back', { key: 'ArrowLeft' }, 'prevFrame'],
  ['shift+ArrowLeft jumps to the previous edge', { key: 'ArrowLeft', shiftKey: true }, 'prevEdge'],
  ['Backspace deletes', { key: 'Backspace' }, 'delete'],
  ['Delete deletes', { key: 'Delete' }, 'delete'],
  ['Escape deselects', { key: 'Escape' }, 'deselect'],
  ['v picks the selection tool', { key: 'v' }, 'toolSelect'],
  ['h picks the hand tool', { key: 'h' }, 'toolHand'],
  ['z picks the zoom tool', { key: 'z' }, 'toolZoom'],
  ['p reveals position', { key: 'p' }, 'revealPos'],
  ['s reveals scale', { key: 's' }, 'revealScale'],
  ['r reveals rotation', { key: 'r' }, 'revealRot'],
  ['t reveals opacity', { key: 't' }, 'revealOpacity'],
  ['a reveals the anchor point', { key: 'a' }, 'revealAnchor'],
  ['u reveals animated properties', { key: 'u' }, 'revealKeys'],
  ['g toggles the graph editor', { key: 'g' }, 'graph'],
  ['b sets the work area in', { key: 'b' }, 'workIn'],
  ['n sets the work area out', { key: 'n' }, 'workOut'],
  ['shift+f fits the composition', { key: 'f', shiftKey: true }, 'fitView'],
  ['j jumps to the previous edge', { key: 'j' }, 'prevEdge'],
  ['k pauses', { key: 'k' }, 'transportPause'],
  ['l plays', { key: 'l' }, 'transportPlay'],
  ['i trims in', { key: 'i' }, 'trimIn'],
  ['o trims out', { key: 'o' }, 'trimOut'],
  /* The old switch(k) sat before the `if (a || m) return` guard, so transport
     keys fired with a modifier held too. */
  ['cmd+space still toggles playback', { key: ' ', metaKey: true }, 'play'],
  ['alt+ArrowRight still steps forward', { key: 'ArrowRight', altKey: true }, 'nextFrame'],
  ['cmd+Delete still deletes', { key: 'Delete', metaKey: true }, 'delete'],
  ['alt+Escape still deselects', { key: 'Escape', altKey: true }, 'deselect'],
  ['shift+Space still toggles playback', { key: ' ', shiftKey: true }, 'play'],
  ['shift+Delete still deletes', { key: 'Delete', shiftKey: true }, 'delete'],
  ['cmd+alt+k still opens the palette', { key: 'k', metaKey: true, altKey: true }, 'palette'],
  ['cmd+ctrl+z still undoes', { key: 'z', metaKey: true, ctrlKey: true }, 'undo'],
  ['alt+F9 still easy-eases', { key: 'F9', altKey: true }, 'easeOut']
];

describe('legacy keymap parity', () => {
  it.each(TABLE)('%s', (_description, init, expected) => {
    const event = press(init);
    expect(ran).toEqual([expected]);
    expect(event.defaultPrevented).toBe(true);
  });

  it('leaves plain F unbound, as the old handler did', () => {
    const event = press({ key: 'f' });
    expect(ran).toEqual([]);
    expect(event.defaultPrevented).toBe(false);
  });

  it('ignores every binding while a text field has focus', () => {
    const input = document.createElement('input');
    document.body.append(input);
    press({ key: 'v' }, input);
    press({ key: ' ' }, input);
    press({ key: 'Backspace' }, input);
    expect(ran).toEqual([]);
  });

  it('blurs the focused field on Escape without running deselect', () => {
    const input = document.createElement('input');
    document.body.append(input);
    input.focus();
    expect(document.activeElement).toBe(input);

    const event = press({ key: 'Escape' }, input);

    expect(document.activeElement).not.toBe(input);
    expect(ran).toEqual([]);
    /* The old handler did not preventDefault for this branch. */
    expect(event.defaultPrevented).toBe(false);
  });

  it('dispatches editor bindings from select elements, matching the old field guard', () => {
    const select = document.createElement('select');
    document.body.append(select);

    const event = press({ key: 'v' }, select);

    expect(ran).toEqual(['toolSelect']);
    expect(event.defaultPrevented).toBe(true);
  });

  it('prevents default before invoking a command that throws', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    PM.Kernel.commands.register('throw-test', {
      id: 'palette',
      label: 'Throwing palette',
      run: () => {
        throw new Error('boom');
      }
    });

    const event = press({ key: 'k', metaKey: true });

    expect(event.defaultPrevented).toBe(true);
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it('registers every editor binding as a keymap-default entry at priority 100', () => {
    const bindings = PM.Kernel.listBindings();
    expect(bindings).toHaveLength(activatedBindings.length);
    const editorBindings = bindings.filter((binding: any) => binding.command !== 'blurField');
    expect(editorBindings.length).toBeGreaterThan(50);
    for (const binding of editorBindings) {
      expect(binding.ownerId).toBe('keymap-default');
      expect(binding.priority).toBe(100);
      expect(binding.inFields).toBe(['save', 'saveAs', 'settings'].includes(binding.command));
    }
    expect(bindings.find((binding: any) => binding.command === 'blurField')).toMatchObject({
      ownerId: 'keymap-default',
      priority: 50,
      inFields: true
    });
  });

  it('lets a later, lower-priority binding win over the built-in one', () => {
    PM.Kernel.commands.register('ext', { id: 'ext.thing', label: 'Thing', run: () => void ran.push('ext.thing') });
    PM.Kernel.bind('ext', { key: 'space', command: 'ext.thing', priority: 0 });

    press({ key: ' ' });

    expect(ran).toEqual(['ext.thing']);
  });
});
