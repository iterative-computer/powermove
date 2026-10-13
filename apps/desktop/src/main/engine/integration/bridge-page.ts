/*
 * The page of the frame bridge's Electron integration test
 * (scripts/frame-bridge-integration.mjs; bridge-main.ts drives it).
 *
 * It plays the kernel's engine connection for the viewer: it takes the
 * interactive engine's port from the preload's window message, negotiates
 * as a renderer, opens a session, and registers nothing more than the
 * viewer's `EngineViewerService` contract, which it hands to the viewer's
 * real `ViewerPresentation` (WebGPU canvas, presenter, jitter buffer,
 * release on `onSubmittedWorkDone`). The frame source is the real preload
 * receiver (`window.powermove.frameBridge`).
 *
 * Every frame drawn is also read back: its frame code (seq and view
 * sequence, written by the engine's test source) must equal the
 * descriptor's. That is what proves repeated `getVideoFrame()` calls on one
 * persistent import see the surface's current contents.
 */
import { PROTOCOL_BUILD, type ClockSample, type Envelope, type FrameDescriptor, type Request, type Response, type SurfacePool, type TransportCommand, type ViewerState } from '../../../shared/engine-protocol';
import type { FrameBridgeApi } from '../../../shared/frame-bridge';
import { createWebGpuPainter, type FramePainter } from '../../../extensions/viewer/engine-present/gpu';
import type { PresentedFrame } from '../../../extensions/viewer/engine-present/presenter';
import { ViewerPresentation, type EngineViewerChannel, type EngineViewerHandlers, type EngineViewerService, type ViewParams } from '../../../extensions/viewer/engine-present/presentation';
import type { EngineFrameMessage } from '../../../extensions/viewer/engine-present/protocol';
import { MAX_MESSAGE_BYTES } from '../msgpack';
import { completeWelcome, decodeEnvelope, encodeEnvelope, engineRole, envelope, eventMessage, localRanges, Outstanding, requestMessage } from '../protocol';

/* eslint-disable @typescript-eslint/no-explicit-any */
const params = new URLSearchParams(location.search);
const displayHz = Number(params.get('hz')) || null;
const log = (message: string): void => console.log(`[bridge-page] ${message}`);

const frameBridge = (globalThis as { powermove?: { frameBridge?: FrameBridgeApi } }).powermove?.frameBridge;
if (!frameBridge) throw new Error('the preload did not expose the frame bridge receiver');

/* ── the engine connection (what the kernel's connection does) ───────── */

type Session = { id: number | null; handlers: EngineViewerHandlers; pending: ViewerState | null; closed: boolean };

class Connection {
  private outstanding = new Outstanding();
  private welcome: ((ok: boolean) => void) | null = null;
  readonly sessions = new Set<Session>();
  ready = false;
  closed = false;
  errors: string[] = [];

  constructor(readonly generation: number, private readonly port: MessagePort, private readonly onClose: () => void) {
    port.onmessage = (event) => this.receive(new Uint8Array(event.data as ArrayBuffer));
    port.addEventListener('close', () => {
      log(`the engine port of generation ${generation} closed`);
      this.close('the engine port closed');
    });
    port.start();
  }

  async open(): Promise<void> {
    const done = new Promise<boolean>((resolve) => (this.welcome = resolve));
    this.port.postMessage(encodeEnvelope(envelope({ kind: 'hello', role: 'renderer', wire: 1, build: PROTOCOL_BUILD, contracts: localRanges(), maxMessageBytes: MAX_MESSAGE_BYTES })));
    if (!(await done)) throw new Error('the interactive engine refused the handshake');
    this.ready = true;
  }

  request(body: Request, session: number | null = null): Promise<Response> {
    const { id, answer } = this.outstanding.open(body);
    this.port.postMessage(encodeEnvelope(envelope(requestMessage(id, body), session)));
    return answer;
  }

  event(body: any, session: number): void {
    if (!this.closed) this.port.postMessage(encodeEnvelope(envelope(eventMessage(body), session)));
  }

  close(reason: string): void {
    if (this.closed) return;
    this.closed = true;
    this.port.close();
    this.outstanding.failAll(new Error(reason));
    for (const session of this.sessions) {
      session.closed = true;
      session.handlers.closed(reason);
    }
    this.sessions.clear();
    this.onClose();
  }

  private receive(bytes: Uint8Array): void {
    let message: Envelope;
    try {
      message = decodeEnvelope(bytes);
    } catch (error) {
      this.errors.push(`decode: ${error instanceof Error ? error.message : String(error)}`);
      return;
    }
    const arrived = performance.now();
    const body = message.message;
    if (this.welcome) {
      const settle = this.welcome;
      this.welcome = null;
      if (body.kind !== 'welcome') return settle(false);
      try {
        completeWelcome(body, engineRole('interactive'));
        settle(true);
      } catch {
        settle(false);
      }
      return;
    }
    if (body.kind === 'response' || body.kind === 'error') {
      if (body.kind === 'error') this.errors.push(`${body.error.code}: ${body.error.message}`);
      this.outstanding.settle(body);
      return;
    }
    if (body.kind !== 'event') return;
    const event = body.body as { type: string };
    let decoded: EngineFrameMessage | null = null;
    if (event.type === 'frame') {
      const { type: _t, ...frame } = event as { type: 'frame' } & FrameDescriptor;
      decoded = { type: 'frame', frame: frame as any };
    } else if (event.type === 'surfacePool') {
      const { type: _t, ...pool } = event as { type: 'surfacePool' } & SurfacePool;
      decoded = { type: 'surfacePool', pool: pool as any };
    } else if (event.type === 'clock') {
      const { type: _t, ...sample } = event as { type: 'clock' } & ClockSample;
      decoded = { type: 'clock', sample: sample as any };
    } else if (event.type === 'failure') {
      this.errors.push(`failure: ${JSON.stringify(event)}`);
    }
    if (!decoded) return;
    for (const session of this.sessions) if (session.id === message.session) session.handlers.message(decoded, arrived);
  }
}

const COLOR = { mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe' } as const;

class PageEngine implements EngineViewerService {
  readonly source = frameBridge!;
  connection: Connection | null = null;
  connections = 0;
  private ready = new Set<() => void>();
  readonly channels: (EngineViewerChannel & { transport(command: TransportCommand): void; session(): number | null })[] = [];

  constructor() {
    window.addEventListener('message', (event) => {
      const data = event.data as { type?: string; process?: string; generation?: number } | null;
      if (data?.type !== 'powermove:engine-port' || data.process !== 'interactive' || !event.ports[0]) return;
      log(`interactive engine port (generation ${data.generation})`);
      void this.connect(data.generation ?? 0, event.ports[0]);
    });
  }

  private async connect(generation: number, port: MessagePort): Promise<void> {
    this.connection?.close('a new interactive engine port arrived');
    const connection = new Connection(generation, port, () => {
      if (this.connection === connection) this.connection = null;
    });
    this.connection = connection;
    try {
      await connection.open();
    } catch (error) {
      log(`handshake failed: ${error instanceof Error ? error.message : String(error)}`);
      return;
    }
    this.connections++;
    log(`connected to the interactive engine (launch generation ${generation})`);
    for (const listener of this.ready) listener();
  }

  composition(): string {
    return 'bridge-test';
  }

  displayHz(): number | null {
    return displayHz;
  }

  onReady(listener: () => void): () => void {
    this.ready.add(listener);
    return () => this.ready.delete(listener);
  }

  open(handlers: EngineViewerHandlers): EngineViewerChannel | null {
    const connection = this.connection;
    if (!connection?.ready || connection.closed) return null;
    const session: Session = { id: null, handlers, pending: null, closed: false };
    connection.sessions.add(session);
    void connection
      .request({ type: 'openSession', document: 'bridge-test', role: 'interactive', process: 'interactive', color: COLOR } as Request)
      .then((opened) => {
        if (opened.type !== 'sessionOpened' || session.closed) return;
        session.id = opened.session;
        if (session.pending) connection.event({ type: 'viewer', ...session.pending }, session.id);
        session.pending = null;
      })
      .catch((error: unknown) => log(`openSession failed: ${error instanceof Error ? error.message : String(error)}`));
    const channel = {
      sendViewerState(state: ViewerState) {
        if (session.id === null) session.pending = state;
        else connection.event({ type: 'viewer', ...state }, session.id);
      },
      release(release: { poolIndex: number; seq: number }) {
        if (session.id !== null) connection.event({ type: 'releaseFrame', ...release }, session.id);
      },
      transport(command: TransportCommand) {
        if (session.id !== null) connection.event({ type: 'transport', ...command }, session.id);
      },
      session: () => session.id,
      close() {
        if (session.closed) return;
        session.closed = true;
        connection.sessions.delete(session);
        if (session.id !== null) void connection.request({ type: 'closeSession' } as Request, session.id).catch(() => {});
      }
    };
    this.channels.push(channel);
    return channel;
  }
}

/* ── frame-code readback ───────────────────────────────────────────── */

const CODE_SHADER = /* wgsl */ `
struct U { cell: f32, width: f32, height: f32, pad: f32 };
@group(0) @binding(0) var src: texture_external;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> u: U;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let uv = vec2f((floor(pos.x) + 0.5) * u.cell / u.width, 16.0 / u.height);
  return textureSampleBaseClampToEdge(src, samp, uv);
}`;

type CodeCheck = { checked: number; ok: number; mismatches: string[]; distinctSeqs: Set<number> };

class CodeReader {
  readonly check: CodeCheck = { checked: 0, ok: 0, mismatches: [], distinctSeqs: new Set() };
  private queue: Promise<{ seq: number; view: number }>[] = [];
  private constructor(private readonly device: any, private readonly pipeline: any, private readonly sampler: any) {}

  static async create(): Promise<CodeReader> {
    const gpu = (navigator as any).gpu;
    const adapter = await gpu.requestAdapter({ powerPreference: 'high-performance' });
    const device = await adapter.requestDevice();
    const module = device.createShaderModule({ code: CODE_SHADER });
    const pipeline = device.createRenderPipeline({
      layout: 'auto',
      vertex: { module, entryPoint: 'vs' },
      fragment: { module, entryPoint: 'fs', targets: [{ format: 'rgba8unorm' }] },
      primitive: { topology: 'triangle-list' }
    });
    return new CodeReader(device, pipeline, device.createSampler({ magFilter: 'nearest', minFilter: 'nearest' }));
  }

  /** Records a readback of `frame`'s code (call before the frame closes). */
  record(frame: VideoFrame): void {
    const d = this.device;
    const width = frame.codedWidth;
    const cell = Math.min(32, Math.max(1, Math.floor(width / 64)));
    const uniform = d.createBuffer({ size: 16, usage: 0x40 | 0x08 }); // UNIFORM | COPY_DST
    d.queue.writeBuffer(uniform, 0, new Float32Array([cell, width, frame.codedHeight, 0]));
    const target = d.createTexture({ size: [64, 1], format: 'rgba8unorm', usage: 0x10 | 0x01 }); // RENDER_ATTACHMENT | COPY_SRC
    const out = d.createBuffer({ size: 256, usage: 0x01 | 0x08 }); // MAP_READ | COPY_DST
    const group = d.createBindGroup({
      layout: this.pipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: d.importExternalTexture({ source: frame }) },
        { binding: 1, resource: this.sampler },
        { binding: 2, resource: { buffer: uniform } }
      ]
    });
    const encoder = d.createCommandEncoder();
    const pass = encoder.beginRenderPass({ colorAttachments: [{ view: target.createView(), loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 0, a: 0 } }] });
    pass.setPipeline(this.pipeline);
    pass.setBindGroup(0, group);
    pass.draw(3);
    pass.end();
    encoder.copyTextureToBuffer({ texture: target }, { buffer: out, bytesPerRow: 256 }, [64, 1]);
    d.queue.submit([encoder.finish()]);
    this.queue.push(
      out.mapAsync(1).then(() => {
        const bytes = new Uint8Array(out.getMappedRange());
        let seq = 0;
        let view = 0;
        for (let c = 0; c < 32; c++) {
          if (bytes[c * 4]! > 127) seq += 2 ** c;
          if (bytes[(c + 32) * 4]! > 127) view += 2 ** c;
        }
        out.unmap();
        out.destroy();
        target.destroy();
        uniform.destroy();
        return { seq, view };
      })
    );
  }

  /** Matches the oldest readback to the frame the presenter reports drawn. */
  verify(frame: PresentedFrame): void {
    const pending = this.queue.shift();
    if (!pending) {
      this.check.mismatches.push(`seq ${frame.seq}: no readback`);
      return;
    }
    void pending.then(({ seq, view }) => {
      this.check.checked++;
      const expectedSeq = frame.seq % 2 ** 32;
      const expectedView = frame.viewSeq % 2 ** 32;
      if (seq === expectedSeq && view === expectedView) {
        this.check.ok++;
        this.check.distinctSeqs.add(seq);
      } else if (this.check.mismatches.length < 20) {
        this.check.mismatches.push(`descriptor seq ${expectedSeq} view ${expectedView}, pixels seq ${seq} view ${view}`);
      }
    });
  }
}

function verifying(inner: FramePainter, reader: CodeReader): FramePainter {
  return {
    configure: (...args) => inner.configure(...args),
    draw(frame) {
      inner.draw(frame);
      reader.record(frame);
    },
    workDone: () => inner.workDone(),
    dispose: () => inner.dispose()
  };
}

/* ── the test API bridge-main.ts calls ─────────────────────────────── */

const engine = new PageEngine();
const stage = document.getElementById('stage')!;
const overlay = document.getElementById('overlay')!;
let view: ViewParams = { zoom: 640 / 3840, pan: [0, 0], resolution: 'full', media: 'automatic', playEveryFrame: false };
let reader: CodeReader | null = null;
let presentation: ViewerPresentation | null = null;
let popout: Window | null = null;
let presented = 0;
const formats = new Set<string>();

async function start(): Promise<void> {
  reader = await CodeReader.create();
  presentation = new ViewerPresentation({
    service: engine,
    stage,
    before: overlay,
    createPainter: async (canvas) => verifying(await createWebGpuPainter(canvas), reader!),
    log: (level, message) => log(`${level}: ${message}`)
  });
  presentation.onPresented((frame) => {
    presented++;
    reader!.verify(frame);
  });
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
  stats() {
    const latest = presentation?.latest ?? null;
    if (latest) formats.add(presentation?.state?.edr ? 'edr' : 'sdr');
    return {
      presented,
      presenting: presentation?.presenting ?? false,
      presenter: presentation?.stats() ?? null,
      state: presentation?.state ?? null,
      latest: latest && { seq: latest.seq, revision: latest.revision, viewSeq: latest.viewSeq, timed: latest.timed },
      overlayCss: presentation?.overlayInCss() ?? null,
      code: reader ? { checked: reader.check.checked, ok: reader.check.ok, mismatches: reader.check.mismatches, distinct: reader.check.distinctSeqs.size } : null,
      receiver: frameBridge!.stats(),
      connections: engine.connections,
      connection: engine.connection ? { generation: engine.connection.generation, errors: engine.connection.errors } : null,
      inPopout: stage.ownerDocument !== document,
      dpr: (stage.ownerDocument.defaultView ?? window).devicePixelRatio
    };
  }
};

(window as unknown as { __bridgeIt: typeof api }).__bridgeIt = api;
log('ready');
