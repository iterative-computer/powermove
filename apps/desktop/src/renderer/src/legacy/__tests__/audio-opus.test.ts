import { afterEach, describe, expect, it, vi } from 'vitest';
import { audioHarness, FakeAudioBuffer, flush } from './audio-fakes';

afterEach(() => vi.unstubAllGlobals());

function opusHarness(options: { failure?: 'configure' | 'data' | 'encode' | 'flush'; unsupported?: boolean;
  registrationDrain?: boolean; closeOnError?: boolean } = {}) {
  const h = audioHarness();
  const failure = new Error(`${options.failure || 'codec'} failed`);
  const frames: Frame[] = [], encoders: Encoder[] = [];
  class Frame {
    closed = 0;
    samples: Float32Array;
    constructor(readonly init: any) {
      if (options.failure === 'data') throw failure;
      this.samples = new Float32Array(init.data);
      frames.push(this);
    }
    close() { this.closed++; }
  }
  class Encoder extends EventTarget {
    static async isConfigSupported() { return { supported: !options.unsupported }; }
    state = 'unconfigured'; encodeQueueSize = 0; maximum = 0; closes = 0; flushes = 0;
    pending: Frame[] = []; inputs: Frame[] = []; listeners = new Set<any>();
    constructor(readonly callbacks: any) { super(); encoders.push(this); }
    configure() { if (options.failure === 'configure') throw failure; this.state = 'configured'; }
    encode(frame: Frame) {
      if (options.failure === 'encode') throw failure;
      this.pending.push(frame); this.inputs.push(frame);
      this.maximum = Math.max(this.maximum, ++this.encodeQueueSize);
    }
    drain(count = 1, notify = true) {
      for (let index = 0; index < count && this.pending.length; index++) {
        const frame = this.pending.shift()!; this.encodeQueueSize--;
        this.callbacks.output({ timestamp: frame.init.timestamp, byteLength: 2,
          copyTo: (output: Uint8Array) => output.set([this.inputs.indexOf(frame), frame.init.numberOfFrames & 255]) },
        { decoderConfig: { description: new Uint8Array([79, 112, 117, 115]).buffer } });
      }
      if (notify) this.dispatchEvent(new Event('dequeue'));
    }
    async flush() {
      this.flushes++;
      if (options.failure === 'flush' || this.state === 'closed') throw failure;
      this.drain(this.pending.length);
    }
    close() { this.closes++; this.state = 'closed'; this.pending = []; this.encodeQueueSize = 0; }
    fail() {
      if (options.closeOnError) { this.state = 'closed'; this.pending = []; this.encodeQueueSize = 0; }
      this.callbacks.error(failure);
    }
    addEventListener(...args: any[]) {
      if (args[0] === 'dequeue') {
        // Model capacity becoming available before the listener is attached,
        // with no later event to rescue a missed notification.
        if (options.registrationDrain) this.drain(this.pending.length, false);
        this.listeners.add(args[1]);
      }
      super.addEventListener(args[0], args[1], args[2]);
    }
    removeEventListener(...args: any[]) {
      if (args[0] === 'dequeue') this.listeners.delete(args[1]);
      super.removeEventListener(args[0], args[1], args[2]);
    }
  }
  h.window.AudioEncoder = Encoder; h.window.AudioData = Frame;
  const source = new FakeAudioBuffer([
    Float32Array.from({ length: 48000 + 311 }, (_, i) => (i % 101) / 101),
    Float32Array.from({ length: 48000 + 311 }, (_, i) => -(i % 53) / 53),
  ], 48000);
  return { ...h, source, frames, encoders, failure };
}

describe('Opus export capacity and cleanup', () => {
  it('waits for actual capacity, bounds queued packets, and preserves planar samples and timestamps', async () => {
    const h = opusHarness();
    let settled = false;
    const encoding = h.PM.Audio.encodeOpus(h.source).finally(() => { settled = true; });
    await flush();
    const encoder = h.encoders[0]!;
    expect(encoder.encodeQueueSize).toBe(17);
    await flush(8);
    expect(encoder.encodeQueueSize).toBe(17);
    expect(encoder.inputs).toHaveLength(17);
    expect(settled).toBe(false);
    for (let iteration = 0; iteration < 100 && !settled; iteration++) {
      encoder.drain(); await Promise.resolve(); await Promise.resolve();
    }
    const result = await encoding;
    expect(encoder.maximum).toBe(17);
    expect(result.chunks).toHaveLength(51);
    expect([...result.priv]).toEqual([79, 112, 117, 115]);
    expect(result).toMatchObject({ rate: 48000, channels: 2 });
    for (const [packet, frame] of encoder.inputs.entries()) {
      const offset = packet * 960, length = Math.min(960, h.source.length - offset);
      expect(frame.init).toMatchObject({format: 'f32-planar', numberOfFrames: length, numberOfChannels: 2,
        timestamp: Math.round(offset / 48000 * 1e6)});
      for (let channel = 0; channel < 2; channel++) {
        expect(frame.samples[channel * length]).toBe(h.source.getChannelData(channel)[offset]);
        expect(frame.samples[(channel + 1) * length - 1]).toBe(h.source.getChannelData(channel)[offset + length - 1]);
      }
      expect(result.chunks[packet].ts).toBe(frame.init.timestamp);
    }
    expect(h.frames.every(frame => frame.closed === 1)).toBe(true);
    expect(encoder.closes).toBe(1);
    expect(encoder.listeners.size).toBe(0);
  });

  it('checks capacity again after listener registration so an earlier dequeue cannot hang export', async () => {
    const h = opusHarness({registrationDrain: true});
    const result = await h.PM.Audio.encodeOpus(h.source);
    expect(result.chunks).toHaveLength(51);
    expect(h.encoders[0]!.maximum).toBe(17);
    expect(h.encoders[0]!.listeners.size).toBe(0);
    expect(h.encoders[0]!.closes).toBe(1);
  });

  it.each([false, true])('wakes a stalled queue on codec error and retains flush outcome, codec closes=%s', async closeOnError => {
    const h = opusHarness({closeOnError});
    const encoding = h.PM.Audio.encodeOpus(h.source);
    const result = encoding.then((value: any) => ({value}), (error: Error) => ({error}));
    await flush();
    const encoder = h.encoders[0]!;
    expect(encoder.listeners.size).toBe(1);
    encoder.fail();
    if (closeOnError) expect(await result).toEqual({error: h.failure});
    else expect(await result).toEqual({value: null});
    expect(encoder.inputs).toHaveLength(17);
    expect(encoder.flushes).toBe(1);
    expect(encoder.listeners.size).toBe(0);
    expect(encoder.closes).toBe(closeOnError ? 0 : 1);
    expect(h.frames.every(frame => frame.closed === 1)).toBe(true);
  });

  it.each(['configure', 'data', 'encode', 'flush'] as const)('releases the codec and input frames after a %s failure', async failure => {
    const h = opusHarness({failure, registrationDrain: true});
    await expect(h.PM.Audio.encodeOpus(h.source)).rejects.toBe(h.failure);
    expect(h.encoders[0]!.closes).toBe(1);
    expect(h.encoders[0]!.listeners.size).toBe(0);
    expect(h.frames.every(frame => frame.closed === 1)).toBe(true);
  });

  it('keeps unsupported-codec results null without allocating a codec or input frame', async () => {
    const h = opusHarness({unsupported: true});
    expect(await h.PM.Audio.encodeOpus(h.source)).toBeNull();
    expect(h.encoders).toHaveLength(0);
    expect(h.frames).toHaveLength(0);
  });
});
