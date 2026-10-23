import { describe, expect, it } from 'vitest';

import type { HostEncode } from '../../shared/engine-protocol';
import type { EncoderCommand, EncoderReply } from '../../shared/export-encoder';
import { ExportEncoders, MAX_IN_FLIGHT, type EncodeContext, type EncoderWindow } from './export-encoder';

/** A window that answers each command from `answer`, in order. */
class FakeWindow implements EncoderWindow {
  commands: EncoderCommand[] = [];
  closed = false;
  private replies: Array<(reply: EncoderReply) => void> = [];
  private gone: Array<(reason: string) => void> = [];
  constructor(private readonly answer: (command: EncoderCommand) => EncoderReply | null) {}
  post(command: EncoderCommand): void {
    this.commands.push(command);
    const reply = this.answer(command);
    if (reply) queueMicrotask(() => { for (const listener of this.replies) listener(reply); });
  }
  onReply(listener: (reply: EncoderReply) => void): void { this.replies.push(listener); }
  onGone(listener: (reason: string) => void): void { this.gone.push(listener); }
  crash(): void { for (const listener of this.gone) listener('the encoder process crashed'); }
  close(): void { this.closed = true; }
}

const ack = (command: EncoderCommand): EncoderReply => command.type === 'finish'
  ? { type: 'file', seq: command.seq, bytes: new Uint8Array([0x1a, 0x45, 0xdf, 0xa3]) }
  : { type: 'ack', seq: command.seq };

const HANDLE = '0'.repeat(32);
const open = (job = 'job-1'): HostEncode => ({
  job, step: 'open', codec: 'vp9', width: 2, height: 2, frameRate: { num: 30000, den: 1001 }, bitrate: 16_000_000,
  frames: 1, audio: { sampleRate: 48000, channels: 2 }, output: HANDLE
});

function harness(answer: (command: EncoderCommand) => EncoderReply | null = ack) {
  const windows: FakeWindow[] = [];
  const written: Array<{ path: string; bytes: Uint8Array }> = [];
  const timers: Array<{ fn: () => void; ms: number; cancelled: boolean }> = [];
  const encoders = new ExportEncoders({
    openWindow: async () => { const w = new FakeWindow(answer); windows.push(w); return w; },
    writeOutput: async (path, bytes) => { written.push({ path, bytes }); },
    schedule: (fn, ms) => { const t = { fn, ms, cancelled: false }; timers.push(t); return { cancel: () => { t.cancelled = true; } }; }
  });
  const taken: string[] = [];
  const context: EncodeContext = {
    owner: 'job:1:7:3',
    resolveOutput: (handle, job) => { expect(handle).toBe(HANDLE); return `/tmp/out-${job}.part`; },
    takePixels: (name, length) => { taken.push(name); return new Uint8Array(length).fill(9); }
  };
  return { encoders, windows, written, timers, context, taken };
}

describe('ExportEncoders', () => {
  it('runs an encode in order and writes the muxed file into the output', async () => {
    const { encoders, windows, written, context, taken, timers } = harness();
    await encoders.serve(open(), context);
    await encoders.serve({ job: 'job-1', step: 'video', index: 0, pixels: { kind: 'sharedMemory', name: '/pm.0123456789abcdef01234567', length: 16 } }, context);
    const samples = new Uint8Array(new Float32Array([0.5, -0.25]).buffer);
    // Message bytes may start at any offset.
    const unaligned = new Uint8Array(samples.length + 1).subarray(1);
    unaligned.set(samples);
    await encoders.serve({ job: 'job-1', step: 'audio', samples: unaligned }, context);
    await encoders.serve({ job: 'job-1', step: 'finish' }, context);
    const commands = windows[0]!.commands;
    expect(commands.map((c) => c.type)).toEqual(['open', 'video', 'audio', 'finish']);
    expect(commands[0]).toMatchObject({ codec: 'vp9', fps: { num: 30000, den: 1001 }, bitrate: 16_000_000, frames: 1, audio: { sampleRate: 48000, channels: 2 } });
    expect(Array.from((commands[1] as Extract<EncoderCommand, { type: 'video' }>).rgba)).toEqual(new Array(16).fill(9));
    expect(Array.from((commands[2] as Extract<EncoderCommand, { type: 'audio' }>).samples)).toEqual([0.5, -0.25]);
    expect(taken).toEqual(['/pm.0123456789abcdef01234567']);
    expect(written).toEqual([{ path: '/tmp/out-job-1.part', bytes: new Uint8Array([0x1a, 0x45, 0xdf, 0xa3]) }]);
    expect(encoders.active).toBe(0);
    // The window closes once no encode needs it.
    timers.filter((t) => !t.cancelled && t.ms === 15_000).forEach((t) => t.fn());
    await Promise.resolve();
    await Promise.resolve();
    expect(windows[0]!.closed).toBe(true);
  });

  it('rejects frames of the wrong size and steps of encodes it does not have', async () => {
    const { encoders, context } = harness();
    await expect(encoders.serve({ job: 'job-2', step: 'finish' }, context)).rejects.toThrow('not encoding');
    await encoders.serve(open('job-2'), context);
    await expect(encoders.serve({ job: 'job-2', step: 'video', index: 0, pixels: { kind: 'inline', bytes: new Uint8Array(15) } }, context))
      .rejects.toThrow('15 bytes; 16 expected');
    expect(encoders.active).toBe(0);
  });

  it('surfaces the window\'s errors at the next step and drops the encode', async () => {
    const { encoders, context, written } = harness((command) => command.type === 'video'
      ? { type: 'error', seq: command.seq, message: 'VideoEncoder: Encoding error.' } : ack(command));
    await encoders.serve(open(), context);
    // Answered before the window: the error fails the next step.
    await encoders.serve({ job: 'job-1', step: 'video', index: 0, pixels: { kind: 'inline', bytes: new Uint8Array(16) } }, context);
    await expect(encoders.serve({ job: 'job-1', step: 'finish' }, context))
      .rejects.toThrow('The export encoder failed: VideoEncoder: Encoding error.');
    await expect(encoders.serve({ job: 'job-1', step: 'finish' }, context)).rejects.toThrow('not encoding');
    expect(written).toEqual([]);
  });

  it('keeps at most MAX_IN_FLIGHT frames waiting in the window', async () => {
    // The window never answers frames.
    const { encoders, context, windows } = harness((command) => command.type === 'video' ? null : ack(command));
    await encoders.serve(open(), context);
    const frame = (index: number) => encoders.serve({ job: 'job-1', step: 'video', index, pixels: { kind: 'inline', bytes: new Uint8Array(16) } }, context);
    for (let i = 0; i < MAX_IN_FLIGHT; i++) await frame(i);
    let fifth = false;
    void frame(MAX_IN_FLIGHT).then(() => { fifth = true; });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(windows[0]!.commands.filter((c) => c.type === 'video')).toHaveLength(MAX_IN_FLIGHT + 1);
    expect(fifth).toBe(false);
  });

  it('fails every waiting step when the window goes, and opens a new one for the next encode', async () => {
    const { encoders, context, windows } = harness((command) => command.type === 'finish' ? null : ack(command));
    await encoders.serve(open(), context);
    const pending = encoders.serve({ job: 'job-1', step: 'finish' }, context);
    await new Promise((resolve) => setTimeout(resolve, 0));
    windows[0]!.crash();
    await expect(pending).rejects.toThrow('window is gone');
    expect(encoders.active).toBe(0);
    await encoders.serve(open('job-3'), context);
    expect(windows).toHaveLength(2);
  });

  it('aborts the encodes of an engine launch that ended', async () => {
    const { encoders, context, windows } = harness();
    await encoders.serve(open('job-4'), context);
    await encoders.serve(open('job-5'), { ...context, owner: 'job:2:8:3' });
    encoders.dropOwner('job:1:7');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(windows[0]!.commands.filter((c) => c.type === 'abort').map((c) => c.id)).toHaveLength(1);
    expect(encoders.active).toBe(1);
  });

  it('aborts an encode no step reaches', async () => {
    const { encoders, context, timers, windows } = harness();
    await encoders.serve(open('job-6'), context);
    const idle = timers.find((t) => t.ms === 300_000 && !t.cancelled)!;
    idle.fn();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(windows[0]!.commands.at(-1)?.type).toBe('abort');
    expect(encoders.active).toBe(0);
  });
});
