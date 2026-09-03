// @vitest-environment happy-dom
// @ts-nocheck -- legacy PM is intentionally a dynamic registry.
/* End-to-end key dispatch for Powermove's AE-style editor profile. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import activateKeymap from '../../../../extensions/keymap-default/index';
import { installKernel } from '../../kernel/install';
import { install as installShortcuts } from '../ui/shortcuts';

let PM: any;
let ran: string[];
let ranArgs: unknown[][];
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
  ranArgs = [];
  /* Shadow every command with a recorder. Registering over an id is exactly
     what an extension override does, so this also exercises that path. */
  for (const id of PM.Kernel.commands.ids()) {
    /* `blurField` is the fall-through binding under Escape; shadowing it would
       swallow the chord before `deselect` ever sees it. */
    if (id === 'blurField') continue;
    PM.Kernel.commands.register('test', {
      id,
      label: id,
      run: (...args: unknown[]) => {
        ran.push(id);
        ranArgs.push(args);
      }
    });
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
  ['cmd+t picks the type tool', { key: 't', metaKey: true }, 'toolText'],
  ['cmd+shift+g makes a shader layer', { key: 'g', metaKey: true, shiftKey: true }, 'newShader'],
  ['cmd+d duplicates', { key: 'd', metaKey: true }, 'duplicate'],
  ['cmd+shift+d splits', { key: 'd', metaKey: true, shiftKey: true }, 'split'],
  ['cmd+c copies layers', { key: 'c', metaKey: true }, 'copyLayers'],
  ['cmd+shift+c precomposes', { key: 'c', metaKey: true, shiftKey: true }, 'precompose'],
  ['cmd+v pastes layers', { key: 'v', metaKey: true }, 'pasteLayers'],
  ['cmd+a selects all', { key: 'a', metaKey: true }, 'selectAll'],
  ['cmd+shift+a deselects all', { key: 'a', metaKey: true, shiftKey: true }, 'deselect'],
  ['cmd+i imports', { key: 'i', metaKey: true }, 'import'],
  ['cmd+s saves', { key: 's', metaKey: true }, 'save'],
  ['cmd+shift+s saves a project as', { key: 's', metaKey: true, shiftKey: true }, 'saveAs'],
  ['cmd+o opens', { key: 'o', metaKey: true }, 'open'],
  ['cmd+e exports', { key: 'e', metaKey: true }, 'export'],
  ['cmd+p shows projects', { key: 'p', metaKey: true }, 'projects'],
  ['cmd+n makes a project', { key: 'n', metaKey: true }, 'newProject'],
  ['F9 applies Easy Ease', { key: 'F9' }, 'easyEase'],
  ['shift+F9 applies Easy Ease In', { key: 'F9', shiftKey: true }, 'easyEaseIn'],
  ['cmd+shift+F9 applies Easy Ease Out', { key: 'F9', metaKey: true, shiftKey: true }, 'easyEaseOut'],
  ['space toggles playback', { key: ' ' }, 'play'],
  ['Home goes to the start', { key: 'Home' }, 'gotoStart'],
  ['End goes to the end', { key: 'End' }, 'gotoEnd'],
  ['Page Down steps forward', { key: 'PageDown' }, 'nextFrame'],
  ['Page Up steps back', { key: 'PageUp' }, 'prevFrame'],
  ['shift+Page Down steps ten frames', { key: 'PageDown', shiftKey: true }, 'stepFrames'],
  ['shift+Page Up steps back ten frames', { key: 'PageUp', shiftKey: true }, 'stepFrames'],
  ['ArrowRight nudges right', { key: 'ArrowRight' }, 'nudgeSelection'],
  ['shift+ArrowRight nudges right ten pixels', { key: 'ArrowRight', shiftKey: true }, 'nudgeSelection'],
  ['ArrowLeft nudges left', { key: 'ArrowLeft' }, 'nudgeSelection'],
  ['shift+ArrowLeft nudges left ten pixels', { key: 'ArrowLeft', shiftKey: true }, 'nudgeSelection'],
  ['Backspace deletes', { key: 'Backspace' }, 'delete'],
  ['Delete deletes', { key: 'Delete' }, 'delete'],
  ['Escape deselects', { key: 'Escape' }, 'deselect'],
  ['v picks the selection tool', { key: 'v' }, 'toolSelect'],
  ['h picks the hand tool', { key: 'h' }, 'toolHand'],
  ['z picks the zoom tool', { key: 'z' }, 'toolZoom'],
  ['w picks the rotation tool', { key: 'w' }, 'toolRotate'],
  ['y picks the pan behind tool', { key: 'y' }, 'toolAnchor'],
  ['q picks and cycles the shape tools', { key: 'q' }, 'toolShape'],
  ['p reveals position', { key: 'p' }, 'revealPos'],
  ['s reveals scale', { key: 's' }, 'revealScale'],
  ['r reveals rotation', { key: 'r' }, 'revealRot'],
  ['t reveals opacity', { key: 't' }, 'revealOpacity'],
  ['a reveals the anchor point', { key: 'a' }, 'revealAnchor'],
  ['u reveals animated properties', { key: 'u' }, 'revealKeys'],
  ['b sets the work area in', { key: 'b' }, 'workIn'],
  ['n sets the work area out', { key: 'n' }, 'workOut'],
  ['shift+f fits the composition', { key: 'f', shiftKey: true }, 'fitView'],
  ['j jumps to the previous visible event', { key: 'j' }, 'prevVisibleEvent'],
  ['k jumps to the next visible event', { key: 'k' }, 'nextVisibleEvent'],
  ['shift+j jumps to the previous selected event', { key: 'j', shiftKey: true }, 'prevSelectedEvent'],
  ['shift+k jumps to the next selected event', { key: 'k', shiftKey: true }, 'nextSelectedEvent'],
  ['i goes to the selected layer In point', { key: 'i' }, 'gotoLayerIn'],
  ['o goes to the selected layer Out point', { key: 'o' }, 'gotoLayerOut'],
  ['left bracket moves the layer In point', { key: '[', code: 'BracketLeft' }, 'moveLayerIn'],
  ['right bracket moves the layer Out point', { key: ']', code: 'BracketRight' }, 'moveLayerOut'],
  ['option+left bracket trims the layer In point', { key: '[', code: 'BracketLeft', altKey: true }, 'trimIn'],
  ['option+right bracket trims the layer Out point', { key: ']', code: 'BracketRight', altKey: true }, 'trimOut'],
  ['period zooms the viewer in', { key: '.', code: 'Period' }, 'zoomIn'],
  ['comma zooms the viewer out', { key: ',', code: 'Comma' }, 'zoomOut'],
  ['slash shows 100 percent', { key: '/', code: 'Slash' }, 'actualSize'],
  ['shift+slash fits the composition', { key: '?', code: 'Slash', shiftKey: true }, 'fitComposition'],
  ['shift+F3 toggles the Graph Editor', { key: 'F3', shiftKey: true }, 'graph'],
  ['cmd+shift+h toggles layer controls', { key: 'h', metaKey: true, shiftKey: true }, 'toggleLayerControls'],
  ['cmd+l locks selected layers', { key: 'l', metaKey: true }, 'lockSelectedLayers'],
  ['cmd+shift+l unlocks all layers', { key: 'l', metaKey: true, shiftKey: true }, 'unlockAllLayers'],
  ['cmd+up selects the previous layer', { key: 'ArrowUp', metaKey: true }, 'selectPreviousLayer'],
  ['cmd+down selects the next layer', { key: 'ArrowDown', metaKey: true }, 'selectNextLayer'],
  ['cmd+shift+up extends to the previous layer', { key: 'ArrowUp', metaKey: true, shiftKey: true }, 'extendSelectionPreviousLayer'],
  ['cmd+shift+down extends to the next layer', { key: 'ArrowDown', metaKey: true, shiftKey: true }, 'extendSelectionNextLayer'],
  /* Space remains global even when another modifier is held. */
  ['cmd+space still toggles playback', { key: ' ', metaKey: true }, 'play'],
  ['shift+Space still toggles playback', { key: ' ', shiftKey: true }, 'play'],
  ['cmd+alt+k still opens the palette', { key: 'k', metaKey: true, altKey: true }, 'palette'],
  ['cmd+ctrl+z still undoes', { key: 'z', metaKey: true, ctrlKey: true }, 'undo']
];

describe('AE-style keymap dispatch', () => {
  it.each(TABLE)('%s', (_description, init, expected) => {
    const event = press(init);
    expect(ran).toEqual([expected]);
    expect(event.defaultPrevented).toBe(true);
  });

  const NUDGE_TABLE: Array<[string, KeyboardEventInit, [number, number]]> = [
    ['ArrowLeft nudges one pixel left', { key: 'ArrowLeft' }, [-1, 0]],
    ['ArrowRight nudges one pixel right', { key: 'ArrowRight' }, [1, 0]],
    ['ArrowUp nudges one pixel up', { key: 'ArrowUp' }, [0, -1]],
    ['ArrowDown nudges one pixel down', { key: 'ArrowDown' }, [0, 1]],
    ['Shift+ArrowLeft nudges ten pixels left', { key: 'ArrowLeft', shiftKey: true }, [-10, 0]],
    ['Shift+ArrowRight nudges ten pixels right', { key: 'ArrowRight', shiftKey: true }, [10, 0]],
    ['Shift+ArrowUp nudges ten pixels up', { key: 'ArrowUp', shiftKey: true }, [0, -10]],
    ['Shift+ArrowDown nudges ten pixels down', { key: 'ArrowDown', shiftKey: true }, [0, 10]]
  ];

  it.each(NUDGE_TABLE)('%s', (_description, init, delta) => {
    const event = press(init);
    expect(ran).toEqual(['nudgeSelection']);
    expect(ranArgs).toEqual([delta]);
    expect(event.defaultPrevented).toBe(true);
  });

  const KEYFRAME_NUDGE_TABLE: Array<[string, KeyboardEventInit, [number]]> = [
    ['Option+Left moves keys one frame earlier', { key: 'ArrowLeft', altKey: true }, [-1]],
    ['Option+Right moves keys one frame later', { key: 'ArrowRight', altKey: true }, [1]],
    ['Option+Shift+Left moves keys ten frames earlier', { key: 'ArrowLeft', altKey: true, shiftKey: true }, [-10]],
    ['Option+Shift+Right moves keys ten frames later', { key: 'ArrowRight', altKey: true, shiftKey: true }, [10]],
  ];

  it.each(KEYFRAME_NUDGE_TABLE)('%s', (_description, init, delta) => {
    const event = press(init);
    expect(ran).toEqual(['nudgeKeyframes']);
    expect(ranArgs).toEqual([delta]);
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
