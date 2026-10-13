// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { prepareFrame } from './frame-preparation';

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function fontFixture() {
  let ready: Promise<unknown> = Promise.resolve();
  const getter = vi.fn(() => ready);
  const fonts = Object.assign(new EventTarget(), { status: 'loaded', size: 0 });
  Object.defineProperty(fonts, 'ready', { get: getter });
  vi.stubGlobal('document', { fonts });
  const handlers = new Map<string, Array<() => void>>();
  let version = 1;
  const on = (event: string, run: () => void) => handlers.set(event, [...handlers.get(event) || [], run]);
  const PM: any = { proj: { fps: 30, layers: [] }, scope: [], active: () => false,
    animVersion: () => version, bus: { on }, Kernel: { events: { on } } };
  return { PM, fonts, getter, edit: () => version++, emit: (event: string) => handlers.get(event)?.forEach(run => run()),
    loading() {
      fonts.status = 'loading'; fonts.dispatchEvent(new Event('loading'));
      let finish!: () => void;
      ready = new Promise<void>(resolve => { finish = () => {
        fonts.status = 'loaded'; fonts.dispatchEvent(new Event('loadingdone')); resolve();
      }; });
      return finish;
    } };
}

it('reuses completed font readiness across unchanged captures without rereading the layout getter', async () => {
  const { PM, getter } = fontFixture();
  await prepareFrame(PM, 0); await prepareFrame(PM, 1); await prepareFrame(PM, 2);
  expect(getter).toHaveBeenCalledOnce();
});

it('invalidates font readiness on font, project, edit and layer-structure changes', async () => {
  const { PM, fonts, getter, emit, edit } = fontFixture();
  await prepareFrame(PM, 0);
  for (const event of ['fonts', 'layers', 'project', 'extensions:changed', 'theme:changed']) { emit(event); await prepareFrame(PM, 0); }
  edit(); await prepareFrame(PM, 0);
  PM.proj.layers.push({ type: 'text' }); await prepareFrame(PM, 0);
  PM.proj = { ...PM.proj }; await prepareFrame(PM, 0);
  fonts.size++; await prepareFrame(PM, 0);
  expect(getter).toHaveBeenCalledTimes(10);
  await prepareFrame(PM, 1);
  expect(getter).toHaveBeenCalledTimes(10);
});

it('waits for a fresh font load and keeps the completed result reusable', async () => {
  const { PM, getter, loading } = fontFixture();
  await prepareFrame(PM, 0);
  const finish = loading();
  let completed = false;
  const preparing = prepareFrame(PM, 1).then(() => { completed = true; });
  await Promise.resolve(); expect(completed).toBe(false);
  finish(); await preparing;
  const reads = getter.mock.calls.length;
  await prepareFrame(PM, 2);
  expect(getter).toHaveBeenCalledTimes(reads);
});

it('does not reuse a stale pending promise when another font load starts', async () => {
  const { PM, getter, loading, fonts } = fontFixture();
  const finishFirst = loading();
  let completed = false;
  const preparing = prepareFrame(PM, 0).then(() => { completed = true; });
  const finishSecond = loading();
  finishFirst(); fonts.status = 'loading';
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(completed).toBe(false);
  expect(getter.mock.calls.length).toBeGreaterThan(1);
  finishSecond(); await preparing;
  expect(completed).toBe(true);
});

it('checks a replacement FontFaceSet instead of using the previous set\'s readiness', async () => {
  const { PM } = fontFixture();
  await prepareFrame(PM, 0);
  const replacement = Object.assign(new EventTarget(), { status: 'loaded', size: 0 });
  const ready = vi.fn(() => Promise.resolve());
  Object.defineProperty(replacement, 'ready', { get: ready });
  (document as any).fonts = replacement;
  await prepareFrame(PM, 1); await prepareFrame(PM, 2);
  expect(ready).toHaveBeenCalledOnce();
});

it.each(['edit', 'project', 'fonts'])('rechecks a pending font wait after a %s change', async change => {
  const { PM, getter, loading, edit, emit } = fontFixture();
  const finish = loading();
  const preparing = prepareFrame(PM, 0);
  if (change === 'edit') edit();
  else if (change === 'project') PM.proj = { ...PM.proj };
  else emit('fonts');
  finish(); await preparing;
  expect(getter.mock.calls.length).toBeGreaterThan(1);
});

it('ignores a queued seek completion until the requested frame has decoded', async () => {
  const video = new EventTarget() as any;
  Object.assign(video, { paused: true, seeking: true, readyState: 2, currentTime: 0, pause() {} });
  const draw = vi.fn();
  const context = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage: draw } as any);
  const layer = { id: 'clip', type: 'video', from: 0, d: { asset: 'source', speed: 1, trim: 0 } };
  const asset = { el: video, w: 16, h: 16, dur: 2 };
  const PM: any = { proj: { layers: [layer], fps: 30 }, active: () => true,
    scope: { push() {}, pop() {} }, assets: { get: () => asset } };
  try {
    const pending = prepareFrame(PM, 1.5);
    video.dispatchEvent(new Event('seeked'));
    await Promise.resolve();
    expect(draw).not.toHaveBeenCalled();
    video.seeking = false; video.currentTime = 0;
    video.dispatchEvent(new Event('seeked'));
    await Promise.resolve();
    expect(draw).not.toHaveBeenCalled();
    video.currentTime = 1.5;
    video.dispatchEvent(new Event('seeked'));
    await pending;
    expect(draw).toHaveBeenCalledOnce();
    expect(PM.preparedVideoFrames.has('clip@1.5')).toBe(true);
  } finally { context.mockRestore(); }
});
