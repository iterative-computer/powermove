// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { PresentationWarmup } from './prewarm';
import { ViewerPresentation, type EngineViewerService } from './presentation';
import type { FramePainter } from './gpu';
import type { FromWorker, ToWorker } from './present-worker';
import type { WorkerLike } from './worker-presenter';

class Worker implements WorkerLike {
  messages: ToWorker[] = [];
  listeners: ((e: MessageEvent<FromWorker>) => void)[] = [];
  terminated = false;
  postMessage(message: ToWorker): void {
    this.messages.push(message);
    if (message.type === 'dispose') this.emit({ type: 'disposed' });
  }
  addEventListener(type: string, listener: any): void { if (type === 'message') this.listeners.push(listener); }
  terminate(): void { this.terminated = true; }
  emit(data: FromWorker): void { for (const fn of this.listeners) fn({ data } as MessageEvent<FromWorker>); }
}
const painter = (): FramePainter & { disposed: boolean } => ({ disposed: false, configure() {}, draw() {}, workDone: async () => {}, dispose() { this.disposed = true; } });
const handlers = { onFailed() {}, onLost() {} };
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };

describe('presentation warmup', () => {
  it('prepares only once and mounts the existing canvas and worker without starting another', async () => {
    const doc = document.implementation.createHTMLDocument();
    const create = doc.createElement.bind(doc);
    doc.createElement = ((name: string) => {
      const el = create(name);
      if (name === 'canvas') (el as HTMLCanvasElement).transferControlToOffscreen = () => ({}) as OffscreenCanvas;
      return el;
    }) as typeof doc.createElement;
    let starts = 0;
    const worker = new Worker();
    const warmup = new PresentationWarmup(doc, () => { starts++; return worker; });
    warmup.warm(); warmup.warm();
    expect(starts).toBe(1);
    const canvas = doc.createElement('canvas');
    const stage = doc.createElement('div'); stage.append(canvas); doc.body.append(stage);
    const service: EngineViewerService = { source: { videoFrame: () => null, onSurface: () => () => {}, retire: () => 0 }, composition: () => 'C', open: () => null, onReady: () => () => {}, drawingWarmup: warmup };
    const view = new ViewerPresentation({ stage, service, createWorker: () => { throw new Error('mount must claim the warm worker'); } });
    await flush();
    expect(starts).toBe(1);
    expect(worker.messages.map(m => m.type)).toEqual(['init']);
    expect(warmup.take(doc, handlers)).toBeNull();
    view.dispose(); warmup.dispose();
    expect(worker.terminated).toBe(true);
  });

  it('prepares the page painter after an unused worker fails, and forwards failures after claim', async () => {
    const doc = document.implementation.createHTMLDocument();
    const create = doc.createElement.bind(doc);
    doc.createElement = ((name: string) => {
      const el = create(name);
      if (name === 'canvas') (el as HTMLCanvasElement).transferControlToOffscreen = () => ({}) as OffscreenCanvas;
      return el;
    }) as typeof doc.createElement;
    const worker = new Worker();
    const made = painter();
    const warmup = new PresentationWarmup(doc, () => worker, async () => made);
    warmup.warm();
    worker.emit({ type: 'failed', message: 'no worker WebGPU' });
    await flush();
    expect(worker.terminated).toBe(true);
    const drawing = warmup.take(doc, handlers)!;
    expect(drawing.worker).toBeNull();
    expect(await drawing.painter).toBe(made);
    made.dispose(); warmup.dispose();

    const next = new Worker();
    const second = new PresentationWarmup(doc, () => next);
    second.warm();
    const messages: string[] = [];
    const claimed = second.take(doc, { onFailed: m => messages.push(m), onLost: m => messages.push(m) })!;
    next.emit({ type: 'failed', message: 'claimed worker failed' });
    expect(messages).toEqual(['claimed worker failed']);
    claimed.worker!.dispose(); second.dispose();
  });

  it('disposes an unused main-thread painter, even when initialization completes after shutdown', async () => {
    let resolve!: (p: FramePainter) => void;
    const warmup = new PresentationWarmup(document, () => { throw new Error('no worker'); }, () => new Promise(r => { resolve = r; }));
    warmup.warm(); warmup.dispose();
    const made = painter(); resolve(made); await flush();
    expect(made.disposed).toBe(true);
    expect(warmup.take(document, handlers)).toBeNull();
  });

  it('does not hand a drawing to another document or dispose an already claimed painter', async () => {
    const made = painter();
    const warmup = new PresentationWarmup(document, () => { throw new Error('no worker'); }, async () => made);
    warmup.warm(); await flush();
    expect(warmup.take(document.implementation.createHTMLDocument(), handlers)).toBeNull();
    const drawing = warmup.take(document, handlers)!;
    expect(await drawing.painter).toBe(made);
    warmup.dispose(); await flush();
    expect(made.disposed).toBe(false);
    made.dispose();
  });
});
