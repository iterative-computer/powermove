import { describe, expect, it } from 'vitest';
import { layoutTracks, planTrackMove, type TrackLayer } from './track-layout';

const clip = (id: string, from: number, dur: number, extra: Partial<TrackLayer> = {}): TrackLayer =>
  ({ id, type: 'solid', from, dur, ...extra });
const open = { collapsed: () => false, duration: 10 };
const lanes = (layers: TrackLayer[], options = open) =>
  Object.fromEntries(layoutTracks(layers, options).items.map(item => [item.id, `${item.area[0]}${item.lane}`]));

describe('layoutTracks', () => {
  it('shares a lane between clips that never overlap', () => {
    expect(lanes([clip('a', 0, 2), clip('b', 2, 2), clip('c', 4, 1)])).toEqual({ a: 'v0', b: 'v0', c: 'v0' });
  });

  it('keeps overlapping clips in stack order: the top layer sits on the higher track', () => {
    const layout = layoutTracks([clip('top', 1, 2), clip('mid', 0, 2), clip('bottom', 0, 5)], open);
    expect(Object.fromEntries(layout.items.map(item => [item.id, item.lane]))).toEqual({ top: 2, mid: 1, bottom: 0 });
    expect(layout.video).toBe(3);
  });

  it('honours a preferred track as a minimum', () => {
    expect(lanes([clip('a', 0, 2, { track: 3 }), clip('b', 0, 2)])).toEqual({ a: 'v3', b: 'v0' });
    // Overlap constraints still win over a lower preference.
    expect(lanes([clip('a', 0, 2), clip('b', 0, 2, { track: 2 })])).toEqual({ a: 'v3', b: 'v2' });
  });

  it('separates top-level audio into its own area', () => {
    const layout = layoutTracks([clip('v', 0, 4), clip('m', 0, 4, { type: 'audio' }), clip('vo', 1, 2, { type: 'audio' })], open);
    expect(Object.fromEntries(layout.items.map(item => [item.id, `${item.area}:${item.lane}`])))
      .toEqual({ v: 'video:0', m: 'audio:1', vo: 'audio:0' });
    expect([layout.video, layout.audio]).toEqual([1, 2]);
  });

  it('packs an expanded group as one block with its bar on top', () => {
    const layers = [
      clip('g', 0, 10, { type: 'group' }),
      clip('g1', 0, 3, { group: 'g' }),
      clip('g2', 1, 3, { group: 'g' }),
      clip('bg', 0, 6),
    ];
    const layout = layoutTracks(layers, open);
    expect(Object.fromEntries(layout.items.map(item => [item.id, item.lane]))).toEqual({ g: 3, g1: 2, g2: 1, bg: 0 });
    expect(layout.bands).toEqual([expect.objectContaining({ id: 'g', laneStart: 1, laneEnd: 3, from: 0, dur: 4 })]);
    expect(layout.byId.get('g1')).toMatchObject({ depth: 1, parent: 'g' });
  });

  it('shows a collapsed group as a single nest clip', () => {
    const layers = [clip('g', 0, 10, { type: 'group' }), clip('g1', 2, 3, { group: 'g' }), clip('g2', 2, 3, { group: 'g' })];
    const layout = layoutTracks(layers, { ...open, collapsed: id => id === 'g' });
    expect(layout.items).toEqual([expect.objectContaining({ id: 'g', kind: 'group', lane: 0, from: 2, dur: 3 })]);
    expect(layout.bands).toEqual([]);
  });

  it('never interleaves outside clips with group members', () => {
    const layers = [
      clip('over', 0, 10),
      clip('g', 0, 10, { type: 'group' }),
      clip('g1', 0, 2, { group: 'g' }),
      clip('g2', 5, 2, { group: 'g' }),
    ];
    // `over` overlaps the group span, so it sits above the whole block even
    // where it would fit beside a member.
    expect(lanes(layers)).toEqual({ over: 'v2', g: 'v1', g1: 'v0', g2: 'v0' });
  });
});

describe('planTrackMove', () => {
  const apply = (layers: TrackLayer[], ids: string[], dt: number, video: number, audio = 0) =>
    planTrackMove(layers, open, { ids, dt, lanes: { video, audio } }).layers;

  it('moves a clip up a track and writes the new stack order', () => {
    const plan = apply([clip('a', 0, 2), clip('b', 3, 2)], ['b'], 0, 1);
    expect(plan.map(layer => layer.id)).toEqual(['b', 'a']);
    expect(lanes(plan)).toEqual({ a: 'v0', b: 'v1' });
  });

  it('moving onto an occupied track lands on the next free one', () => {
    const plan = apply([clip('top', 0, 2), clip('low', 0, 2), clip('free', 5, 2)], ['free'], -5, 0);
    // `free` arrives on V1 where `low` already sits, so it rises above it.
    expect(lanes(plan).free).toBe('v1');
    expect(plan.find(layer => layer.id === 'free')!.from).toBe(0);
  });

  it('lets a clip drop onto V1 beneath others when the time is free', () => {
    const plan = apply([clip('a', 0, 2), clip('b', 0, 2)], ['a'], 4, -1);
    expect(lanes(plan)).toEqual({ a: 'v0', b: 'v0' });
  });

  it('opens a new track above the top one but no further', () => {
    const plan = apply([clip('a', 0, 2), clip('b', 0, 2)], ['b'], 0, 5);
    expect(lanes(plan)).toEqual({ a: 'v1', b: 'v2' });
  });

  it('moves a group with its members and keeps them contiguous', () => {
    const layers = [
      clip('x', 0, 2),
      clip('g', 0, 10, { type: 'group' }),
      clip('g1', 4, 2, { group: 'g' }),
      clip('y', 6, 2),
    ];
    const plan = apply(layers, ['g', 'g1'], 2, 0);
    // Members stay directly under their group in the stack.
    expect(plan.map(layer => layer.id)).toEqual(['g', 'g1', 'x', 'y']);
    expect(plan.find(layer => layer.id === 'g1')!.from).toBe(6);
    // The group now overlaps `y`, so the stack order lifts the block.
    expect(lanes(plan)).toMatchObject({ g: 'v2', g1: 'v1', y: 'v0' });
  });

  it('keeps a member inside its group block', () => {
    const layers = [clip('g', 0, 10, { type: 'group' }), clip('g1', 0, 2, { group: 'g' }), clip('bg', 0, 4)];
    const plan = apply(layers, ['g1'], 0, -3);
    expect(lanes(plan)).toEqual({ g: 'v2', g1: 'v1', bg: 'v0' });
    expect(plan.find(layer => layer.id === 'g1')!.group).toBe('g');
  });

  it('never reorders audio against video', () => {
    const layers = [clip('v1', 0, 2), clip('music', 0, 4, { type: 'audio' }), clip('v2', 0, 2), clip('vo', 0, 1, { type: 'audio' })];
    const plan = apply(layers, ['v2', 'vo'], 0, 1, 1);
    expect(plan.map(layer => layer.type)).toEqual(['solid', 'audio', 'solid', 'audio']);
    expect(lanes(plan)).toEqual({ v1: 'v1', v2: 'v2', music: 'a1', vo: 'a2' });
  });

  it('reproduces the same lanes when packed again', () => {
    const layers = [clip('a', 0, 2), clip('b', 1, 2), clip('c', 5, 2, { track: 4 })];
    const plan = apply(layers, ['a'], 3, 1);
    expect(lanes(apply(plan, [], 0, 0))).toEqual(lanes(plan));
  });
});
