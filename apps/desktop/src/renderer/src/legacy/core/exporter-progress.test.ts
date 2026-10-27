// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { install } from './exporter';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';

/* With Use New Engine on, an export asks for its destination first, then the
   sheet shows at once and stays until the job ends: the project check, media
   and the job engine's setup take seconds on a large project and read
   "Preparing…". A notice then reports the outcome (Show in Finder, or why it
   failed). */

vi.mock('./export-preview', () => ({ animateExportPreview: () => () => {} }));
afterEach(() => { resetBridgeForTests(); document.body.replaceChildren(); });

function fixture() {
  let checked!: () => void;
  let chose!: (destination: unknown) => void;
  let answer!: (start: unknown) => void;
  let listener: ((status: unknown) => void) | null = null;
  const exportDestination = vi.fn(() => new Promise(resolve => { chose = resolve; }));
  const exportVideo = vi.fn(() => new Promise(resolve => { answer = resolve; }));
  const beforeExport = vi.fn(() => new Promise<void>(resolve => { checked = resolve; }));
  const cancelExport = vi.fn(async () => undefined);
  const revealExport = vi.fn(async () => true);
  installBridgeForTests({
    revealExport,
    nativeEngine: {
      enabledSync: () => true,
      exportSheet: () => {},
      status: async () => ({ enabled: true, processes: { job: { state: 'ready' } } }),
      onExportStatus: (fn: (status: unknown) => void) => { listener = fn; return () => { listener = null; }; },
      exportDestination, exportVideo, cancelExport,
    }
  } as any);
  const toasts: Array<[string, any]> = [];
  let cancel!: () => unknown;
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
    proj: { id: 'P1', name: 'Film', w: 64, h: 64, fps: 30, dur: 1, work: [0, 1], layers: [] },
    store: { get: () => ({}), set() {} },
    modal(options: any) {
      const el = document.createElement('div');
      el.className = 'modal';
      el.append(options.body);
      document.body.append(el);
      cancel = options.actions[0].run;
      return { el, close() { if (el.isConnected) { el.remove(); options.onClose?.(); } } };
    },
    EngineDoc: { beforeExport },
    toast: (text: string, _ms: number, options: any = {}) => { toasts.push([text, options]); },
    round: (n: number) => n,
  };
  install(PM);
  const sheet = () => document.querySelector('.export-progress');
  const label = () => document.querySelector('.export-progress-label')?.textContent ?? null;
  const percent = () => document.querySelector('.export-progress [role="progressbar"]')?.getAttribute('aria-valuenow') ?? null;
  return {
    PM, exportDestination, exportVideo, beforeExport, cancelExport, revealExport, toasts, sheet, label, percent,
    check: () => checked(), answer: (start: unknown) => answer(start), status: (status: unknown) => listener?.(status),
    choose: async (destination: unknown = { status: 'chosen', destination: 'D1', path: '/x/Film.mp4' }) => {
      await vi.waitFor(() => expect(exportDestination).toHaveBeenCalledOnce());
      chose(destination);
      await new Promise(resolve => setTimeout(resolve, 0));
    },
    cancel: () => cancel(),
    run: () => PM.Export.run({ format: 'h264', fps: 30, scale: 1, range: 'all', quality: 'high', audio: false }),
  };
}

it('asks for the destination first, then shows the sheet through setup to Show in Finder', async () => {
  const f = fixture();
  const run = f.run();
  await vi.waitFor(() => expect(f.exportDestination).toHaveBeenCalledOnce());
  // The save panel comes before any progress or project work.
  expect(f.sheet()).toBeNull();
  expect(f.beforeExport).not.toHaveBeenCalled();
  expect(f.exportDestination.mock.calls[0]![0]).toMatchObject({ name: 'Film', options: { format: 'h264' }, clips: [] });
  await f.choose();
  // The project check runs with the sheet up.
  expect(f.beforeExport).toHaveBeenCalledOnce();
  expect(f.sheet()).not.toBeNull();
  expect(f.label()).toBe('Preparing…');
  expect(f.PM.Export.busy).toBe(true);
  f.check();
  await vi.waitFor(() => expect(f.exportVideo).toHaveBeenCalledOnce());
  expect(f.exportVideo.mock.calls[0]![0]).toMatchObject({ destination: 'D1' });
  f.answer({ status: 'accepted', jobId: 'J1', frames: 10, path: '/x/Film.mp4' });
  await new Promise(resolve => setTimeout(resolve, 0));
  f.status({ jobId: 'J1', state: 'queued', progress: 0 });
  f.status({ jobId: 'J1', state: 'running', progress: 0.5 });
  expect(f.label()).toContain('Frame 5 / 10');
  expect(f.percent()).toBe('50');
  f.status({ jobId: 'J1', state: 'done', progress: 1, path: '/x/Film.mp4' });
  expect(await run).toEqual({ cancelled: false });
  expect(f.sheet()).toBeNull();
  expect(f.PM.Export.busy).toBe(false);
  const [text, options] = f.toasts.at(-1)!;
  expect(text).toMatch(/^Export finished in /);
  expect(options).toMatchObject({ error: false, action: { label: 'Show in Finder' } });
  options.action.run();
  expect(f.revealExport).toHaveBeenCalledWith('/x/Film.mp4');
});

it('closes the sheet and reports a failed job until dismissed', async () => {
  const f = fixture();
  const run = f.run();
  await f.choose();
  f.check();
  await vi.waitFor(() => expect(f.exportVideo).toHaveBeenCalledOnce());
  f.answer({ status: 'accepted', jobId: 'J1', frames: 10, path: '/x/Film.mp4' });
  await new Promise(resolve => setTimeout(resolve, 0));
  f.status({ jobId: 'J1', state: 'failed', progress: 0.2, error: 'The disk is full.' });
  expect(await run).toEqual({ error: 'The disk is full.' });
  expect(f.sheet()).toBeNull();
  expect(f.toasts.at(-1)).toEqual(['Export failed: The disk is full.', expect.objectContaining({ error: true })]);
});

it('cancels while preparing without starting a job', async () => {
  const f = fixture();
  const run = f.run();
  await f.choose();
  f.cancel();
  expect(f.label()).toBe('Cancelling…');
  f.check();
  expect(await run).toEqual({ cancelled: true });
  expect(f.exportVideo).not.toHaveBeenCalled();
  expect(f.sheet()).toBeNull();
  expect(f.PM.Export.busy).toBe(false);
});

it('shows nothing when the save panel is cancelled', async () => {
  const f = fixture();
  const run = f.run();
  await f.choose({ status: 'cancelled' });
  expect(await run).toEqual({ cancelled: true });
  expect(f.beforeExport).not.toHaveBeenCalled();
  expect(f.exportVideo).not.toHaveBeenCalled();
  expect(f.sheet()).toBeNull();
  expect(f.toasts).toEqual([]);
});
