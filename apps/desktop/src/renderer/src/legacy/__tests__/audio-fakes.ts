import { expect, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from './make-pm';

export class FakeAudioBuffer {
  private readonly channels: Float32Array[];
  numberOfChannels: number;
  length: number;
  sampleRate: number;
  duration: number;

  constructor(channels: ArrayLike<number>[], sampleRate = 8) {
    this.channels = channels.map((channel) => Float32Array.from(channel));
    this.numberOfChannels = this.channels.length;
    this.length = this.channels[0]?.length ?? 0;
    this.sampleRate = sampleRate;
    this.duration = this.length / sampleRate;
    expect(this.channels.every((channel) => channel.length === this.length)).toBe(true);
  }

  getChannelData(channel: number): Float32Array {
    return this.channels[channel]!;
  }
}

export class FakeAudioParam {
  value: number;
  cancellations: number[] = [];
  curves: Array<{ values: number[]; at: number; duration: number }> = [];
  values: Array<{ value: number; at: number }> = [];
  ramps: Array<{ value: number; at: number }> = [];
  events: Array<Record<string, any>> = [];

  constructor(value = 1) { this.value = value; }

  cancelScheduledValues(at: number): void {
    this.cancellations.push(at);
    this.events.push({ method: 'cancel', at });
  }

  setValueCurveAtTime(curve: ArrayLike<number>, at: number, duration: number): void {
    const values = Array.from(curve);
    this.curves.push({ values, at, duration });
    this.events.push({ method: 'curve', values, at, duration });
    this.value = values.at(-1) ?? this.value;
  }

  setValueAtTime(value: number, at: number): void {
    this.values.push({ value, at });
    this.events.push({ method: 'set', value, at });
    this.value = value;
  }

  linearRampToValueAtTime(value: number, at: number): void {
    this.ramps.push({ value, at });
    this.events.push({ method: 'linear', value, at });
    this.value = value;
  }
}

export class FakeGainNode {
  gain = new FakeAudioParam();
  connectedTo: any = null;
  disconnectCalls = 0;

  connect(target: any): any { this.connectedTo = target; return target; }
  disconnect(): void { this.disconnectCalls++;
  }
}

export class FakeBufferSource {
  buffer: any = null;
  connectedTo: any = null;
  starts: Array<{ when: number; offset: number; duration: number }> = [];
  stopCalls = 0;
  disconnectCalls = 0;
  onended: (() => void) | null = null;

  connect(target: any): any { this.connectedTo = target; return target; }
  start(when: number, offset: number, duration: number): void { this.starts.push({ when, offset, duration }); }
  stop(): void { this.stopCalls++; }
  disconnect(): void { this.disconnectCalls++; }
}

export interface Deferred<T> {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(error: unknown): void;
}

export function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

export async function flush(count = 1): Promise<void> {
  for (let index = 0; index < count; index++) {
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  }
}

interface AudioHarnessOptions {
  decodedBuffer?: any;
  decode?: (bytes: ArrayBuffer) => any;
  currentTime?: number;
}

export function audioHarness(options: AudioHarnessOptions = {}) {
  const listeners = new Map<string, Array<(value: unknown) => void>>();
  const events: Array<[string, unknown]> = [];
  const invalidations: string[] = [];
  const toasts: string[] = [];
  const contexts: any[] = [];
  const offlineContexts: any[] = [];
  const decodeCalls: ArrayBuffer[] = [];
  const assets = new Map<string, any>();

  class AudioContext {
    currentTime = options.currentTime ?? 0.25;
    state = 'running';
    destination = { kind: 'destination' };
    sources: FakeBufferSource[] = [];
    gains: FakeGainNode[] = [];
    resumeCalls = 0;

    constructor() { contexts.push(this); }
    createBufferSource(): FakeBufferSource { const source = new FakeBufferSource(); this.sources.push(source); return source; }
    createGain(): FakeGainNode { const gain = new FakeGainNode(); this.gains.push(gain); return gain; }
    createMediaStreamDestination(): any {
      const track = { stopped: false, stop() { this.stopped = true; } };
      return { track, stream: { getAudioTracks: () => [track], getTracks: () => [track] } };
    }
    resume(): Promise<void> { this.resumeCalls++; this.state = 'running'; return Promise.resolve(); }
    decodeAudioData(bytes: ArrayBuffer, success?: (buffer: any) => void, failure?: (error: unknown) => void): Promise<any> {
      decodeCalls.push(bytes);
      const pending = Promise.resolve().then(() => options.decode ? options.decode(bytes) : options.decodedBuffer);
      pending.then((value) => success?.(value), (error) => failure?.(error));
      return pending;
    }
  }

  class OfflineAudioContext {
    destination = { kind: 'offline-destination' };
    sources: FakeBufferSource[] = [];
    gains: FakeGainNode[] = [];
    rendered: FakeAudioBuffer;

    constructor(public numberOfChannels: number, public length: number, public sampleRate: number) {
      this.rendered = new FakeAudioBuffer(
        Array.from({ length: numberOfChannels }, () => new Float32Array(length)), sampleRate,
      );
      offlineContexts.push(this);
    }
    createBufferSource(): FakeBufferSource { const source = new FakeBufferSource(); this.sources.push(source); return source; }
    createGain(): FakeGainNode { const gain = new FakeGainNode(); this.gains.push(gain); return gain; }
    startRendering(): Promise<FakeAudioBuffer> { return Promise.resolve(this.rendered); }
  }

  const document = { documentElement: { dataset: {} as Record<string, string> } };
  const windowObject: any = { AudioContext, OfflineAudioContext, document };
  vi.stubGlobal('window', windowObject);
  vi.stubGlobal('document', document);

  const PM = makePM('core/audio') as PMRegistry & Record<string, any>;
  PM.proj = { dur: 12, fps: 30, work: [0, 12], layers: [], comps: {}, assets: {} };
  PM.assets = { map: assets, get(id: string) { return assets.get(id); } };
  PM.time = 0;
  PM.playing = false;
  PM.bus = {
    on(name: string, handler: (value: unknown) => void) {
      const handlers = listeners.get(name) ?? [];
      handlers.push(handler);
      listeners.set(name, handlers);
    },
    emit(name: string, value: unknown) {
      events.push([name, value]);
      for (const handler of listeners.get(name) ?? []) handler(value);
    },
  };
  PM.invalidate = (name: string) => invalidations.push(name);
  PM.toast = (message: string) => toasts.push(message);
  windowObject.PM = PM;

  return { PM, assets, contexts, offlineContexts, decodeCalls, events, invalidations, toasts, document, window: windowObject };
}

export function audioAsset(buffer: any, id = 'asset-1'): any {
  return {
    id, name: `${id}.wav`, kind: 'audio', dur: buffer.duration,
    channels: buffer.numberOfChannels, sampleRate: buffer.sampleRate,
    audioBlob: new Blob([new Uint8Array([82, 73, 70, 70])], { type: 'audio/wav' }),
    audioBuffer: buffer, audioDecoding: null, audioDisposed: false,
    audioToken: Symbol(id), peaks: null,
  };
}

export function audioLayer(overrides: Record<string, any> = {}): any {
  const data = { asset: 'asset-1', trim: 0, gain: 1, fadeIn: 0, fadeOut: 0, ...(overrides.d ?? {}) };
  return {
    id: 'audio-1', name: 'Audio', type: 'audio', on: true,
    from: 0, dur: 4, ...overrides, d: data,
  };
}
