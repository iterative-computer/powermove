// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { install } from './exporter';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';

vi.mock('./export-preview', () => ({ animateExportPreview: () => () => {} }));
afterEach(() => { resetBridgeForTests(); document.body.replaceChildren(); });

function fixture(queue = false) {
  let finish!: () => void;
  const flushBindings = vi.fn(async (_project, progress) => {
    progress(0, 2);
    progress(.5, 2);
    await new Promise<void>(resolve => { finish = resolve; });
    progress(2, 2);
  });
  const submit = vi.fn(async () => ({ status: 'cancelled' }));
  installBridgeForTests({ nativeEngine: {
    enabledSync: () => true,
    status: async () => ({ enabled: true, processes: { job: { state: 'ready' } } }),
    onExportStatus: () => () => {}, exportVideo: submit, queueAdd: submit,
  } } as any);
  let cancel!: () => void;
  const PM: any = {
    h(selector: string, ...args: any[]) {
      const [tag, ...classes] = selector.split('.');
      const node = document.createElement(tag!); node.className = classes.join(' ');
      for (const arg of args) {
        if (arg instanceof Node || typeof arg === 'string') node.append(arg);
        else if (arg) for (const [key, value] of Object.entries(arg)) {
          if (key === 'style') Object.assign(node.style, value);
          else node.setAttribute(key, String(value));
        }
      }
      return node;
    },
    proj: { id: 'P1', name: 'Preparation', w: 64, h: 64, fps: 30, dur: 1, work: [0, 1], layers: [] },
    store: { get: () => ({}), set() {} },
    modal(options: any) {
      document.body.append(options.body);
      cancel = options.actions[0].run;
      return { el: options.body, close() { options.body.remove(); options.onClose?.(); } };
    },
    HostMedia: { flushBindings }, toast: vi.fn(), round: (n: number) => n,
  };
  install(PM);
  const run = () => queue ? PM.Export.addToQueue({ format: 'still' }) : PM.Export.run({ format: 'still' });
  return { PM, flushBindings, submit, run, finish: () => finish(), cancel: () => cancel() };
}

it.each([false, true])('shows media preparation and progress before submitting (queue=%s)', async queue => {
  const f = fixture(queue);
  const run = f.run();
  await vi.waitFor(() => expect(f.flushBindings).toHaveBeenCalled());
  expect(document.body.textContent).toContain('Preparing media…');
  expect(document.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('25');
  expect(f.submit).not.toHaveBeenCalled();
  f.finish(); await run;
  expect(f.submit).toHaveBeenCalledOnce();
  expect(document.querySelector('.export-progress')).toBeNull();
  expect(f.PM.Export.busy).toBe(false);
});

it.each([false, true])('cancels before submission and closes preparation (queue=%s)', async queue => {
  const f = fixture(queue);
  const run = f.run();
  await vi.waitFor(() => expect(f.flushBindings).toHaveBeenCalled());
  f.cancel(); f.finish(); await run;
  expect(f.submit).not.toHaveBeenCalled();
  expect(f.PM.Export.busy).toBe(false);
  expect(document.querySelector('.export-progress')).toBeNull();
});
