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
  it('a frame drawn on arrival goes at its report; the newest of the others at the next animation frame', () => {
    const frames: Array<(t: number) => void> = [];
    const emitted: number[] = [];
    const events = new PresentedEvents((frame) => emitted.push(frame.seq), (fn) => frames.push(fn));
    const paused = (seq: number) => ({ pts: 0, revision: 1, viewSeq: 1, seq });
    // On screen already: announced now, not a refresh later.
    events.push(paused(1), 1);
    expect(emitted).toEqual([1]);
    expect(frames).toHaveLength(1);
    // Two more before the next animation frame: the newest goes there.
    events.push(paused(2), 1);
    events.push(paused(3), 1);
    expect(emitted).toEqual([1]);
    frames.shift()!(0);
    expect(emitted).toEqual([1, 3]);
    // Still one per animation frame after that one.
    events.push(paused(4), 1);
    expect(emitted).toEqual([1, 3]);
    frames.shift()!(16);
    expect(emitted).toEqual([1, 3, 4]);
    // An animation frame with nothing to announce: the next goes at once again.
    frames.shift()!(33);
    expect(frames).toHaveLength(0);
    events.push(paused(5), 1);
    expect(emitted).toEqual([1, 3, 4, 5]);
    expect(events.emitted).toBe(4);
    expect(events.coalesced).toBe(1);
  });

  it('a timed frame goes at the first animation frame after the refresh that drew it, whichever side of it its report arrived', () => {
    const P = 1000 / 120;
    const frames: Array<(t: number) => void> = [];
    const emitted: number[] = [];
    const events = new PresentedEvents((frame) => emitted.push(frame.seq), (fn) => frames.push(fn));
    const timed = (seq: number, refresh: number) => ({ pts: seq, revision: 1, viewSeq: 1, seq, timed: true, rafMs: refresh * P + 0.05 });
    // Refresh 1's report arrives before the page's animation frame of refresh 1 runs.
    events.push(timed(1, 1), 1);
    frames.shift()!(1 * P);
    expect(emitted).toEqual([]);
    // Refresh 2's arrives before refresh 2's animation frame; refresh 3's after it.
    events.push(timed(2, 2), 1);
    frames.shift()!(2 * P);
    expect(emitted).toEqual([1]);
    frames.shift()!(3 * P);
    events.push(timed(3, 3), 1);
    expect(emitted).toEqual([1, 2]);
    frames.shift()!(4 * P);
    expect(emitted).toEqual([1, 2, 3]);
    expect(events.coalesced).toBe(0);
    expect(events.pending).toBe(0);
    expect(frames).toHaveLength(0);
  });
});

describe('a Play from rest announces the frame on screen first, as today\'s first frame does', () => {
  it('at the next animation frame, ahead of the run, and only when it is current', () => {
    const frames: Array<(t: number) => void> = [];
    const emitted: number[] = [];
    const events = new PresentedEvents((frame) => emitted.push(frame.seq), (fn) => frames.push(fn));
    const now = (over: Partial<{ revision: number | null; viewSeq: number | null; pts: number | null }> = {}) => ({ revision: 4, viewSeq: 2, pts: 1000, frameTicks: 100, ...over });
    expect(events.replay(now())).toBe(false);
    events.push({ pts: 1000, revision: 4, viewSeq: 2, seq: 7 }, 1);
    frames.shift()!(0);
    // An edit not drawn yet: the engine draws the run's first frame itself.
    expect(events.replay(now({ revision: 5 }))).toBe(false);
    expect(events.replay(now())).toBe(true);
    expect(emitted).toEqual([7]);
    frames.shift()!(16);
    expect(emitted).toEqual([7, 7]);
    expect(events.replays).toBe(1);
  });
});

describe('a paused redraw announces the frame on screen when nothing new is coming', () => {
  it('as today\'s frame loop announces the frame it draws again, but never ahead of a changed revision, view or time', () => {
    const frames: Array<(t: number) => void> = [];
    const events = new PresentedEvents(() => {}, (fn) => frames.push(fn));
    const announced: number[] = [];
    const announce = (frame: { seq: number }) => { announced.push(frame.seq); return true; };
    const now = (over: Partial<{ revision: number | null; viewSeq: number | null; pts: number | null }> = {}) => ({ revision: 4, viewSeq: 2, pts: 1000, frameTicks: 100, ...over });
    // Nothing on screen yet.
    expect(events.redraw(now(), announce)).toBe('none');
    events.push({ pts: 1000, revision: 4, viewSeq: 2, seq: 7 }, 1);
    frames.shift()!(0);
    expect(events.redraw(now(), announce)).toBe('announced');
    expect(announced).toEqual([7]);
    // An edit (a newer revision), a view change, or another frame: the
    // engine presents a new frame, which announces itself, so nothing is
    // announced now.
    expect(events.redraw(now({ revision: 5 }), announce)).toBe('coming');
    expect(events.redraw(now({ viewSeq: 3 }), announce)).toBe('coming');
    expect(events.redraw(now({ pts: 1100 }), announce)).toBe('coming');
    // A frame waiting for its animation frame wins (one went at once).
    events.push({ pts: 1000, revision: 4, viewSeq: 2, seq: 8 }, 1);
    events.push({ pts: 1000, revision: 4, viewSeq: 2, seq: 9 }, 1);
    expect(events.redraw(now(), announce)).toBe('coming');
    frames.shift()!(16);
    // The announcer declines when the redraw changed what the engine draws.
    expect(events.redraw(now(), () => false)).toBe('coming');
    expect(events.redraws).toBe(1);
  });
});

describe('an announcement in progress', () => {
  it('is visible to the listeners of a presented frame and of a redraw, and only to them', () => {
    const frames: Array<(t: number) => void> = [];
    const seen: boolean[] = [];
    let events!: PresentedEvents;
    events = new PresentedEvents(() => seen.push(events.announcing), (fn) => frames.push(fn));
    expect(events.announcing).toBe(false);
    events.push({ pts: 1000, revision: 4, viewSeq: 2, seq: 1 }, 1);
    frames.shift()!(0);
    expect(events.redraw({ revision: 4, viewSeq: 2, pts: 1000, frameTicks: 100 }, () => { seen.push(events.announcing); return true; })).toBe('announced');
    expect(() => events.redraw({ revision: 4, viewSeq: 2, pts: 1000, frameTicks: 100 }, () => { throw new Error('listener'); })).toThrow('listener');
    expect(seen).toEqual([true, true]);
    expect(events.announcing).toBe(false);
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
    // An unsupported composition never hands the flag-on transport over.
    session = { closed: false, route: 'legacy', frames: 0, id: 5, clock: null, transport: () => {} } as never;
    expect(transport.active()).toBe(true);
  });

  it('runs what a seek\'s frame needs before the seek leaves, and rests where it was sent until the line moves', () => {
    const order: string[] = [];
    let sample: unknown = null;
    const session = { closed: false, route: 'engine', frames: 1, id: 4, get clock() { return sample; }, transport: (c: { command: string }) => order.push(c.command) };
    const transport = new EngineTransport(() => session as never, clock, {
      beforeSeek: (time) => order.push(`answers ${time}`)
    });
    expect(transport.resting()).toBeNull();
    transport.seek(2, true);
    expect(order).toEqual(['audioScrub', `answers ${2 * 705_600_000}`, 'seek']);
    expect(transport.resting()).toBe(2 * 705_600_000);
    // A pause rests where the clock stopped.
    sample = { seq: 1, epoch: 2, state: 'stopped', hostTime: 0, mediaTime: 5 * 705_600_000, rate: { num: 0, den: 1 } };
    transport.pause();
    expect(transport.resting()).toBe(5 * 705_600_000);
    // A line played with the editor stopped (a cached preview) rests nowhere.
    transport.play(5);
    sample = { seq: 2, epoch: 3, state: 'playing', hostTime: 0, mediaTime: 5 * 705_600_000, rate: { num: 1, den: 1 } };
    expect(transport.resting()).toBeNull();
  });

  it('is unchanged when not exclusive', () => {
    const transport = new EngineTransport(() => null, clock);
    expect(transport.active()).toBe(false);
  });
});
