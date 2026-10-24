/*
 * The export encoder window (decision EXPORT-1; main/engine/export-encoder.ts).
 *
 * Today's exporter writes WebM by drawing each frame into the compositor's
 * WebGL2 canvas, handing the canvas to a WebCodecs VideoEncoder, encoding
 * the OfflineAudioContext mix with an AudioEncoder, and muxing the chunks
 * with its own EBML writer. The job engine renders and mixes instead; this
 * page runs the rest exactly as today's exporter does: the engine's canvas
 * bytes are put into a WebGL2 canvas with the compositor's context
 * attributes (so `new VideoFrame(canvas)` takes the same GPU readback to
 * REC709 I420), and the same encoder settings (webm-video.ts), Opus
 * encoding (opus-encode.ts), and muxer (webm-mux.ts) follow.
 */
import type { EncoderCommand, EncoderReply } from '../../../shared/export-encoder';
import { EXPORT_ENCODER_PORT } from '../../../shared/export-encoder';
import { encodeOpus } from '../legacy/core/opus-encode';
import { PM_VERSION } from '../legacy/core/version';
import { muxWebM } from '../legacy/core/webm-mux';
import {
  WEBM_ENCODE_QUEUE_LIMIT, WEBM_VIDEO_CODECS, webmFrameTiming, webmKeyFrame, webmVideoConfig, type WebmVideoCodec
} from '../legacy/core/webm-video';

type Open = Extract<EncoderCommand, { type: 'open' }>;
type Chunk = { ts: number; key: boolean; data: Uint8Array };

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** `GL.init`'s context attributes for the compositor canvas (legacy/gl/compositor.ts). */
export const COMPOSITOR_CONTEXT: WebGLContextAttributes = {
  alpha: false, antialias: false, premultipliedAlpha: true,
  preserveDrawingBuffer: true, powerPreference: 'high-performance', desynchronized: true,
};

class Encode {
  readonly fps: number;
  private readonly canvas: HTMLCanvasElement;
  private readonly gl: WebGL2RenderingContext;
  private readonly framebuffer: WebGLFramebuffer;
  private readonly texture: WebGLTexture;
  private encoder: any = null;
  private chosen: WebmVideoCodec | null = null;
  private readonly chunks: Chunk[] = [];
  private failure: Error | null = null;
  private next = 0;
  private readonly audio: Float32Array[] = [];
  private audioFrames = 0;

  constructor(readonly open: Open) {
    this.fps = open.fps.num / open.fps.den;
    this.canvas = document.createElement('canvas');
    this.canvas.width = open.width;
    this.canvas.height = open.height;
    const gl = this.canvas.getContext('webgl2', COMPOSITOR_CONTEXT);
    if (!gl) throw new Error('The export encoder needs WebGL2.');
    this.gl = gl;
    const texture = gl.createTexture(), framebuffer = gl.createFramebuffer();
    if (!texture || !framebuffer) throw new Error('The export encoder could not allocate its frame.');
    this.texture = texture;
    this.framebuffer = framebuffer;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, open.width, open.height);
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.READ_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    if (gl.checkFramebufferStatus(gl.READ_FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
      throw new Error('The export encoder frame is incomplete.');
    }
  }

  /** exporter.ts `exportWebCodecs`: VP9, else VP8, configured as today. */
  async start(): Promise<void> {
    const Encoder = (window as any).VideoEncoder;
    if (!Encoder) throw new Error('WebCodecs is unavailable.');
    this.encoder = new Encoder({
      output: (chunk: any) => {
        const data = new Uint8Array(chunk.byteLength);
        chunk.copyTo(data);
        this.chunks.push({ ts: chunk.timestamp, key: chunk.type === 'key', data });
      },
      error: (error: Error) => { this.failure = error; },
    });
    const candidates = this.open.codec === 'vp9' ? WEBM_VIDEO_CODECS : WEBM_VIDEO_CODECS.filter((c) => c.id === 'V_VP8');
    for (const candidate of candidates) {
      const config = webmVideoConfig(candidate.codec, this.open.width, this.open.height, this.open.bitrate, this.fps);
      const support = await Encoder.isConfigSupported(config).catch(() => null);
      if (support?.supported) { this.encoder.configure(config); this.chosen = candidate; return; }
    }
    throw new Error(`No supported ${this.open.codec === 'vp9' ? 'VP9/VP8' : 'VP8'} encoder`);
  }

  async video(index: number, rgba: Uint8Array): Promise<void> {
    if (this.failure) throw this.failure;
    if (index !== this.next) throw new Error(`frame ${index} arrived, frame ${this.next} expected`);
    const { width, height } = this.open;
    if (rgba.byteLength !== width * height * 4) throw new Error(`a ${rgba.byteLength}-byte frame for ${width}×${height}`);
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, this.framebuffer);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
    // Row 0 is the top: the canvas shows its framebuffer bottom-up.
    gl.blitFramebuffer(0, 0, width, height, 0, height, width, 0, gl.COLOR_BUFFER_BIT, gl.NEAREST);
    const frame = new (window as any).VideoFrame(this.canvas, webmFrameTiming(index, this.fps));
    try { this.encoder.encode(frame, { keyFrame: webmKeyFrame(index, this.fps) }); } finally { frame.close(); }
    this.next++;
    while (this.encoder.encodeQueueSize > WEBM_ENCODE_QUEUE_LIMIT && !this.failure) await sleep(6);
    if (this.failure) throw this.failure;
  }

  audioChunk(samples: Float32Array): void {
    const channels = this.open.audio?.channels;
    if (!channels) throw new Error('This export has no audio track.');
    if (samples.length % channels) throw new Error('Audio samples are not whole frames.');
    this.audio.push(samples);
    this.audioFrames += samples.length / channels;
  }

  /** Today's order: flush the frames, encode the whole mix, mux. */
  async finish(): Promise<Uint8Array> {
    if (this.next !== this.open.frames) throw new Error(`${this.next} frames encoded, ${this.open.frames} expected`);
    await this.encoder.flush();
    if (this.failure) throw this.failure;
    this.encoder.close();
    let audio: any = null;
    const spec = this.open.audio;
    if (spec && this.audioFrames > 0) {
      const buffer = new AudioBuffer({ length: this.audioFrames, numberOfChannels: spec.channels, sampleRate: spec.sampleRate });
      const planes = Array.from({ length: spec.channels }, (_, c) => buffer.getChannelData(c));
      let at = 0;
      for (const chunk of this.audio) {
        const frames = chunk.length / spec.channels;
        for (let i = 0; i < frames; i++) for (let c = 0; c < spec.channels; c++) planes[c]![at + i] = chunk[i * spec.channels + c]!;
        at += frames;
      }
      this.audio.length = 0;
      audio = await encodeOpus(buffer);
      if (!audio) throw new Error('Could not encode the audio track');
    }
    const blob: Blob = muxWebM(this.chunks, {
      width: this.open.width, height: this.open.height, fps: this.fps, codecId: this.chosen!.id, audio,
    }, PM_VERSION);
    return new Uint8Array(await blob.arrayBuffer());
  }

  close(): void {
    if (this.encoder && this.encoder.state !== 'closed') this.encoder.close();
    this.gl.deleteFramebuffer(this.framebuffer);
    this.gl.deleteTexture(this.texture);
    this.gl.getExtension('WEBGL_lose_context')?.loseContext();
    this.canvas.width = this.canvas.height = 0;
    this.chunks.length = 0;
    this.audio.length = 0;
  }
}

function serve(port: MessagePort): void {
  const encodes = new Map<string, Encode>();
  const reply = (message: EncoderReply) => port.postMessage(message);
  let queue = Promise.resolve();
  const run = async (command: EncoderCommand): Promise<void> => {
    try {
      switch (command.type) {
        case 'open': {
          if (encodes.has(command.id)) throw new Error('This export is already encoding.');
          const encode = new Encode(command);
          try { await encode.start(); } catch (error) { encode.close(); throw error; }
          encodes.set(command.id, encode);
          reply({ type: 'ack', seq: command.seq });
          return;
        }
        case 'video': {
          const encode = encodes.get(command.id);
          if (!encode) throw new Error('This export is not encoding.');
          await encode.video(command.index, command.rgba);
          reply({ type: 'ack', seq: command.seq });
          return;
        }
        case 'audio': {
          const encode = encodes.get(command.id);
          if (!encode) throw new Error('This export is not encoding.');
          encode.audioChunk(command.samples);
          reply({ type: 'ack', seq: command.seq });
          return;
        }
        case 'finish': {
          const encode = encodes.get(command.id);
          if (!encode) throw new Error('This export is not encoding.');
          encodes.delete(command.id);
          try {
            const bytes = await encode.finish();
            // Copied, not transferred: a MessagePortMain takes no transferred buffers.
            reply({ type: 'file', seq: command.seq, bytes });
          } finally { encode.close(); }
          return;
        }
        case 'abort': {
          encodes.get(command.id)?.close();
          encodes.delete(command.id);
          reply({ type: 'ack', seq: command.seq });
          return;
        }
      }
    } catch (error) {
      const encode = encodes.get(command.id);
      if (encode) { encode.close(); encodes.delete(command.id); }
      reply({ type: 'error', seq: command.seq, message: error instanceof Error ? error.message : String(error) });
    }
  };
  // One command at a time, in arrival order: frames reach the encoder in order.
  port.onmessage = (event: MessageEvent<EncoderCommand>) => { queue = queue.then(() => run(event.data)); };
  reply({ type: 'ready' });
}

window.addEventListener('message', (event) => {
  // Only this page's preload posts it; the page has no frames.
  if (event.data === EXPORT_ENCODER_PORT && event.ports[0]) serve(event.ports[0]);
});
