import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './audio';

/* A recording Web Audio graph: enough to see routing, envelopes and taps. */
class FakeParam {
  value: number;
  events: Array<[string, number]> = [];
  constructor(value = 1) { this.value = value; }
  setValueAtTime(value: number) { this.events.push(['set', value]); this.value = value; }
  linearRampToValueAtTime(value: number) { this.events.push(['ramp', value]); this.value = value; }
  setTargetAtTime(value: number) { this.events.push(['target', value]); this.value = value; }
  cancelScheduledValues() { this.events.push(['cancel', 0]); }
}

class FakeNode {
  outputs: FakeNode[] = [];
  constructor(public kind: string) {}
  connect(node: FakeNode) { this.outputs.push(node); return node; }
  disconnect(node?: FakeNode) { this.outputs = node ? this.outputs.filter((item) => item !== node) : []; }
}

class FakeGain extends FakeNode {
  gain = new FakeParam(1);
  channelCount = 2; channelCountMode = 'max'; channelInterpretation = 'speakers';
  constructor() { super('gain'); }
}

class FakeSource extends FakeNode {
  buffer: unknown = null;
  started: number[] | null = null;
  stopped = false;
  onended: (() => void) | null = null;
  constructor() { super('source'); }
  start(...args: number[]) { this.started = args; }
  stop() { this.stopped = true; }
}

class FakeAnalyser extends FakeNode {
  fftSize = 2048; smoothingTimeConstant = 0.8; level = 0;
  constructor() { super('analyser'); }
  getFloatTimeDomainData(buffer: Float32Array) { for (let i = 0; i < buffer.length; i++) buffer[i] = i % 2 ? -this.level : this.level * 0.5; }
}

function fakeContext() {
  const created: FakeNode[] = [];
  const track = <T extends FakeNode>(node: T) => { created.push(node); return node; };
  const ctx = {
    currentTime: 1,
    state: 'running',
    destination: new FakeNode('destination'),
    createGain: () => track(new FakeGain()),
    createBufferSource: () => track(new FakeSource()),
    createAnalyser: () => track(new FakeAnalyser()),
    createChannelSplitter: () => track(new FakeNode('splitter')),
    resume: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    created
  };
  return ctx;
}

const asset = { id: 'tone', kind: 'audio', dur: 20, audioBlob: {}, audioToken: Symbol('t'), audioBuffer: { duration: 20, length: 960000, numberOfChannels: 2 } };

function mixerRegistry(project: any) {
  const ctx = fakeContext();
  function AudioContext() { return ctx; }
  vi.stubGlobal('window', { AudioContext, document: undefined });
  const listeners = new Map<string, Function[]>();
  const PM: PMRegistry = {
    proj: project,
    assets: { map: new Map([['tone', asset]]), get: (id: string) => (id === 'tone' ? asset : null) },
    time: 0,
    playing: true,
    headlessPlayer: true,
    evP: (_layer: any, prop: any) => prop.v,
    bus: {
      on(event: string, handler: Function) { listeners.set(event, [...(listeners.get(event) || []), handler]); },
      emit(event: string) { for (const handler of listeners.get(event) || []) handler(); }
    },
    invalidate() {},
    toast: vi.fn()
  };
  install(PM);
  return { PM, ctx };
}

function project(extra: any = {}) {
  return {
    dur: 12, fps: 30, work: [0, 12], assets: {},
    layers: [
      { id: 'music', name: 'Music', type: 'audio', on: true, from: 0, dur: 10, d: { asset: 'tone', trim: 0, gain: 0.5, fadeIn: 0, fadeOut: 0 } },
      { id: 'clip', name: 'Clip', type: 'video', on: true, from: 0, dur: 10, d: { asset: 'tone', trim: 0, embeddedAudio: true, audioGain: 2 } },
      { id: 'scene', name: 'Scene', type: 'precomp', on: true, from: 0, dur: 10, d: { comp: 'inner', audioGain: 0.5 } }
    ],
    comps: {
      inner: {
        id: 'inner', name: 'Inner', dur: 10, audioGain: 0.5,
        layers: [{ id: 'voice', name: 'Voice', type: 'audio', on: true, from: 0, dur: 10, d: { asset: 'tone', trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 } }]
      }
    },
    ...extra
  };
}

const voiceGain = (node: FakeNode) => (node.outputs[0] as FakeGain);

afterEach(() => vi.unstubAllGlobals());

describe('audio mixer routing', () => {
  it('mixes every top-level layer into its own strip and nested clips into their precomp strip', () => {
    const { PM, ctx } = mixerRegistry(project({ audioGain: 0.25 }));
    PM.Audio.start(0);
    const state = PM.Audio.inspect();
    expect(state.voices.map((voice: any) => [voice.layerId, voice.strip]).sort()).toEqual([
      ['clip:embedded-audio', 'clip'], ['music', 'music'], ['scene/voice', 'scene']
    ]);
    expect(state.strips.map((strip: any) => strip.id).sort()).toEqual(['clip', 'music', 'scene']);
    expect(state.master).toBe(0.25);
    // source → voice gain → strip sum → strip solo → master → speakers
    const sources = ctx.created.filter((node): node is FakeSource => node instanceof FakeSource);
    for (const source of sources) {
      const sum = voiceGain(source).outputs[0]!;
      const solo = sum.outputs[0]!;
      const master = solo.outputs[0]!;
      expect(master.outputs).toEqual([ctx.destination]);
    }
  });

  it('applies precomp, nested composition and embedded soundtrack levels to each voice', () => {
    const { PM, ctx } = mixerRegistry(project());
    PM.Audio.start(0);
    const sources = ctx.created.filter((node): node is FakeSource => node instanceof FakeSource);
    const levels = sources.map((source) => voiceGain(source).gain.value);
    // music 0.5 · clip soundtrack 2 · scene 0.5 × inner comp 0.5 × voice 1
    expect(levels).toEqual([0.5, 2, 0.25]);
  });

  it('evaluates chained levels in the envelope', () => {
    const { PM } = mixerRegistry(project());
    const nested = { ...PM.proj.comps.inner.layers[0], id: 'scene/voice', _audioChain: [{ layer: PM.proj.layers[2], offset: 0, comp: PM.proj.comps.inner }] };
    expect(PM.Audio.gainAt(nested, 1)).toBeCloseTo(1 * 0.5 * 0.5);
    const animated = { ...nested, _audioChain: [{ layer: { ...PM.proj.layers[2], d: { comp: 'inner', audioGain: { v: 2, kf: [{ t: 0, v: 2 }] } } }, offset: 0, comp: { audioGain: 1 } }] };
    expect(PM.Audio.gainAt(animated, 1)).toBeCloseTo(2);
    // Animated chain levels are sampled densely, not just at the clip ends.
    expect(PM.Audio.envelopePoints(animated, 0, 1, 10).length).toBeGreaterThan(20);
  });

  it('plays a nested video soundtrack from the precomp\'s start, like a nested audio clip', () => {
    const nested = project();
    nested.layers = [{ ...nested.layers[2]!, from: 2 }];
    nested.comps.inner.layers = [
      { id: 'voice', name: 'Voice', type: 'audio', on: true, from: 0, dur: 10, d: { asset: 'tone', trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 } },
      { id: 'shot', name: 'Shot', type: 'video', on: true, from: 0, dur: 10, d: { asset: 'tone', trim: 0, embeddedAudio: true } }
    ] as any;
    const { PM, ctx } = mixerRegistry(nested);
    PM.time = 3;
    PM.Audio.start(3);
    const offsets = ctx.created.filter((node): node is FakeSource => node instanceof FakeSource).map((source) => source.started?.[1]);
    // One second into the precomp, both nested clips are one second into their media.
    expect(offsets).toEqual([1, 1]);
  });

  it('silences a muted precomp and a muted video soundtrack', () => {
    const muted = project();
    muted.layers[1]!.d.audioMuted = true;
    (muted.layers[2]!.d as any).audioMuted = true;
    const { PM } = mixerRegistry(muted);
    PM.Audio.start(0);
    expect(PM.Audio.inspect().voices.map((voice: any) => voice.layerId)).toEqual(['music']);
  });

  it('re-schedules a level change without restarting the voice', () => {
    const { PM, ctx } = mixerRegistry(project());
    PM.Audio.start(0);
    const before = ctx.created.filter((node) => node instanceof FakeSource).length;
    PM.proj.layers[0].d.gain = 1.5;
    PM.bus.emit('layers');
    expect(ctx.created.filter((node) => node instanceof FakeSource).length).toBe(before);
    PM.proj.layers[1].d.audioGain = 0.5;
    PM.Audio.retune();
    expect(ctx.created.filter((node) => node instanceof FakeSource).length).toBe(before);
    const music = ctx.created.find((node): node is FakeSource => node instanceof FakeSource && voiceGain(node).outputs[0] !== undefined)!;
    expect(voiceGain(music).gain.value).toBeCloseTo(1.5);
  });

  it('glides a live level change instead of stepping it', () => {
    const { PM, ctx } = mixerRegistry(project());
    PM.Audio.start(0);
    const music = ctx.created.find((node): node is FakeSource => node instanceof FakeSource && voiceGain(node).outputs[0] !== undefined)!;
    const param = voiceGain(music).gain;
    expect(param.value).toBeCloseTo(0.5);
    param.events = [];
    PM.proj.layers[0].d.gain = 1.5;
    PM.Audio.retune();
    // Hold the current value, then ramp to the new level; never a hard set to 1.5.
    expect(param.events.slice(0, 3)).toEqual([['cancel', 0], ['set', 0.5], ['ramp', 1.5]]);
    expect(param.events.some(([kind, value]) => kind === 'set' && value === 1.5)).toBe(false);
  });

  it('restarts a voice when what it plays changes', () => {
    const { PM, ctx } = mixerRegistry(project());
    PM.Audio.start(0);
    const before = ctx.created.filter((node) => node instanceof FakeSource).length;
    PM.proj.layers[0].d.trim = 2;
    PM.bus.emit('layers');
    expect(ctx.created.filter((node) => node instanceof FakeSource).length).toBe(before + 1);
  });

  it('monitor solo silences the other strips without touching the voices or the project', () => {
    const { PM, ctx } = mixerRegistry(project());
    PM.Audio.start(0);
    const sources = ctx.created.filter((node) => node instanceof FakeSource).length;
    const json = JSON.stringify(PM.proj);
    PM.Audio.setMonitorSolo(['music']);
    const strips = Object.fromEntries(PM.Audio.inspect().strips.map((strip: any) => [strip.id, strip.solo]));
    expect(strips).toEqual({ music: 1, clip: 0, scene: 0 });
    expect(PM.Audio.monitorSolo()).toEqual(['music']);
    expect(ctx.created.filter((node) => node instanceof FakeSource).length).toBe(sources);
    expect(JSON.stringify(PM.proj)).toBe(json);
    PM.Audio.setMonitorSolo([]);
    expect(PM.Audio.inspect().strips.every((strip: any) => strip.solo === 1)).toBe(true);
  });

  it('moves the master level live and drops strips that left the composition', () => {
    const { PM, ctx } = mixerRegistry(project());
    PM.Audio.start(0);
    const sources = ctx.created.filter((node) => node instanceof FakeSource).length;
    PM.proj.audioGain = 2;
    PM.Audio.retune();
    expect(PM.Audio.inspect().master).toBe(2);
    expect(ctx.created.filter((node) => node instanceof FakeSource).length).toBe(sources);
    PM.proj.layers = PM.proj.layers.slice(0, 1);
    PM.bus.emit('layers');
    expect(PM.Audio.inspect().strips.map((strip: any) => strip.id)).toEqual(['music']);
  });

  it('taps levels only on demand and releases every tap', () => {
    const { PM, ctx } = mixerRegistry(project());
    expect(PM.Audio.meters.read('music', { peak: [0, 0], rms: [0, 0] })).toBe(false);
    PM.Audio.start(0);
    const out = { peak: [0, 0], rms: [0, 0] };
    expect(PM.Audio.meters.read('music', out)).toBe(true);
    expect(PM.Audio.meters.read(PM.Audio.meters.MASTER, out)).toBe(true);
    const analysers = ctx.created.filter((node): node is FakeAnalyser => node instanceof FakeAnalyser);
    expect(analysers).toHaveLength(4);
    analysers[0]!.level = 0.8;
    PM.Audio.meters.read('music', out);
    expect(out.peak[0]).toBeCloseTo(0.8);
    expect(out.rms[0]).toBeGreaterThan(0.5);
    expect(PM.Audio.inspect().taps.sort()).toEqual(['$master', 'music']);
    // Taps hang off the mix; they never feed it.
    for (const analyser of analysers) expect(analyser.outputs).toEqual([]);
    PM.Audio.meters.release();
    expect(PM.Audio.inspect().taps).toEqual([]);
    const strip = ctx.created.find((node) => node instanceof FakeGain && node.outputs.some((out) => out.kind === 'splitter'));
    expect(strip).toBeUndefined();
  });
});

describe('audio export mix', () => {
  it('applies the composition output level to an offline mixdown', async () => {
    const offlineNodes: FakeNode[] = [];
    class OfflineAudioContext {
      destination = new FakeNode('destination');
      createGain() { const node = new FakeGain(); offlineNodes.push(node); return node; }
      createBufferSource() { const node = new FakeSource(); offlineNodes.push(node); return node; }
      async startRendering() { return { rendered: true }; }
    }
    const { PM } = mixerRegistry(project({ audioGain: 0.5 }));
    (globalThis as any).window.OfflineAudioContext = OfflineAudioContext;
    PM.playing = false;
    const result = await PM.Audio.renderOffline(0, 1);
    expect(result).toEqual({ rendered: true });
    const output = offlineNodes.find((node): node is FakeGain => node instanceof FakeGain && node.outputs[0]?.kind === 'destination')!;
    expect(output.gain.value).toBe(0.5);
    const voices = offlineNodes.filter((node) => node instanceof FakeSource);
    expect(voices.length).toBe(3);
    for (const voice of voices) expect(voiceGain(voice).outputs[0]).toBe(output);
  });
});
