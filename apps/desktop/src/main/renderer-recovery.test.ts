import { EventEmitter } from 'node:events';
import { afterEach, expect, it, vi } from 'vitest';
import { installRendererRecovery, isRendererUnavailable, recoverRenderer } from './renderer-recovery';

afterEach(() => vi.restoreAllMocks());

function harness() {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  const contents = Object.assign(new EventEmitter(), {
    isDestroyed: vi.fn(() => false), isCrashed: vi.fn(() => true), reload: vi.fn(),
  });
  const window = { webContents: contents, isDestroyed: vi.fn(() => false), close: vi.fn() };
  const prompt = vi.fn(async () => ({ response: 0, checkboxChecked: false }));
  const onUnavailable = vi.fn();
  installRendererRecovery(window as never, { showMessageBox: prompt } as never, onUnavailable);
  return { contents, window, prompt, onUnavailable, crash: (reason = 'oom') => contents.emit('render-process-gone', {}, { reason, exitCode: 5 }) };
}

const settle = () => new Promise<void>(resolve => setImmediate(resolve));

it('defers the native prompt until teardown completes and never prompts for clean exits', async () => {
  const h = harness();
  h.crash('clean-exit'); await settle(); expect(h.prompt).not.toHaveBeenCalled();
  h.crash(); expect(h.prompt).not.toHaveBeenCalled();
  await settle();
  expect(h.prompt.mock.calls[0]).toEqual([h.window, expect.objectContaining({ detail: expect.stringContaining('ran out of memory') })]);
  expect(h.contents.reload).toHaveBeenCalledOnce();
});

it('keeps one pending prompt and does not reload a closed or already recovered window', async () => {
  const h = harness();
  let answer!: (result: any) => void;
  h.prompt.mockImplementation(() => new Promise(resolve => { answer = resolve; }));
  h.crash(); h.crash(); await settle();
  expect(h.prompt).toHaveBeenCalledOnce();
  h.contents.isCrashed.mockReturnValue(false);
  h.contents.emit('did-finish-load');
  answer({ response: 0 }); await settle();
  expect(h.contents.reload).not.toHaveBeenCalled();
  h.crash(); h.window.isDestroyed.mockReturnValue(true); await settle();
  expect(h.prompt).toHaveBeenCalledOnce();
});

it('contains prompt errors and permits a later recovery attempt', async () => {
  const h = harness();
  h.prompt.mockRejectedValueOnce(new Error('Prompt failed'));
  h.crash(); await settle(); await settle();
  expect(h.contents.reload).not.toHaveBeenCalled();
  h.crash(); await settle();
  expect(h.contents.reload).toHaveBeenCalledOnce();
});

it('marks even clean renderer exits unavailable and clears the mark only after reload completes', async () => {
  const h = harness();
  h.contents.isCrashed.mockReturnValue(false);
  expect(isRendererUnavailable(h.contents as never)).toBe(false);
  h.crash('clean-exit');
  expect(h.onUnavailable).toHaveBeenCalledOnce();
  expect(isRendererUnavailable(h.contents as never)).toBe(true);
  expect(recoverRenderer(h.window as never)).toBe(true);
  expect(recoverRenderer(h.window as never)).toBe(true);
  expect(h.contents.reload).not.toHaveBeenCalled();
  await settle();
  expect(h.contents.reload).toHaveBeenCalledOnce();
  h.contents.emit('did-finish-load');
  expect(isRendererUnavailable(h.contents as never)).toBe(false);
  expect(recoverRenderer(h.window as never)).toBe(false);
  expect(h.prompt).not.toHaveBeenCalled();
});

it('does not reload if a crashed window is closed before activation recovery runs', async () => {
  const h = harness();
  expect(recoverRenderer(h.window as never)).toBe(true);
  h.window.isDestroyed.mockReturnValue(true);
  await settle();
  expect(h.contents.reload).not.toHaveBeenCalled();
});
