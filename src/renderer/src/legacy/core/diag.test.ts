import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './diag';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('legacy diag install', () => {
  it('wraps async entry points while preserving ids and reporting failures', () => {
    const listeners: Record<string, any> = {};
    const messages: string[] = [];
    let timerCallback: any;
    let frameCallback: any;
    const windowStub: any = {
      addEventListener: (name: any, fn: any) => { listeners[name] = fn; },
      setTimeout: (fn: any) => { timerCallback = fn; return 41; },
      setInterval: () => 42,
      requestAnimationFrame: (fn: any) => { frameCallback = fn; return 43; },
      webkit: { messageHandlers: { pmLog: { postMessage: (message: any) => messages.push(message) } } },
    };
    vi.stubGlobal('window', windowStub);

    install({} as PMRegistry);

    expect(Object.keys(listeners)).toEqual(['error', 'unhandledrejection']);
    expect(windowStub.setTimeout(() => { throw new Error('timer boom'); }, 5)).toBe(41);
    expect(() => timerCallback()).toThrow('timer boom');
    expect(messages.at(-1)).toContain('[js] setTimeout timer boom');
    expect(windowStub.requestAnimationFrame(() => { throw new Error('frame boom'); })).toBe(43);
    expect(() => frameCallback(16)).toThrow('frame boom');
  });
});
