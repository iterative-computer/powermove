import { describe, expect, it } from 'vitest';

import type { RenderStats } from '../../../shared/engine-protocol';
import { EngineStatsView, PresentedEvents, RevisionVersions, emitPresented } from './presentation-events';
import { EngineTransport } from './transport';
import type { DocumentSession } from './session';
import type { HostClock } from './host-clock';

describe('frame events on presentation', () => {
  it("emit today's preview:presented then overlay, with the exact viewport object and the frame's own version", () => {
    const versions = new RevisionVersions();
    versions.note(4, 40);
    versions.note(5, 51);
    versions.note(7, 70);
    const viewport = { x: 0, y: 0, width: 640, height: 360 };
    const project = { id: 'p' };
    const events: Array<[string, unknown]> = [];
    const bus = { emit: (event: string, payload?: unknown) => events.push([event, payload]) };
    const payload = emitPresented(bus, { pts: 352_800_000, revision: 6, viewSeq: 3, seq: 9 }, 0.5, { viewport: () => viewport, versions, project: () => project });
    expect(events.map(([e]) => e)).toEqual(['preview:presented', 'overlay']);
    expect(events[0]![1]).toBe(payload);
    expect(payload.viewport).toBe(viewport);
    expect(payload).toMatchObject({ time: 0.5, version: 51, quality: 0.5, project });
  });

  it('versions follow revisions, forget old ones, and restart after a resync to a lower revision', () => {
    const v = new RevisionVersions(3);
    for (let r = 1; r <= 5; r++) v.note(r, r * 10);
    expect(v.at(5)).toBe(50);
    expect(v.at(1)).toBeUndefined();
    v.note(5, 55);
    expect(v.at(9)).toBe(55);
    v.note(2, 20);
    expect(v.at(5)).toBe(20);
  });
});

describe('presented frames are emitted once per animation frame', () => {
  it('the newest frame of an animation frame wins', () => {
    const frames: Array<() => void> = [];
    const emitted: number[] = [];
    const events = new PresentedEvents((frame) => emitted.push(frame.seq), (fn) => frames.push(fn));
    events.push({ pts: 0, revision: 1, viewSeq: 1, seq: 1 }, 1);
    events.push({ pts: 0, revision: 1, viewSeq: 1, seq: 2 }, 1);
    expect(frames).toHaveLength(1);
    frames.shift()!();
    expect(emitted).toEqual([2]);
    events.push({ pts: 0, revision: 2, viewSeq: 1, seq: 3 }, 1);
    frames.shift()!();
    expect(emitted).toEqual([2, 3]);
    expect(events.emitted).toBe(2);
  });
});

describe('engine stats for the status bar', () => {
  const stats = (frames: number, avg: number): RenderStats => ({ frames, timedFrames: frames, lateSkipped: 0, cpuUs: 1000, gpuUs: 2000, layers: 12, frameUsAvg: avg, programs: 4, rasters: 9, layerFailures: 0, snapshots: 0, snapshotUsMax: 0, revision: 3 });

  it('map to the perf fields with presented fps over the interval', () => {
    const view = new EngineStatsView();
    expect(view.perf()).toBeNull();
    view.receive(stats(100, 8_000), 1_000);
    expect(view.perf()).toEqual({ ms: 8, fps: 0, draws: 12, passes: 0, progs: 4, raster: 9 });
    view.receive(stats(130, 9_000), 1_500);
    expect(view.perf()).toMatchObject({ ms: 9, fps: 60 });
  });
});

describe('the exclusive transport ("Use New Engine" on)', () => {
  const clock = { hostUs: () => 0 } as unknown as HostClock;

  it('keeps the transport while no session draws, and replays the playhead when one does', () => {
    let session: Partial<DocumentSession> | null = null;
    const all: Array<{ command: string }> = [];
    const sent = {
      push: (c: unknown) => all.push(c as { command: string }),
      get moves() { return all.filter((c) => c.command !== 'audioScrub'); },
      clear: () => { all.length = 0; }
    };
    const transport = new EngineTransport(() => session as DocumentSession | null, clock, { exclusive: true });
    // Cold start: the current engine never takes the transport.
    expect(transport.active()).toBe(true);
    transport.play(2);
    expect(sent.moves).toEqual([]);
    transport.seek(3, false);
    session = { closed: false, route: 'engine', frames: 1, id: 4, clock: null, transport: (c: unknown) => sent.push(c) } as never;
    transport.refresh();
    expect(sent.moves).toEqual([
      { command: 'seek', time: 3 * 705_600_000, mode: 'exact' },
      { command: 'play', rate: { num: 1, den: 1 } }
    ]);
    // A crash: the session goes away, the transport stays.
    session = null;
    transport.refresh();
    expect(transport.active()).toBe(true);
    transport.pause();
    session = { closed: false, route: 'engine', frames: 1, id: 5, clock: null, transport: (c: unknown) => sent.push(c) } as never;
    sent.clear();
    transport.refresh();
    expect(sent.moves).toEqual([{ command: 'seek', time: 3 * 705_600_000, mode: 'exact' }]);
    // Capability routing (scaffolding) still hands it over.
    session = { closed: false, route: 'legacy', frames: 0, id: 5, clock: null, transport: () => {} } as never;
    expect(transport.active()).toBe(false);
  });

  it('is unchanged when not exclusive', () => {
    const transport = new EngineTransport(() => null, clock);
    expect(transport.active()).toBe(false);
  });
});
