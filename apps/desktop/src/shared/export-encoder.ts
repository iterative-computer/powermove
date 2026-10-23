/*
 * Main ↔ export encoder window messages (decision EXPORT-1; main/engine/
 * export-encoder.ts, renderer/src/export-encoder/main.ts).
 *
 * The job engine's WebM exports are encoded by Chromium's WebCodecs
 * encoders, as today's exporter encodes them. Main owns a hidden encoder
 * window and talks to it over one MessagePort: each command carries a
 * sequence number and is answered once, in order, by `ack`, `error`, or (for
 * `finish`) `file` with the muxed WebM.
 */

/** The IPC channel that hands the window its MessagePort (the preload
 *  repeats the literal: a sandboxed preload cannot import shared chunks). */
export const EXPORT_ENCODER_PORT = 'export-encoder:port';

export type EncoderCodec = 'vp9' | 'vp8';

export type EncoderCommand =
  | {
      type: 'open'; seq: number; id: string; codec: EncoderCodec; width: number; height: number;
      /** The export rate; the encoder uses `num / den` as today's `opts.fps`. */
      fps: { num: number; den: number }; bitrate: number; frames: number;
      audio: { sampleRate: number; channels: number } | null;
      capture?: boolean;
    }
  /** Top-down RGBA8, premultiplied, `width × height × 4` bytes. */
  | { type: 'video'; seq: number; id: string; index: number; rgba: Uint8Array }
  | { type: 'videoSurface'; seq: number; id: string; index: number; generation: number; poolIndex: number }
  /** Interleaved samples of the mix, in order. */
  | { type: 'audio'; seq: number; id: string; samples: Float32Array }
  | { type: 'finish'; seq: number; id: string }
  | { type: 'abort'; seq: number; id: string };

export type EncoderReply =
  | { type: 'ready' }
  | { type: 'ack'; seq: number; rgba?: Uint8Array }
  | { type: 'error'; seq: number; message: string }
  | { type: 'file'; seq: number; bytes: Uint8Array };
