// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Disposable } from './api';
import { createKernel, type Kernel } from './registries';

const listeners: Disposable[] = [];

afterEach(() => {
  for (const listener of listeners.splice(0)) listener.dispose();
  document.body.replaceChildren();
});

function install(kernel: Kernel): { runs: Array<[string, unknown[]]> } {
  const runs: Array<[string, unknown[]]> = [];
  listeners.push(
    kernel.installKeyListener((command, args) => {
      runs.push([command, args]);
      return undefined;
    })
  );
  return { runs };
}

const press = (init: KeyboardEventInit & { key: string }, target: EventTarget = window): boolean => {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event.defaultPrevented;
};

describe('installKeyListener', () => {
  it('dispatches the highest-priority binding first and prevents default', () => {
    const kernel = createKernel();
    const { runs } = install(kernel);
    kernel.bind('built-in', { key: 'cmd+k', command: 'palette', priority: 100 });
    kernel.bind('ext', { key: 'cmd+k', command: 'ext-palette', priority: 0 });

    expect(press({ key: 'k', metaKey: true })).toBe(true);
    expect(runs.map(([command]) => command)).toEqual(['ext-palette']);
  });

  it('prefers the most specific loose binding at the same priority', () => {
    const kernel = createKernel();
    const { runs } = install(kernel);
    kernel.bind('built-in', { key: 'cmd+k', command: 'palette', priority: 100, looseModifiers: true });
    kernel.bind('built-in', { key: 'cmd+shift+k', command: 'agent', priority: 100, looseModifiers: true });

    expect(press({ key: 'k', metaKey: true, altKey: true, shiftKey: true })).toBe(true);
    expect(runs.map(([command]) => command)).toEqual(['agent']);
  });

  it('falls through to the next binding when a handler returns false', () => {
    const kernel = createKernel();
    const runs: string[] = [];
    listeners.push(
      kernel.installKeyListener((command) => {
        runs.push(command);
        return command === 'first' ? false : undefined;
      })
    );
    kernel.bind('ext', { key: 'cmd+k', command: 'first', priority: 0 });
    kernel.bind('built-in', { key: 'cmd+k', command: 'second', priority: 10 });

    expect(press({ key: 'k', metaKey: true })).toBe(true);
    expect(runs).toEqual(['first', 'second']);
  });

  it('suppresses ordinary bindings on repeated keydowns without preventing default', () => {
    const kernel = createKernel();
    const { runs } = install(kernel);
    kernel.bind('ext', { key: 'cmd+k', command: 'palette' });

    expect(press({ key: 'k', metaKey: true, repeat: true })).toBe(false);
    expect(runs).toEqual([]);
  });

  it('dispatches repeat-enabled bindings and prevents default when handled', () => {
    const kernel = createKernel();
    const { runs } = install(kernel);
    kernel.bind('ext', { key: 'cmd+k', command: 'palette', repeat: true });

    expect(press({ key: 'k', metaKey: true, repeat: true })).toBe(true);
    expect(runs).toEqual([['palette', []]]);
  });

  it('does not fall through from a repeat-enabled decline to a suppressed binding', () => {
    const kernel = createKernel();
    const runs: string[] = [];
    listeners.push(
      kernel.installKeyListener((command) => {
        runs.push(command);
        return false;
      })
    );
    kernel.bind('ext', { key: 'cmd+k', command: 'repeat-first', repeat: true, priority: 0 });
    kernel.bind('built-in', { key: 'cmd+k', command: 'ordinary-second', priority: 10 });

    expect(press({ key: 'k', metaKey: true, repeat: true })).toBe(false);
    expect(runs).toEqual(['repeat-first']);
  });

  it('falls through between repeat-enabled bindings and prevents once one handles', () => {
    const kernel = createKernel();
    const runs: string[] = [];
    listeners.push(
      kernel.installKeyListener((command) => {
        runs.push(command);
        return command === 'repeat-first' ? false : undefined;
      })
    );
    kernel.bind('ext', { key: 'cmd+k', command: 'repeat-first', repeat: true, priority: 0 });
    kernel.bind('built-in', { key: 'cmd+k', command: 'repeat-second', repeat: true, priority: 10 });

    expect(press({ key: 'k', metaKey: true, repeat: true })).toBe(true);
    expect(runs).toEqual(['repeat-first', 'repeat-second']);
  });

  it('prevents before invoking even when every binding declines', () => {
    const kernel = createKernel();
    listeners.push(kernel.installKeyListener(() => false));
    kernel.bind('ext', { key: 'cmd+k', command: 'nope' });
    expect(press({ key: 'k', metaKey: true })).toBe(true);
  });

  it('skips bindings in text fields unless inFields is set', () => {
    const kernel = createKernel();
    const { runs } = install(kernel);
    kernel.bind('ext', { key: 'space', command: 'play' });
    kernel.bind('ext', { key: 'escape', command: 'blur', inFields: true });

    const input = document.createElement('input');
    document.body.append(input);

    expect(press({ key: ' ' }, input)).toBe(false);
    expect(runs).toEqual([]);

    expect(press({ key: 'Escape' }, input)).toBe(true);
    expect(runs).toEqual([['blur', []]]);

    expect(press({ key: ' ' })).toBe(true);
    expect(runs.map(([command]) => command)).toEqual(['blur', 'play']);
  });

  it('passes binding args and ignores bare modifier keydowns', () => {
    const kernel = createKernel();
    const { runs } = install(kernel);
    kernel.bind('ext', { key: 'shift+f9', command: 'ease', args: ['power'] });

    expect(press({ key: 'Shift', shiftKey: true })).toBe(false);
    expect(press({ key: 'F9', shiftKey: true })).toBe(true);
    expect(runs).toEqual([['ease', ['power']]]);
  });

  it('contains a throwing command and stops dispatching that chord', () => {
    const kernel = createKernel();
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    listeners.push(
      kernel.installKeyListener((command) => {
        if (command === 'bad') throw new Error('boom');
        return undefined;
      })
    );
    kernel.bind('ext', { key: 'cmd+b', command: 'bad', priority: 0 });
    kernel.bind('ext', { key: 'cmd+b', command: 'good', priority: 1 });

    let prevented = false;
    expect(() => {
      prevented = press({ key: 'b', metaKey: true });
    }).not.toThrow();
    expect(prevented).toBe(true);
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it('installs exactly one listener and removes it on dispose', () => {
    const kernel = createKernel();
    const add = vi.spyOn(window, 'addEventListener');
    const remove = vi.spyOn(window, 'removeEventListener');
    const { runs } = install(kernel);
    kernel.bind('ext', { key: 'cmd+k', command: 'palette' });

    expect(add.mock.calls.filter(([type]) => type === 'keydown')).toHaveLength(1);
    press({ key: 'k', metaKey: true });
    expect(runs).toHaveLength(1);

    listeners.splice(0).forEach((listener) => listener.dispose());
    expect(remove.mock.calls.filter(([type]) => type === 'keydown')).toHaveLength(1);
    press({ key: 'k', metaKey: true });
    expect(runs).toHaveLength(1);

    add.mockRestore();
    remove.mockRestore();
  });
});
