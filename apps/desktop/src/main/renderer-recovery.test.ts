import { EventEmitter } from 'node:events';
import { afterEach, expect, it, vi } from 'vitest';
import { installRendererRecovery } from './renderer-recovery';

afterEach(() => vi.restoreAllMocks());

function harness() {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  const contents = Object.assign(new EventEmitter(), {
    isDestroyed: vi.fn(() => false), isCrashed: vi.fn(() => true), reload: vi.fn(),
  });
  const window = { webContents: contents, isDestroyed: vi.fn(() => false), close: vi.fn() };
  const prompt = vi.fn(async () => ({ response: 0, checkboxChecked: false }));
  installRendererRecovery(window as never, { showMessageBox: prompt } as never);
  return { contents, window, prompt, crash: (reason = 'oom') => contents.emit('render-process-gone', {}, { reason, exitCode: 5 }) };
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
