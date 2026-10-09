import { EventEmitter } from 'node:events';
import type { BrowserWindow } from 'electron';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { openOnboardingEditor } from './editor-startup';

class Window extends EventEmitter {
  webContents = new EventEmitter();
  destroyed = false;
  isDestroyed() { return this.destroyed; }
  destroy = vi.fn(() => { this.destroyed = true; this.emit('closed'); });
}
const native = (window: Window) => window as unknown as BrowserWindow;
afterEach(() => vi.useRealTimers());

describe('first-launch editor recovery', () => {
  it('destroys a stalled editor and releases setup after the startup deadline', async () => {
    vi.useFakeTimers();
    const window = new Window();
    const opening = openOnboardingEditor(() => native(window));
    const rejected = expect(opening).rejects.toThrow('could not finish opening');
    await vi.advanceTimersByTimeAsync(30_000);
    await rejected;
    expect(window.destroy).toHaveBeenCalledOnce();
    expect(window.listenerCount('closed')).toBe(0);
    expect(window.webContents.eventNames()).toEqual([]);
  });
  it('releases setup when the user closes the loading window', async () => {
    const window = new Window();
    const opening = openOnboardingEditor(() => native(window));
    const rejected = expect(opening).rejects.toThrow('closed before');
    window.destroy(); await rejected;
    expect(window.destroy).toHaveBeenCalledOnce();
  });
  it('reports a main-document load failure and removes the blank window', async () => {
    const window = new Window();
    const opening = openOnboardingEditor(() => native(window));
    const rejected = expect(opening).rejects.toThrow('Missing entry');
    window.webContents.emit('did-fail-load', {}, -6, 'Missing entry', 'app://powermove/', true);
    await rejected;
    expect(window.destroy).toHaveBeenCalledOnce();
  });
  it('releases setup if the renderer crashes before it is ready', async () => {
    const window = new Window();
    const opening = openOnboardingEditor(() => native(window));
    const rejected = expect(opening).rejects.toThrow('stopped before');
    window.webContents.emit('render-process-gone', {}, { reason: 'crashed' });
    await rejected;
    expect(window.destroy).toHaveBeenCalledOnce();
  });
  it('keeps a successfully opened editor and ignores a failed subframe', async () => {
    const window = new Window(); let ready!: () => void;
    const opening = openOnboardingEditor(callback => { ready = callback; return native(window); });
    window.webContents.emit('did-fail-load', {}, -6, 'Missing frame', 'app://powermove/frame', false);
    ready();
    await expect(opening).resolves.toBe(window);
    window.webContents.emit('render-process-gone', {}, { reason: 'crashed' });
    expect(window.destroy).not.toHaveBeenCalled();
  });
});
