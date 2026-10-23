/*
 * The host's WebCodecs encoders for the job engine's WebM exports (decision
 * EXPORT-1; contract 14 `hostEncode`, pm_protocol::host_encode).
 *
 * Today's exporter encodes WebM with Chromium's own VideoEncoder and
 * AudioEncoder; no other build of libvpx and libopus gives the same
 * pictures and samples. So the job engine renders each frame and mixes the
 * audio, and asks the host to encode them: main keeps one hidden encoder
 * window (renderer/src/export-encoder/main.ts) that runs today's encoder
 * settings and muxer on the engine's canvas bytes and mix, and main writes
 * the muxed file into the export's temporary output, which the engine then
 * verifies and commits as usual.
 *
 * One encode per job and engine port. Steps arrive in order. Frame and
 * audio steps are answered as soon as the window has them while at most
 * `MAX_IN_FLIGHT` are unanswered (a window error fails the next step), so a
 * slow encoder slows the engine rather than buffering frames here; `open`
 * and `finish` are answered after the window answered them. An encode no step reaches for
 * `idleMs` while nothing is in flight is aborted (the engine that asked is
 * gone); the window closes `closeMs` after its last encode ends and is
 * reopened by the next.
 */
import type { CapabilityHandle, HostEncode } from '../../shared/engine-protocol';
import type { EncoderCommand, EncoderReply } from '../../shared/export-encoder';
import { ProtocolError } from './protocol';
import type { Receiver } from './bridge';

/** The encoder window as main drives it. */
export interface EncoderWindow {
  post(command: EncoderCommand): void;
  readonly receiver?: Receiver;
  retire?(generation: number): void;
  onReply(listener: (reply: EncoderReply) => void): void;
  /** The window crashed or closed. */
  onGone(listener: (reason: string) => void): void;
  close(): void;
}

export type ExportEncodersOptions = {
  openWindow(): Promise<EncoderWindow>;
  /** Writes the muxed file over the export's temporary output. */
  writeOutput(path: string, bytes: Uint8Array): Promise<void>;
  /** Test-only capture read from the actual Chromium encoder canvas. */
  capture?: (rgba: Uint8Array) => Promise<void>;
  profile?: (timings: Record<string, number>) => void;
  closeMs?: number;
  idleMs?: number;
  schedule?: (fn: () => void, ms: number) => { cancel(): void };
};

/** What main knows about the step's sender. */
export type EncodeContext = {
  /** The engine launch, port and session the step came from. */
  owner: string;
  /** The temporary output an `outputWrite` handle of this job names. */
  resolveOutput(handle: CapabilityHandle, job: string): string;
  /** Maps, copies, and unlinks a shared-memory frame (pm-bridge). */
  takePixels: ((name: string, length: number) => Uint8Array) | null;
};

type Timer = { cancel(): void };
type Waiting = { resolve(reply: EncoderReply): void; reject(error: Error): void };
type Session = {
  key: string; id: string; output: string; frameBytes: number; busy: number; idle: Timer | null;
  /** Frame and audio steps the window has not answered yet. */
  inFlight: Promise<void>[];
  /** The first failure of an unanswered step, thrown at the next step. */
  failure: Error | null;
  capture: Promise<void>;
  timings: Record<string, number>;
};

/** Frame and audio steps answered before the window answers them, so the
 *  engine renders the next frame while the window encodes this one. At most
 *  this many wait in the window (four 1080p frames: 33 MB). */
export const MAX_IN_FLIGHT = 4;
type Live = { window: EncoderWindow; seq: number; waiting: Map<number, Waiting>; gone: boolean };

const defaultSchedule = (fn: () => void, ms: number): Timer => {
  const t = setTimeout(fn, ms);
  return { cancel: () => clearTimeout(t) };
};

let nextId = 0;

export class ExportEncoders {
  private readonly sessions = new Map<string, Session>();
  private cpuFrames = 0;
  private gpuFrames = 0;
  get frames(): { cpu: number; gpu: number } { return { cpu: this.cpuFrames, gpu: this.gpuFrames }; }
  private live: Promise<Live> | null = null;
  private receiverWindow: EncoderWindow | null = null;
  private closeTimer: Timer | null = null;
  private readonly schedule: (fn: () => void, ms: number) => Timer;
  private readonly closeMs: number;
  private readonly idleMs: number;

  constructor(private readonly options: ExportEncodersOptions) {
    this.schedule = options.schedule ?? defaultSchedule;
    this.closeMs = options.closeMs ?? 15_000;
    this.idleMs = options.idleMs ?? 300_000;
  }

  receiver(): Receiver | null { return this.receiverWindow?.receiver ?? null; }

  retireSurface(generation: number): void { this.receiverWindow?.retire?.(generation); }

  /** Encodes in flight (tests and diagnostics). */
  get active(): number {
    return this.sessions.size;
  }

  /** One `hostEncode` step; resolves when it is done (answer `ack`). */
  async serve(body: HostEncode, context: EncodeContext): Promise<void> {
    const key = `${context.owner}/${body.job}`;
    if (body.step === 'open') {
      if (this.sessions.has(key)) throw new ProtocolError('invalidMessage', 'This export is already encoding.');
      const output = context.resolveOutput(body.output, body.job);
      const session: Session = {
        key, id: `e${++nextId}`, output, frameBytes: body.width * body.height * 4, busy: 0, idle: null, inFlight: [], failure: null, capture: Promise.resolve(), timings: {}
      };
      this.sessions.set(key, session);
      this.closeTimer?.cancel();
      this.closeTimer = null;
      try {
        await this.command(session, {
          type: 'open', seq: 0, id: session.id, codec: body.codec, width: body.width, height: body.height,
          fps: { num: body.frameRate.num, den: body.frameRate.den }, bitrate: body.bitrate, frames: body.frames,
          audio: body.audio ? { sampleRate: body.audio.sampleRate, channels: body.audio.channels } : null,
          capture: !!this.options.capture, profile: !!this.options.profile
        });
      } catch (error) {
        this.end(session);
        throw error;
      }
      return;
    }
    const session = this.sessions.get(key);
    if (body.step === 'abort') {
      if (session) await this.abort(session);
      return;
    }
    if (!session) throw new ProtocolError('invalidMessage', 'This export is not encoding.');
    try {
      switch (body.step) {
        case 'video': {
          this.cpuFrames++;
          const rgba = this.pixels(body.pixels, session.frameBytes, context);
          await this.pipelined(session, { type: 'video', seq: 0, id: session.id, index: body.index, rgba });
          return;
        }
        case 'videoSurface': {
          this.gpuFrames++;
          await this.pipelined(session, { type: 'videoSurface', seq: 0, id: session.id, index: body.index, generation: body.generation, poolIndex: body.poolIndex });
          return;
        }
        case 'audio': {
          const bytes = body.samples;
          // Copy into an aligned buffer: message bytes may start at any offset.
          const samples = new Float32Array(bytes.byteLength / 4);
          new Uint8Array(samples.buffer).set(bytes);
          await this.pipelined(session, { type: 'audio', seq: 0, id: session.id, samples });
          return;
        }
        case 'finish': {
          await this.drain(session);
          const reply = await this.command(session, { type: 'finish', seq: 0, id: session.id });
          if (reply.type !== 'file') throw new ProtocolError('exportFailed', 'The export encoder did not return a file.');
          this.end(session);
          const started = this.options.profile ? performance.now() : 0;
          await this.options.writeOutput(session.output, reply.bytes);
          this.options.profile?.({ ...session.timings, ...reply.profile, write_ms: performance.now() - started });
          return;
        }
      }
    } catch (error) {
      // The window dropped the encode with its error; so does main.
      this.end(session);
      throw error;
    }
  }

  /** Aborts every encode whose owner is `owner` or starts with `owner:`
   *  (an engine launch that ended, with all its sessions). */
  dropOwner(owner: string): void {
    for (const session of [...this.sessions.values()]) {
      if (session.key.startsWith(`${owner}/`) || session.key.startsWith(`${owner}:`)) void this.abort(session);
    }
  }

  dispose(): void {
    for (const session of [...this.sessions.values()]) this.end(session);
    this.closeTimer?.cancel();
    this.closeTimer = null;
    const live = this.live;
    this.live = null;
    void live?.then((l) => this.retire(l, 'the host is shutting down')).catch(() => undefined);
  }

  private pixels(payload: Extract<HostEncode, { step: 'video' }>['pixels'], expected: number, context: EncodeContext): Uint8Array {
    if (payload.kind === 'inline') {
      if (payload.bytes.byteLength !== expected) throw new ProtocolError('invalidMessage', `a frame of ${payload.bytes.byteLength} bytes; ${expected} expected`);
      return payload.bytes;
    }
    if (payload.length !== expected) throw new ProtocolError('invalidMessage', `a shared frame of ${payload.length} bytes; ${expected} expected`);
    if (!context.takePixels) throw new ProtocolError('unsupported', 'main cannot read shared frames without the frame bridge addon');
    const data = context.takePixels(payload.name, payload.length);
    if (data.byteLength !== expected) throw new ProtocolError('invalidMessage', `the shared frame held ${data.byteLength} of ${expected} bytes`);
    return data;
  }

  /** Sends a frame or audio step and answers it once at most
   *  `MAX_IN_FLIGHT` steps of the encode wait in the window. */
  private async pipelined(session: Session, command: EncoderCommand): Promise<void> {
    if (session.failure) throw session.failure;
    const sent = this.command(session, command).then(() => undefined, (error: unknown) => {
      session.failure ??= error instanceof Error ? error : new Error(String(error));
    });
    session.inFlight.push(sent);
    while (session.inFlight.length > MAX_IN_FLIGHT) {
      const started = this.options.profile ? performance.now() : 0;
      await session.inFlight.shift();
      if (this.options.profile) session.timings.backpressure_ms = (session.timings.backpressure_ms ?? 0) + performance.now() - started;
    }
    if (session.failure) throw session.failure;
  }

  /** Waits for every unanswered step of the encode. */
  private async drain(session: Session): Promise<void> {
    while (session.inFlight.length) await session.inFlight.shift();
    if (session.failure) throw session.failure;
  }

  private async abort(session: Session): Promise<void> {
    this.end(session);
    const live = await this.live?.catch(() => null);
    if (!live || live.gone) return;
    await this.send(live, { type: 'abort', seq: 0, id: session.id }).catch(() => undefined);
  }

  private end(session: Session): void {
    session.idle?.cancel();
    session.idle = null;
    if (this.sessions.get(session.key) !== session) return;
    this.sessions.delete(session.key);
    if (this.sessions.size === 0 && !this.closeTimer) {
      this.closeTimer = this.schedule(() => {
        this.closeTimer = null;
        if (this.sessions.size) return;
        const live = this.live;
        this.live = null;
        void live?.then((l) => this.retire(l, 'the export encoder closed')).catch(() => undefined);
      }, this.closeMs);
    }
  }

  private async command(session: Session, command: EncoderCommand): Promise<EncoderReply> {
    session.busy++;
    session.idle?.cancel();
    session.idle = null;
    try {
      let started = this.options.profile ? performance.now() : 0;
      const live = await this.window();
      if (this.options.profile) session.timings.window_wait_ms = (session.timings.window_wait_ms ?? 0) + performance.now() - started;
      if (this.sessions.get(session.key) !== session) throw new ProtocolError('cancelled', 'The export encode was abandoned.');
      started = this.options.profile ? performance.now() : 0;
      const reply = await this.send(live, command);
      if (this.options.profile) {
        const key = `${command.type}_roundtrip_ms`;
        session.timings[key] = (session.timings[key] ?? 0) + performance.now() - started;
      }
      if (reply.type === 'error') throw new ProtocolError('exportFailed', `The export encoder failed: ${reply.message}`);
      if (reply.type === 'ack' && reply.rgba && this.options.capture) {
        const rgba = reply.rgba;
        session.capture = session.capture.then(() => this.options.capture!(rgba));
        await session.capture;
      }
      return reply;
    } finally {
      session.busy--;
      if (!session.busy && this.sessions.get(session.key) === session) {
        session.idle = this.schedule(() => { if (!session.busy) void this.abort(session); }, this.idleMs);
      }
    }
  }

  private send(live: Live, command: EncoderCommand): Promise<EncoderReply> {
    if (live.gone) return Promise.reject(new ProtocolError('exportFailed', 'The export encoder window is gone.'));
    const seq = ++live.seq;
    return new Promise<EncoderReply>((resolve, reject) => {
      live.waiting.set(seq, { resolve, reject });
      try {
        live.window.post({ ...command, seq });
      } catch (error) {
        live.waiting.delete(seq);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  private window(): Promise<Live> {
    if (!this.live) {
      const opening = this.options.openWindow().then((window) => {
        this.receiverWindow = window;
        const live: Live = { window, seq: 0, waiting: new Map(), gone: false };
        window.onReply((reply) => {
          if (reply.type === 'ready') return;
          const waiting = live.waiting.get(reply.seq);
          if (!waiting) return;
          live.waiting.delete(reply.seq);
          waiting.resolve(reply);
        });
        window.onGone((reason) => {
          if (this.live === opening) this.live = null;
          this.retire(live, reason);
          // Every encode in the window is lost with it.
          for (const session of [...this.sessions.values()]) this.end(session);
        });
        return live;
      });
      opening.catch(() => { if (this.live === opening) this.live = null; });
      this.live = opening;
    }
    return this.live;
  }

  private retire(live: Live, reason: string): void {
    if (this.receiverWindow === live.window) this.receiverWindow = null;
    if (!live.gone) {
      live.gone = true;
      live.window.close();
    }
    for (const waiting of live.waiting.values()) waiting.reject(new ProtocolError('exportFailed', `The export encoder window is gone: ${reason}.`));
    live.waiting.clear();
  }
}
