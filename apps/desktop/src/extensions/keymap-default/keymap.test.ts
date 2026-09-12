import { describe, expect, it, vi } from 'vitest';

import type { KeybindingDefinition, PowermoveAPI } from 'powermove';

import activate from './index';
import { KEYMAP_DEFAULT } from './keymap';
import { chordOfEvent } from '../../renderer/src/kernel/keychord';
import { createKernel } from '../../renderer/src/kernel/registries';

describe('keymap-default', () => {
  it('registers the complete built-in keymap with its intended priorities', () => {
    const captured: KeybindingDefinition[] = [];
    const bind = vi.fn((definition: KeybindingDefinition) => {
      captured.push(definition);
      return { dispose() {} };
    });

    activate({ keybindings: { bind } } as unknown as PowermoveAPI);

    expect(captured).toEqual(KEYMAP_DEFAULT);
    expect(captured).toHaveLength(188);
    expect(captured.filter(({ command }) => command !== 'blurField')).toHaveLength(187);
    expect(captured.find(({ command }) => command === 'blurField')).toEqual({
      key: 'escape',
      command: 'blurField',
      inFields: true,
      priority: 50
    });
    expect(captured.filter(({ command }) => command !== 'blurField').every(({ priority }) => priority === 100)).toBe(true);
  });

  it('matches the After Effects tool shortcuts', () => {
    expect(KEYMAP_DEFAULT).toEqual(expect.arrayContaining([
      expect.objectContaining({ key: 'v', command: 'toolSelect' }),
      expect.objectContaining({ key: 'h', command: 'toolHand' }),
      expect.objectContaining({ key: 'z', command: 'toolZoom' }),
      expect.objectContaining({ key: 'w', command: 'toolRotate' }),
      expect.objectContaining({ key: 'y', command: 'toolAnchor' }),
      expect.objectContaining({ key: 'q', command: 'toolShape' }),
      expect.objectContaining({ key: 'cmd+t', command: 'toolText' }),
      expect.objectContaining({ key: 'ctrl+t', command: 'toolText' })
    ]));
    expect(KEYMAP_DEFAULT.find(({ key, command }) => key === 'cmd+t' && command === 'newText')).toBeUndefined();
  });

  it('marks only repeat-safe continuous actions as repeatable', () => {
    const repeatable = KEYMAP_DEFAULT.filter(({ repeat }) => repeat);
    const repeatableCommands = new Set(repeatable.map(({ command }) => command));

    expect(repeatableCommands).toEqual(new Set(['nextFrame', 'prevFrame', 'stepFrames', 'nudgeSelection', 'nudgeKeyframes']));
    expect(repeatable).toHaveLength(22);
    expect(
      KEYMAP_DEFAULT.filter(({ command }) => !['nextFrame', 'prevFrame', 'stepFrames', 'nudgeSelection', 'nudgeKeyframes'].includes(command)).every(({ repeat }) => repeat !== true)
    ).toBe(true);
    expect(KEYMAP_DEFAULT.filter(({ command }) => ['play', 'delete', 'nextEdge', 'prevEdge'].includes(command)).every(({ repeat }) => repeat !== true)).toBe(true);
  });

  it('routes primary paste through the native-aware context command', () => {
    expect(KEYMAP_DEFAULT).toEqual(expect.arrayContaining([
      expect.objectContaining({ key: 'cmd+v', command: 'contextPaste', inFields: true }),
      expect.objectContaining({ key: 'ctrl+v', command: 'contextPaste', inFields: true })
    ]));
    expect(KEYMAP_DEFAULT.find(({ key, command }) => key === 'cmd+v' && command === 'pasteLayers')).toBeUndefined();
  });

  it('registers strict editor-style primary shortcuts and nudge deltas', () => {
    const strictKeys = [
      'cmd+b', 'ctrl+b',
      'cmd+x', 'ctrl+x',
      'cmd+shift+h', 'ctrl+shift+h',
      'cmd+l', 'ctrl+l',
      'cmd+shift+l', 'ctrl+shift+l',
      'cmd+up', 'ctrl+up',
      'cmd+down', 'ctrl+down',
      'cmd+shift+up', 'ctrl+shift+up',
      'cmd+shift+down', 'ctrl+shift+down',
      'cmd+=', 'ctrl+=',
      'cmd+shift++', 'ctrl+shift++',
      'cmd+-', 'ctrl+-',
      'cmd+0', 'ctrl+0',
      'cmd+1', 'ctrl+1',
      'cmd+alt+home', 'ctrl+alt+home',
      'cmd+]', 'ctrl+]',
      'cmd+[', 'ctrl+[',
      'cmd+shift+]', 'ctrl+shift+]',
      'cmd+shift+[', 'ctrl+shift+['
    ];
    const strict = KEYMAP_DEFAULT.filter(({ key }) => strictKeys.includes(key));

    expect(strict).toHaveLength(strictKeys.length);
    expect(strict.every(({ looseModifiers }) => looseModifiers !== true)).toBe(true);
    expect(strict.map(({ key, command }) => `${key}:${command}`)).toEqual([
      'cmd+b:split', 'ctrl+b:split',
      'cmd+x:cutLayers', 'ctrl+x:cutLayers',
      'cmd+shift+h:toggleLayerControls', 'ctrl+shift+h:toggleLayerControls',
      'cmd+l:lockSelectedLayers', 'ctrl+l:lockSelectedLayers',
      'cmd+shift+l:unlockAllLayers', 'ctrl+shift+l:unlockAllLayers',
      'cmd+up:selectPreviousLayer', 'ctrl+up:selectPreviousLayer',
      'cmd+down:selectNextLayer', 'ctrl+down:selectNextLayer',
      'cmd+shift+up:extendSelectionPreviousLayer', 'ctrl+shift+up:extendSelectionPreviousLayer',
      'cmd+shift+down:extendSelectionNextLayer', 'ctrl+shift+down:extendSelectionNextLayer',
      'cmd+=:zoomIn', 'ctrl+=:zoomIn',
      'cmd+shift++:zoomIn', 'ctrl+shift++:zoomIn',
      'cmd+-:zoomOut', 'ctrl+-:zoomOut',
      'cmd+0:fitComposition', 'ctrl+0:fitComposition',
      'cmd+1:actualSize', 'ctrl+1:actualSize',
      'cmd+alt+home:centerAnchor', 'ctrl+alt+home:centerAnchor',
      'cmd+]:bringForward', 'ctrl+]:bringForward',
      'cmd+[:sendBackward', 'ctrl+[:sendBackward',
      'cmd+shift+]:bringToFront', 'ctrl+shift+]:bringToFront',
      'cmd+shift+[:sendToBack', 'ctrl+shift+[:sendToBack'
    ]);

    expect(KEYMAP_DEFAULT.filter(({ command }) => command === 'nudgeSelection')).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: 'left', args: [-1, 0], repeat: true }),
        expect.objectContaining({ key: 'right', args: [1, 0], repeat: true }),
        expect.objectContaining({ key: 'up', args: [0, -1], repeat: true }),
        expect.objectContaining({ key: 'down', args: [0, 1], repeat: true }),
        expect.objectContaining({ key: 'shift+left', args: [-10, 0], repeat: true }),
        expect.objectContaining({ key: 'shift+right', args: [10, 0], repeat: true }),
        expect.objectContaining({ key: 'shift+up', args: [0, -10], repeat: true }),
        expect.objectContaining({ key: 'shift+down', args: [0, 10], repeat: true })
      ])
    );
    expect(KEYMAP_DEFAULT.filter(({ command }) => command === 'nudgeKeyframes')).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: 'alt+left', args: [-1], repeat: true }),
        expect.objectContaining({ key: 'alt+right', args: [1], repeat: true }),
        expect.objectContaining({ key: 'alt+shift+left', args: [-10], repeat: true }),
        expect.objectContaining({ key: 'alt+shift+right', args: [10], repeat: true })
      ])
    );
    expect(KEYMAP_DEFAULT.filter(({ command }) => command === 'nudgeSelection').every(({ looseModifiers }) => looseModifiers !== true)).toBe(true);
    expect(KEYMAP_DEFAULT.find(({ key, command }) => key === 'cmd+shift+n' && command === 'newProject')).toBeUndefined();
    expect(KEYMAP_DEFAULT.filter(({ key, command }) => key === 'cmd+shift+e' && command === 'export')).toHaveLength(1);
  });

  it('uses After Effects timeline, layer timing, easing, and viewer shortcuts', () => {
    const expected = [
      ['pageup', 'prevFrame'], ['pagedown', 'nextFrame'],
      ['shift+pageup', 'stepFrames'], ['shift+pagedown', 'stepFrames'],
      ['shift+home', 'gotoWorkIn'], ['shift+end', 'gotoWorkOut'],
      ['j', 'prevVisibleEvent'], ['k', 'nextVisibleEvent'],
      ['shift+j', 'prevKeyframe'], ['shift+k', 'nextKeyframe'],
      ['i', 'gotoLayerIn'], ['o', 'gotoLayerOut'],
      ['[', 'moveLayerIn'], [']', 'moveLayerOut'],
      ['alt+[', 'trimIn'], ['alt+]', 'trimOut'],
      ['f9', 'easyEase'], ['shift+f9', 'easyEaseIn'],
      ['cmd+shift+f9', 'easyEaseOut'],
      ['.', 'zoomIn'], [',', 'zoomOut'], ['/', 'actualSize'],
      ['shift+/', 'fitComposition'], ['shift+f3', 'graph'],
      ['cmd+shift+a', 'deselect'], ['cmd+shift+h', 'toggleLayerControls'],
    ];
    for (const [key, command] of expected) {
      expect(KEYMAP_DEFAULT).toContainEqual(expect.objectContaining({ key, command }));
    }
    expect(KEYMAP_DEFAULT).not.toContainEqual(expect.objectContaining({ key: 'i', command: 'trimIn' }));
    expect(KEYMAP_DEFAULT).not.toContainEqual(expect.objectContaining({ key: 'k', command: 'transportPause' }));
    expect(KEYMAP_DEFAULT).not.toContainEqual(expect.objectContaining({ key: 'g', command: 'graph' }));
  });

  it('uses the browser plus key chord and does not alias Cmd+Shift+N to newProject', () => {
    expect(chordOfEvent({
      key: '+',
      code: 'Equal',
      metaKey: true,
      ctrlKey: false,
      altKey: false,
      shiftKey: true
    } as KeyboardEvent)).toBe('cmd+shift++');
    expect(KEYMAP_DEFAULT).toEqual(expect.arrayContaining([
      expect.objectContaining({ key: 'cmd+shift++', command: 'zoomIn' }),
      expect.objectContaining({ key: 'ctrl+shift++', command: 'zoomIn' })
    ]));
    expect(KEYMAP_DEFAULT.filter(({ key, command }) => key.endsWith('+shift+n') && command === 'newProject')).toEqual([]);
    expect(chordOfEvent({
      key: '?', code: 'Slash', metaKey: false, ctrlKey: false, altKey: false, shiftKey: true
    } as KeyboardEvent)).toBe('shift+/');
  });

  it('matches shifted physical bracket events to strict layer-order commands', () => {
    const kernel = createKernel();
    for (const definition of KEYMAP_DEFAULT) kernel.bind('keymap-default', definition);

    const left = chordOfEvent({
      key: '{',
      code: 'BracketLeft',
      metaKey: true,
      ctrlKey: false,
      altKey: false,
      shiftKey: true
    } as KeyboardEvent);
    const right = chordOfEvent({
      key: '}',
      code: 'BracketRight',
      metaKey: true,
      ctrlKey: false,
      altKey: false,
      shiftKey: true
    } as KeyboardEvent);

    expect(left).toBe('cmd+shift+[');
    expect(right).toBe('cmd+shift+]');
    expect(kernel.bindingsFor(left ?? '').find(({ command }) => command === 'sendToBack')).toMatchObject({
      chord: 'cmd+shift+['
    });
    expect(kernel.bindingsFor(right ?? '').find(({ command }) => command === 'bringToFront')).toMatchObject({
      chord: 'cmd+shift+]'
    });

    const target = new EventTarget();
    const runs: Array<[string, unknown[]]> = [];
    const listener = kernel.installKeyListener((command, args) => {
      runs.push([command, args]);
    }, target);
    const makeEvent = (key: string, code: string): Event => {
      const event = new Event('keydown', { cancelable: true });
      Object.defineProperties(event, {
        key: { value: key },
        code: { value: code },
        metaKey: { value: true },
        ctrlKey: { value: false },
        altKey: { value: false },
        shiftKey: { value: true },
        repeat: { value: false }
      });
      return event;
    };
    const leftEvent = makeEvent('{', 'BracketLeft');
    const rightEvent = makeEvent('}', 'BracketRight');
    const shiftedNewProjectEvent = makeEvent('n', 'KeyN');
    target.dispatchEvent(leftEvent);
    target.dispatchEvent(rightEvent);
    target.dispatchEvent(shiftedNewProjectEvent);
    listener.dispose();

    expect(runs).toEqual([
      ['sendToBack', []],
      ['bringToFront', []]
    ]);
    expect(leftEvent.defaultPrevented).toBe(true);
    expect(rightEvent.defaultPrevented).toBe(true);
    expect(shiftedNewProjectEvent.defaultPrevented).toBe(false);
  });
});
