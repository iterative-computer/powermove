/*
 * What the frame bridge's Electron test pages share (bridge-page.ts, the
 * integration test; measure-page.ts, the S1 measurement): the engine
 * connection playing the kernel's part for the viewer, and the frame-code
 * readback.
 */
import { PROTOCOL_BUILD, type ClockSample, type Envelope, type FrameDescriptor, type Request, type Response, type SurfacePool, type TransportCommand, type ViewerState } from '../../../shared/engine-protocol';
import type { FrameBridgeApi } from '../../../shared/frame-bridge';
import type { FramePainter } from '../../../extensions/viewer/engine-present/gpu';
import type { PresentedFrame } from '../../../extensions/viewer/engine-present/presenter';
import type { EngineViewerChannel, EngineViewerHandlers, EngineViewerService } from '../../../extensions/viewer/engine-present/presentation';
import type { EngineFrameMessage } from '../../../extensions/viewer/engine-present/protocol';
import { MAX_MESSAGE_BYTES } from '../msgpack';
import { completeWelcome, decodeEnvelope, encodeEnvelope, engineRole, envelope, eventMessage, localRanges, Outstanding, requestMessage } from '../protocol';

/* eslint-disable @typescript-eslint/no-explicit-any */
export const log = (message: string): void => console.log(`[bridge-page] ${message}`);

export function frameBridge(): FrameBridgeApi {
  // The test pages keep `window.powermove`; the real app's kernel takes it
  // off the page at boot, and hidden test launches publish the same bridge
  // as the e2e seam (shared/test-bridge.ts), which the real-viewer
  // measurement uses.
  const host = globalThis as { powermove?: { frameBridge?: FrameBridgeApi }; __powermoveTestBridge?: { frameBridge?: FrameBridgeApi } };
  const bridge = host.powermove?.frameBridge ?? host.__powermoveTestBridge?.frameBridge;
  if (!bridge) throw new Error('the preload did not expose the frame bridge receiver');
  return bridge;
}

export type PageChannel = EngineViewerChannel & { transport(command: TransportCommand): void; session(): number | null };

/* ── the engine connection (what the kernel's connection does) ───────── */

type Session = { id: number | null; handlers: EngineViewerHandlers; pending: ViewerState | null; closed: boolean };

export class Connection {
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

export class PageEngine implements EngineViewerService {
  readonly source = frameBridge();
  connection: Connection | null = null;
  connections = 0;
  private ready = new Set<() => void>();
  readonly channels: PageChannel[] = [];

  constructor(private readonly hz: number | null = null) {
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
    return this.hz;
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

/** Samples one frame row at `n` evenly spaced columns into an n×1 rgba32float
 *  target (exact for half floats: an f16 target would add its own rounding;
 *  Apple GPUs truncate on render-target writes). */
const ROW_SHADER = /* wgsl */ `
struct U { y: f32, height: f32, n: f32, pad: f32 };
@group(0) @binding(0) var src: texture_external;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> u: U;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  return textureSampleBaseClampToEdge(src, samp, vec2f((floor(pos.x) + 0.5) / u.n, (u.y + 0.5) / u.height));
}`;

/** The test pattern's quality rows read back from one drawn frame. */
export type QualityRows = { width: number; height: number; ramp: number[]; patches: number[] };

export type CodeCheck = { checked: number; ok: number; mismatches: string[]; distinctSeqs: Set<number> };

export class CodeReader {
  readonly check: CodeCheck = { checked: 0, ok: 0, mismatches: [], distinctSeqs: new Set() };
  private queue: Promise<{ seq: number; view: number }>[] = [];
  private rowPipeline: any = null;
  private quality: ((rows: Promise<QualityRows>) => void) | null = null;
  private constructor(private readonly device: any, private readonly pipeline: any, private readonly sampler: any) {}

  /** Reads the ramp (every column of row 192) and the HDR patches (row 704)
   *  of the next frame drawn. */
  nextQuality(): Promise<QualityRows> {
    return new Promise((resolve) => (this.quality = (rows) => void rows.then(resolve)));
  }

  private readRow(frame: VideoFrame, y: number, n: number): Promise<number[]> {
    const d = this.device;
    if (!this.rowPipeline) {
      const module = d.createShaderModule({ code: ROW_SHADER });
      this.rowPipeline = d.createRenderPipeline({ layout: 'auto', vertex: { module, entryPoint: 'vs' }, fragment: { module, entryPoint: 'fs', targets: [{ format: 'rgba32float' }] }, primitive: { topology: 'triangle-list' } });
    }
    const uniform = d.createBuffer({ size: 16, usage: 0x40 | 0x08 });
    d.queue.writeBuffer(uniform, 0, new Float32Array([y, frame.codedHeight, n, 0]));
    const target = d.createTexture({ size: [n, 1], format: 'rgba32float', usage: 0x10 | 0x01 });
    const bytesPerRow = Math.ceil((n * 16) / 256) * 256;
    const out = d.createBuffer({ size: bytesPerRow, usage: 0x01 | 0x08 });
    const group = d.createBindGroup({
      layout: this.rowPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: d.importExternalTexture({ source: frame }) },
        { binding: 1, resource: this.sampler },
        { binding: 2, resource: { buffer: uniform } }
      ]
    });
    const encoder = d.createCommandEncoder();
    const pass = encoder.beginRenderPass({ colorAttachments: [{ view: target.createView(), loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 0, a: 0 } }] });
    pass.setPipeline(this.rowPipeline);
    pass.setBindGroup(0, group);
    pass.draw(3);
    pass.end();
    encoder.copyTextureToBuffer({ texture: target }, { buffer: out, bytesPerRow }, [n, 1]);
    d.queue.submit([encoder.finish()]);
    return out.mapAsync(1).then(() => {
      const floats = new Float32Array(out.getMappedRange());
      const red = Array.from({ length: n }, (_, i) => floats[i * 4]!);
      out.unmap();
      out.destroy();
      target.destroy();
      uniform.destroy();
      return red;
    });
  }

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
    if (this.quality) {
      const deliver = this.quality;
      this.quality = null;
      const width = frame.codedWidth;
      const height = frame.codedHeight;
      deliver(Promise.all([this.readRow(frame, 192, width), this.readRow(frame, 704, 8)]).then(([ramp, patches]) => ({ width, height, ramp, patches })));
    }
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

export function verifying(inner: FramePainter, reader: CodeReader): FramePainter {
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
