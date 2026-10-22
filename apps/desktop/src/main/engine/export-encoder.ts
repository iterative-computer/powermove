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
 * One encode per job and engine port. Steps arrive in order and each is
 * answered after the window answered it, so a slow encoder slows the engine
 * rather than buffering frames here. An encode no step reaches for
 * `idleMs` while nothing is in flight is aborted (the engine that asked is
 * gone); the window closes `closeMs` after its last encode ends and is
 * reopened by the next.
 */
import type { CapabilityHandle, HostEncode } from '../../shared/engine-protocol';
import type { EncoderCommand, EncoderReply } from '../../shared/export-encoder';
import { ProtocolError } from './protocol';

/** The encoder window as main drives it. */
export interface EncoderWindow {
  post(command: EncoderCommand): void;
  onReply(listener: (reply: EncoderReply) => void): void;
  /** The window crashed or closed. */
  onGone(listener: (reason: string) => void): void;
  close(): void;
}

export type ExportEncodersOptions = {
  openWindow(): Promise<EncoderWindow>;
  /** Writes the muxed file over the export's temporary output. */
  writeOutput(path: string, bytes: Uint8Array): Promise<void>;
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
type Session = { key: string; id: string; output: string; frameBytes: number; busy: number; idle: Timer | null };
type Live = { window: EncoderWindow; seq: number; waiting: Map<number, Waiting>; gone: boolean };

const defaultSchedule = (fn: () => void, ms: number): Timer => {
  const t = setTimeout(fn, ms);
  return { cancel: () => clearTimeout(t) };
};

let nextId = 0;

export class ExportEncoders {
  private readonly sessions = new Map<string, Session>();
  private live: Promise<Live> | null = null;
  private closeTimer: Timer | null = null;
  private readonly schedule: (fn: () => void, ms: number) => Timer;
  private readonly closeMs: number;
  private readonly idleMs: number;

  constructor(private readonly options: ExportEncodersOptions) {
    this.schedule = options.schedule ?? defaultSchedule;
    this.closeMs = options.closeMs ?? 15_000;
    this.idleMs = options.idleMs ?? 300_000;
  }

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
      const session: Session = { key, id: `e${++nextId}`, output, frameBytes: body.width * body.height * 4, busy: 0, idle: null };
      this.sessions.set(key, session);
      this.closeTimer?.cancel();
      this.closeTimer = null;
      try {
        await this.command(session, {
          type: 'open', seq: 0, id: session.id, codec: body.codec, width: body.width, height: body.height,
          fps: { num: body.frameRate.num, den: body.frameRate.den }, bitrate: body.bitrate, frames: body.frames,
          audio: body.audio ? { sampleRate: body.audio.sampleRate, channels: body.audio.channels } : null
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
          const rgba = this.pixels(body.pixels, session.frameBytes, context);
          await this.command(session, { type: 'video', seq: 0, id: session.id, index: body.index, rgba });
          return;
        }
        case 'audio': {
          const bytes = body.samples;
          // Copy into an aligned buffer: message bytes may start at any offset.
          const samples = new Float32Array(bytes.byteLength / 4);
          new Uint8Array(samples.buffer).set(bytes);
          await this.command(session, { type: 'audio', seq: 0, id: session.id, samples });
          return;
        }
        case 'finish': {
          const reply = await this.command(session, { type: 'finish', seq: 0, id: session.id });
          if (reply.type !== 'file') throw new ProtocolError('exportFailed', 'The export encoder did not return a file.');
          this.end(session);
          await this.options.writeOutput(session.output, reply.bytes);
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
      const live = await this.window();
      if (this.sessions.get(session.key) !== session) throw new ProtocolError('cancelled', 'The export encode was abandoned.');
      const reply = await this.send(live, command);
      if (reply.type === 'error') throw new ProtocolError('exportFailed', `The export encoder failed: ${reply.message}`);
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
    if (!live.gone) {
      live.gone = true;
      live.window.close();
    }
    for (const waiting of live.waiting.values()) waiting.reject(new ProtocolError('exportFailed', `The export encoder window is gone: ${reason}.`));
    live.waiting.clear();
  }
}
