/*
 * The engine path's main-thread work per presented frame, counted on a
 * scripted playback (deterministic: real wire encoding, a frozen clock for
 * the lookahead's time budget, animation frames run by hand), held to the
 * shared perfbench baseline. `PM_PERF_RATCHET=update` lowers exact counts.
 *
 * The bench is a large project's playback as the engine drives it: each
 * frame the engine asks for the raster override's layers of a frame ahead
 * (`needs`), sends a frame descriptor, and every sixth frame a clock sample;
 * the window runs one animation frame per frame. Before round 3 the window
 * posted one message per answered layer (about 300 a frame here) and ran a
 * 4 ms timer besides its port tasks; before the lookahead followed every
 * layer (it kept 256), it answered 15 frames of each again every frame
 * (385 KB a frame here, against 24 KB).
 */
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';

import { PROTOCOL_BUILD, type CompatNeed, type Envelope } from '../../../shared/engine-protocol';
import { decodeEnvelope, encodeEnvelope, envelope, localRanges } from '../../../main/engine/protocol';
import { EngineConnection, type PortLike } from './connection';
import { MainThreadCounters } from './main-thread-counters';
import { compareAndLowerMainThread, type PerfBaselines } from './main-thread-ratchet';
import { EngineRenderTransport, type TransportSession } from './render-transport';

const BASELINE = path.resolve(__dirname, '../../../../../../engine/crates/pm-perfbench/baselines.json');
const BENCH = 'desktop/playback-300-passthrough-layers';

class Port implements PortLike {
  private listeners: ((e: { data: unknown }) => void)[] = [];
  sent: Envelope[] = [];
  constructor(private readonly onSent: (m: Envelope) => void) {}
  postMessage(message: unknown): void {
    const m = decodeEnvelope(message as Uint8Array);
    this.sent.push(m);
    this.onSent(m);
  }
  addEventListener(type: 'message' | 'close', listener: never): void {
    if (type === 'message') this.listeners.push(listener);
  }
  start(): void {}
  close(): void {}
  deliver(m: Envelope | Envelope[]): void {
    // An array: what the engine's process sends this window in one turn of
    // its event loop, as one message (engine-process.ts).
    const data = Array.isArray(m) ? m.map((e) => encodeEnvelope(e)) : encodeEnvelope(m);
    for (const l of this.listeners) l({ data });
  }
}

const SESSION = 5;
const FRAME = 705_600_000 / 24;

async function playback(layers: number, frames: number): Promise<{ frames: number; counts: Record<string, number> }> {
  const counters = new MainThreadCounters(() => 0);
  // What the engine holds answers for (its compat store): it asks only for
  // the rest (`CompatStore::to_ask`).
  const answered = new Set<string>();
  const port = new Port((m) => {
    const body = m.message.kind === 'event' ? (m.message.body as any) : null;
    if (body?.compat === 'answers') for (const n of body.natives) answered.add(`${n.layer}@${n.time}`);
    if (body?.compat === 'native') answered.add(`${body.layer}@${body.time}`);
    if (m.message.kind === 'hello') queueMicrotask(() => port.deliver(envelope({ kind: 'welcome', role: 'interactiveEngine', wire: 1, build: PROTOCOL_BUILD, contracts: localRanges().map((r) => ({ contract: r.contract, version: 0 })), maxMessageBytes: 16 * 1024 * 1024 } as never)));
  });
  const connection = new EngineConnection(1, port, () => {}, () => 0, counters);
  await connection.open();
  const session: TransportSession = { id: SESSION, closed: false, generation: 1, resyncPending: false, applied: 3, uploadFallback: () => {}, post: (e) => { connection.event(e as never, SESSION); return true; } };
  const state = { time: 0 };
  const rafs: Array<() => void> = [];
  const transport = new EngineRenderTransport({
    native: {}, session: () => session, revision: () => 3, composition: () => 'root', document: () => 'doc', viewSeq: () => 1, fallbackRunner: () => null, log: () => {},
    playing: () => true, frameTicks: () => FRAME, playhead: () => ({ time: state.time, rate: 1 }),
    nextFrame: (fn: () => void) => rafs.push(fn), counters
  } as never);
  // A wrapper that passes every layer through (today's documented pattern).
  transport.answerCompat('rasterOverride', () => ({ kind: 'native' }));
  connection.listen(SESSION, (m) => {
    const body = m.message.kind === 'event' ? (m.message.body as { type: string }) : null;
    if (body?.type === 'compat') transport.receiveCompat(body as never, session);
    if (body?.type === 'frame') counters.presentedFrame();
  });
  const names = Array.from({ length: layers }, (_, i) => `layer-${i}`);
  const needs = (k: number): CompatNeed[] => names.map((layer) => ({ composition: 'root', layer, time: k * FRAME, source: 'rasterOverride', scale: 1 }));
  const envelopeOf = (body: unknown): Envelope => ({ ...envelope({ kind: 'event', body } as never, SESSION), revision: 3 });
  const event = (body: unknown) => port.deliver(envelopeOf(body));
  // The run's first frame: everything up to the engine's ask-ahead.
  event({ type: 'compat', compat: 'needs', revision: 3, needs: [0, 1, 2, 3, 4, 5, 6, 7, 8].flatMap(needs) });
  for (const fn of rafs.splice(0)) fn();
  const before = counters.snapshot();
  for (let k = 0; k < frames; k++) {
    state.time = k * FRAME;
    // The engine keeps the frame eight ahead asked (what it has no answer
    // for), then sends this frame.
    // What the engine's turn for the frame sends, in one message.
    const turn: Envelope[] = [];
    const missing = needs(k + 8).filter((n) => !answered.has(`${n.layer}@${n.time}`));
    if (missing.length) turn.push(envelopeOf({ type: 'compat', compat: 'needs', revision: 3, needs: missing }));
    turn.push(envelopeOf({ type: 'frame', frame: { poolIndex: k % 8, poolGeneration: 1, seq: k + 1, pts: k * FRAME, revision: 3, colorSpace: 'srgb', format: 'bgra8Unorm', quality: { resolution: 'full', representation: 'original', draft3d: false, cached: false, standIn: false }, overlay: null, depthId: null, viewSeq: 1, duration: FRAME } }));
    if (k % 6 === 0) turn.push(envelopeOf({ type: 'clock', sample: { seq: k + 1, epoch: 1, state: 'playing', hostTime: k * 41_667, mediaTime: k * FRAME, rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48_000, loopRange: null } }));
    port.deliver(turn);
    for (const fn of rafs.splice(0)) fn();
  }
  transport.dispose();
  if (process.env['BUDGET_DEBUG']) {
    const tail = port.sent.slice(-12).map((m) => { const b = (m.message as any).body; return b?.compat === 'answers' ? `answers ${b.natives.length}/${b.reuses.length} times ${[...new Set(b.natives.map((n: any) => n.time / FRAME))].join(',')}` : `${b?.type}:${b?.compat ?? ''}`; });
    console.log(tail.join('\n'));
  }
  const after = counters.snapshot();
  return {
    frames: after.presented - before.presented,
    counts: {
      'main_thread.entries': Object.keys(after.entries).reduce((sum, k) => sum + after.entries[k as keyof typeof after.entries] - before.entries[k as keyof typeof before.entries], 0),
      'main_thread.port_entries': after.entries.port - before.entries.port,
      'main_thread.turns': after.entries.turn - before.entries.turn,
      'main_thread.posted': after.posted - before.posted,
      'main_thread.posted_bytes': after.postedBytes - before.postedBytes,
      'main_thread.received': after.received - before.received,
      'main_thread.received_bytes': after.receivedBytes - before.receivedBytes
    }
  };
}

describe('the engine path\'s main-thread budget', () => {
  it('a 300-layer pass-through project costs the window at most its baseline per presented frame', async () => {
    const now = vi.spyOn(performance, 'now').mockReturnValue(0);
    try {
      const { frames, counts } = await playback(300, 48);
      expect(frames).toBe(48);
      const baseline = JSON.parse(readFileSync(BASELINE, 'utf8')) as PerfBaselines;
      // The structural bounds, whatever the baseline: one turn per animation
      // frame and one answers message per turn.
      if (process.env['BUDGET_DEBUG']) console.log('main-thread budget', JSON.stringify({ frames, counts }));
      expect(counts['main_thread.turns']).toBeLessThanOrEqual(frames);
      expect(counts['main_thread.posted']).toBeLessThanOrEqual(frames);
      // One task for what the engine sent in its turn for the frame.
      expect(counts['main_thread.port_entries']).toBeLessThanOrEqual(frames);
      const update = process.env['PM_PERF_RATCHET'] === 'update';
      const { next, regressions } = compareAndLowerMainThread(baseline, BENCH, frames, counts, update);
      if (update) writeFileSync(BASELINE, `${JSON.stringify(next, null, 2)}\n`);
      expect(regressions, `above the baseline: ${JSON.stringify(counts)}`).toEqual([]);
    } finally {
      now.mockRestore();
    }
  });
});
