/* The native connection prepares one detached drawing before a viewer opens.
 * No session, surfaces or animation loop exists until the drawing is claimed.
 * The canvas and its worker stay together: a popout cannot claim another
 * document's OffscreenCanvas. Closing a viewer prepares the next drawing. */
import { createWebGpuPainter, type FramePainter } from './gpu';
import { PresentWorker, type WorkerLike } from './worker-presenter';

type Handlers = { onFailed(message: string): void; onLost(message: string): void };
export type WarmDrawing = {
  canvas: HTMLCanvasElement;
  worker: PresentWorker | null;
  painter: Promise<FramePainter | null> | null;
};

type Prepared = WarmDrawing & { handlers: Handlers | null; failed: boolean; claimed: boolean };

export class PresentationWarmup {
  private prepared: Prepared | null = null;
  private disposed = false;
  private workerUnavailable = false;

  constructor(private readonly doc: Document, private readonly createWorker: () => WorkerLike,
    private readonly createPainter = createWebGpuPainter) {}

  warm(): void {
    if (this.disposed || this.prepared) return;
    const canvas = this.doc.createElement('canvas');
    const prepared: Prepared = { canvas, worker: null, painter: null, handlers: null, failed: false, claimed: false };
    this.prepared = prepared;
    if (!this.workerUnavailable && typeof canvas.transferControlToOffscreen === 'function') {
      try {
        const offscreen = canvas.transferControlToOffscreen();
        prepared.worker = new PresentWorker({
          worker: this.createWorker(), canvas: offscreen,
          timeOrigin: this.doc.defaultView?.performance.timeOrigin,
          onFailed: message => {
            if (prepared.claimed) prepared.handlers?.onFailed(message);
            else this.failed(prepared);
          },
          onLost: message => {
            if (prepared.claimed) prepared.handlers?.onLost(message);
            else this.failed(prepared);
          }
        });
        return;
      } catch {
        // A transferred canvas cannot get a main-thread context. Start
        // the fallback on a fresh canvas while the viewer is still closed.
        this.workerUnavailable = true;
        this.prepared = null;
        this.warm();
        return;
      }
    }
    prepared.painter = this.createPainter(canvas).then(painter => {
      if ((this.disposed && !prepared.claimed) || prepared.failed) {
        painter.dispose();
        return null;
      }
      void painter.lost?.then(message => {
        if (prepared.claimed) prepared.handlers?.onLost(message);
        else this.failed(prepared);
      });
      return painter;
    }, () => { prepared.failed = true; return null; });
  }

  private failed(prepared: Prepared): void {
    prepared.failed = true;
    prepared.worker?.dispose();
    void prepared.painter?.then(painter => painter?.dispose());
    if (this.disposed || this.prepared !== prepared) return;
    this.prepared = null;
    // Failure in worker WebGPU does not imply the page cannot draw.
    this.workerUnavailable = true;
    this.warm();
  }

  /** The prepared drawing for a viewer in `doc`. A viewer that draws on
   *  the page's main thread (`allowWorker` false: the measurement harness's
   *  `offMainThread: false`) never claims a worker drawing; it stays
   *  prepared for the next viewer. */
  take(doc: Document, handlers: Handlers, allowWorker = true): WarmDrawing | null {
    const prepared = this.prepared;
    if (this.disposed || doc !== this.doc || !prepared || prepared.failed || prepared.worker?.failed || prepared.worker?.lost) return null;
    if (prepared.worker && !allowWorker) return null;
    this.prepared = null;
    prepared.claimed = true;
    prepared.handlers = handlers;
    return prepared;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    const prepared = this.prepared;
    this.prepared = null;
    prepared?.worker?.dispose();
    void prepared?.painter?.then(painter => painter?.dispose());
  }
}
