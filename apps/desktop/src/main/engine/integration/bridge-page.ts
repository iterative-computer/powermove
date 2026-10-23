/*
 * The page of the frame bridge's Electron integration test
 * (scripts/frame-bridge-integration.mjs; bridge-main.ts drives it).
 *
 * It plays the kernel's engine connection for the viewer (page-engine.ts)
 * and hands the viewer's `EngineViewerService` contract to the viewer's
 * real `ViewerPresentation` (WebGPU canvas, presenter, jitter buffer,
 * release on `onSubmittedWorkDone`). The frame source is the real preload
 * receiver (`window.powermove.frameBridge`).
 *
 * Every frame drawn is also read back: its frame code (seq and view
 * sequence, written by the engine's test source) must equal the
 * descriptor's. That is what proves repeated `getVideoFrame()` calls on one
 * persistent import see the surface's current contents.
 */
import type { TransportCommand } from '../../../shared/engine-protocol';
import { createFramePainter } from '../../../extensions/viewer/engine-present/painter';
import { ViewerPresentation, type ViewParams } from '../../../extensions/viewer/engine-present/presentation';
import { CodeReader, frameBridge, log, PageEngine, verifying } from './page-engine';

const params = new URLSearchParams(location.search);
const displayHz = Number(params.get('hz')) || null;
/** `worker`: the viewer's presentation worker draws (bridge-worker.ts reads frames back there). */
const offMain = params.get('present') === 'worker';
let worker: Worker | null = null;
type WorkerCheck = { checked: number; ok: number; mismatches: string[]; distinct: number };
/** The latest checks of every presentation worker this page made. */
const workerChecks = new Map<Worker, WorkerCheck>();

function makeWorker(): Worker {
  const w = new Worker('bridge-worker.js', { name: 'engine-present' });
  w.addEventListener('message', (event) => {
    if (event.data?.type === 'it-check') workerChecks.set(w, event.data.check as WorkerCheck);
  });
  worker = w;
  return w;
}

function summedChecks(): WorkerCheck {
  const sum: WorkerCheck = { checked: 0, ok: 0, mismatches: [], distinct: 0 };
  for (const c of workerChecks.values()) {
    sum.checked += c.checked;
    sum.ok += c.ok;
    sum.mismatches.push(...c.mismatches);
    sum.distinct += c.distinct;
  }
  return sum;
}

/** Asks the current presentation worker and waits for its answer. */
function askWorker<T>(type: 'it-quality', pick: (data: { type: string } & Record<string, unknown>) => T | undefined): Promise<T> {
  const w = worker!;
  return new Promise((resolve) => {
    const listen = (event: MessageEvent): void => {
      if (event.data?.type !== type) return;
      w.removeEventListener('message', listen);
      resolve(pick(event.data)!);
    };
    w.addEventListener('message', listen);
    w.postMessage({ type });
  });
}
const receiver = frameBridge();

/* ── the test API bridge-main.ts calls ─────────────────────────────── */

const engine = new PageEngine(displayHz);
const stage = document.getElementById('stage')!;
const overlay = document.getElementById('overlay')!;
let view: ViewParams = { zoom: 640 / 3840, pan: [0, 0], resolution: 'full', media: 'automatic', playEveryFrame: false };
let reader: CodeReader | null = null;
let presentation: ViewerPresentation | null = null;
let popout: Window | null = null;
let presented = 0;

async function start(): Promise<void> {
  if (offMain) {
    presentation = new ViewerPresentation({
      service: engine,
      stage,
      before: overlay,
      createWorker: makeWorker,
      log: (level, message) => log(`${level}: ${message}`)
    });
    presentation.onPresented(() => presented++);
  } else {
    reader = await CodeReader.create();
    presentation = new ViewerPresentation({
      service: engine,
      stage,
      before: overlay,
      createPainter: async (canvas) => verifying(await createFramePainter(canvas), reader!),
      log: (level, message) => log(`${level}: ${message}`)
    });
    presentation.onPresented((frame) => {
      presented++;
      reader!.verify(frame);
    });
  }
  presentation.update(view);
  new ResizeObserver(() => presentation?.update(view)).observe(stage);
}

function currentChannel(): (typeof engine.channels)[number] | undefined {
  return engine.channels.at(-1);
}

const api = {
  start,
  setView(next: Partial<ViewParams>): number {
    view = { ...view, ...next };
    presentation!.update(view);
    return presentation!.state?.viewSeq ?? 0;
  },
  transport(command: TransportCommand): boolean {
    const channel = currentChannel();
    if (!channel || channel.session() === null) return false;
    channel.transport(command);
    return true;
  },
  /** Moves the stage into a same-origin popout window (or back). */
  popout(): boolean {
    popout = window.open('about:blank', 'pm-bridge-popout', 'width=480,height=270');
    if (!popout) return false;
    popout.document.body.style.cssText = 'margin:0;background:#000';
    popout.document.body.appendChild(popout.document.adoptNode(stage));
    presentation!.update(view);
    return true;
  },
  popin(): boolean {
    document.body.prepend(document.adoptNode(stage));
    presentation!.update(view);
    popout?.close();
    popout = null;
    return true;
  },
  /** The next drawn frame's ramp and HDR patch rows (a view change forces one). */
  async quality(): Promise<{ format: string; rows: import('./page-engine').QualityRows }> {
    const rows = offMain ? askWorker('it-quality', (d) => d['rows'] as import('./page-engine').QualityRows) : reader!.nextQuality();
    view = { ...view, pan: [view.pan[0] + 1, view.pan[1]] };
    presentation!.update(view);
    return { format: presentation!.state?.edr ? 'rgbaf16' : 'bgra', rows: await rows };
  },
  stats() {
    const latest = presentation?.latest ?? null;
    return {
      offMainThread: presentation?.offMainThread ?? false,
      presented,
      presenting: presentation?.presenting ?? false,
      presenter: presentation?.stats() ?? null,
      state: presentation?.state ?? null,
      latest: latest && { seq: latest.seq, revision: latest.revision, viewSeq: latest.viewSeq, timed: latest.timed },
      overlayCss: presentation?.overlayInCss() ?? null,
      code: offMain ? summedChecks() : reader ? { checked: reader.check.checked, ok: reader.check.ok, mismatches: reader.check.mismatches, distinct: reader.check.distinctSeqs.size } : null,
      receiver: receiver.stats(),
      connections: engine.connections,
      connection: engine.connection ? { generation: engine.connection.generation, errors: engine.connection.errors } : null,
      inPopout: stage.ownerDocument !== document,
      dpr: (stage.ownerDocument.defaultView ?? window).devicePixelRatio
    };
  }
};

(window as unknown as { __bridgeIt: typeof api }).__bridgeIt = api;
log('ready');
