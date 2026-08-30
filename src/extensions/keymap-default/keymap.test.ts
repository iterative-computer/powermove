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
    expect(captured).toHaveLength(168);
    expect(captured.filter(({ command }) => command !== 'blurField')).toHaveLength(167);
    expect(captured.find(({ command }) => command === 'blurField')).toEqual({
      key: 'escape',
      command: 'blurField',
      inFields: true,
      priority: 50
    });
    expect(captured.filter(({ command }) => command !== 'blurField').every(({ priority }) => priority === 100)).toBe(true);
  });

  it('marks only repeat-safe continuous actions as repeatable', () => {
    const repeatable = KEYMAP_DEFAULT.filter(({ repeat }) => repeat);
    const repeatableCommands = new Set(repeatable.map(({ command }) => command));

    expect(repeatableCommands).toEqual(new Set(['nextFrame', 'prevFrame', 'nudgeSelection']));
    expect(repeatable).toHaveLength(16);
    expect(
      KEYMAP_DEFAULT.filter(({ command }) => !['nextFrame', 'prevFrame', 'nudgeSelection'].includes(command)).every(({ repeat }) => repeat !== true)
    ).toBe(true);
    expect(KEYMAP_DEFAULT.filter(({ command }) => ['play', 'delete', 'nextEdge', 'prevEdge'].includes(command)).every(({ repeat }) => repeat !== true)).toBe(true);
  });

  it('registers strict editor-style primary shortcuts and nudge deltas', () => {
    const strictKeys = [
      'cmd+b', 'ctrl+b',
      'cmd+x', 'ctrl+x',
      'cmd+shift+h', 'ctrl+shift+h',
      'cmd+=', 'ctrl+=',
      'cmd+shift++', 'ctrl+shift++',
      'cmd+-', 'ctrl+-',
      'cmd+0', 'ctrl+0',
      'cmd+1', 'ctrl+1',
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
      'cmd+shift+h:toggleVisibility', 'ctrl+shift+h:toggleVisibility',
      'cmd+=:zoomIn', 'ctrl+=:zoomIn',
      'cmd+shift++:zoomIn', 'ctrl+shift++:zoomIn',
      'cmd+-:zoomOut', 'ctrl+-:zoomOut',
      'cmd+0:fitComposition', 'ctrl+0:fitComposition',
      'cmd+1:actualSize', 'ctrl+1:actualSize',
      'cmd+]:bringForward', 'ctrl+]:bringForward',
      'cmd+[:sendBackward', 'ctrl+[:sendBackward',
      'cmd+shift+]:bringToFront', 'ctrl+shift+]:bringToFront',
      'cmd+shift+[:sendToBack', 'ctrl+shift+[:sendToBack'
    ]);

    expect(KEYMAP_DEFAULT.filter(({ command }) => command === 'nudgeSelection')).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: 'alt+left', args: [-1, 0], repeat: true }),
        expect.objectContaining({ key: 'alt+right', args: [1, 0], repeat: true }),
        expect.objectContaining({ key: 'alt+up', args: [0, -1], repeat: true }),
        expect.objectContaining({ key: 'alt+down', args: [0, 1], repeat: true }),
        expect.objectContaining({ key: 'alt+shift+left', args: [-10, 0], repeat: true }),
        expect.objectContaining({ key: 'alt+shift+right', args: [10, 0], repeat: true }),
        expect.objectContaining({ key: 'alt+shift+up', args: [0, -10], repeat: true }),
        expect.objectContaining({ key: 'alt+shift+down', args: [0, 10], repeat: true })
      ])
    );
    expect(KEYMAP_DEFAULT.filter(({ command }) => command === 'nudgeSelection').every(({ looseModifiers }) => looseModifiers !== true)).toBe(true);
    expect(KEYMAP_DEFAULT.find(({ key, command }) => key === 'cmd+shift+n' && command === 'newProject')).toBeUndefined();
    expect(KEYMAP_DEFAULT.filter(({ key, command }) => key === 'cmd+shift+e' && command === 'export')).toHaveLength(1);
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
