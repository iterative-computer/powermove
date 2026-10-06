// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { createCaptionTrack } from './caption-track';
import { layoutTracks, planTrackMove } from './track-layout';

function harness(cueCount = 3) {
  const cues = Array.from({ length: cueCount }, (_, index) => ({ id: `c${index}`, start: index * 2, end: index * 2 + 1.5, text: `Cue ${index}` }));
  const layer: any = { id: 'cap', type: 'captions', name: 'English', from: 1, dur: cueCount * 2 + 2, lock: false, d: { cues, style: {} } };
  const project: any = { fps: 30, dur: 60, work: [0, 60], markers: [], layers: [layer] };
  let drag: any = null;
  const api: any = {
    project: { get: () => project },
    transport: { time: () => 30 },
    selection: { set: vi.fn(), select: vi.fn(), layers: () => ['cap'] },
    services: { get: () => null },
    model: { layer: () => layer },
    edit: { begin: vi.fn(), dispatch: vi.fn(), commit: vi.fn(), cancel: vi.fn(), apply: vi.fn(() => ({ ok: true })) },
  };
  const T: any = { gut: 100, pps: 100, scrollT: 0 };
  const t2x = (t: number) => T.gut + (t - T.scrollT) * T.pps;
  const x2t = (x: number) => (x - T.gut) / T.pps + T.scrollT;
  const calls = { rects: 0, text: [] as string[] };
  const context: any = new Proxy({
    fillRect: () => { calls.rects++; }, fillText: (text: string) => { calls.text.push(text); },
    measureText: (text: string) => ({ width: text.length * 6 }),
  }, { get: (target, key) => key in target ? (target as any)[key] : () => {}, set: (target, key, value) => { (target as any)[key] = value; return true; } });
  const track = createCaptionTrack({
    api, T, t2x, x2t,
    theme: () => ({ accent: '#0A84FF' }),
    roundRect: () => {},
    clipText: (c, text, x, y) => c.fillText(text, x, y),
    clipPalette: () => ({ body: '#444444', primary: '#999999', foreground: '#FFFFFF', ring: '#000000' }),
    mix: (color) => color,
    fui: () => 'system-ui',
    beginDrag: (_event, options) => { drag = options; return {}; },
    invalidate: vi.fn(),
    edgeSnapping: () => false,
    wrap: () => document.body,
    cleanup: () => {},
  });
  return { api, T, layer, track, context, calls, drag: () => drag, x: (t: number) => t2x(t) };
}

const pointer = (extra: Partial<PointerEvent> = {}) => ({ shiftKey: false, metaKey: false, ctrlKey: false, ...extra }) as PointerEvent;

describe('caption track', () => {
  it('hits cue bodies and edges in composition time', () => {
    const { track, x } = harness();
    // Layer starts at 1s: cue c1 spans 3..4.5 in the composition.
    expect(track.hit({ ...harness().layer }, x(3.7))?.cue.id).toBe('c1');
    const { layer, track: t2 } = harness();
    expect(t2.hit(layer, x(3) + 1)?.edge).toBe('start');
    expect(t2.hit(layer, x(4.5) - 1)?.edge).toBe('end');
    expect(t2.hit(layer, x(4.8))).toBeNull();
    expect(track).toBeTruthy();
  });

  it('selects on click and moves selected cues without crossing a neighbour', () => {
    const { track, layer, api, drag, x } = harness();
    expect(track.down(pointer(), layer, x(3.7))).toBe(true);
    expect(track.selection()).toEqual({ layerId: 'cap', cues: ['c1'] });
    drag().move(400, 0, pointer());
    expect(api.edit.begin).toHaveBeenCalledWith('Move caption', { origin: 'timeline' });
    // c1 (3..4.5) can move until c2 starts at 5: at most +0.5s.
    expect(api.edit.dispatch).toHaveBeenLastCalledWith({ type: 'edit_captions', target: 'cap', op: 'update', cues: [{ id: 'c1', start: 3.5, end: 5 }] });
    drag().up();
    expect(api.edit.commit).toHaveBeenCalledWith('Move caption');
  });

  it('trims an edge to the frame grid, clamped by the neighbour', () => {
    const { track, layer, api, drag, x } = harness();
    track.down(pointer(), layer, x(4.5) - 1);
    drag().move(-21, 0, pointer());
    expect(api.edit.dispatch).toHaveBeenLastCalledWith({ type: 'edit_captions', target: 'cap', op: 'update', cues: [{ id: 'c1', end: 1 + 3.3 }] });
    drag().move(-500, 0, pointer());
    const last = api.edit.dispatch.mock.calls.at(-1)[0];
    expect(last.cues[0].end).toBeCloseTo(1 + 2 + 0.04);
    drag().cancel();
    expect(api.edit.cancel).toHaveBeenCalled();
  });

  it('extends and toggles the selection with modifiers and leaves locked cues in place', () => {
    const { track, layer, api, drag, x } = harness();
    track.down(pointer(), layer, x(1.5));
    track.down(pointer({ shiftKey: true }), layer, x(3.5));
    expect(track.selection().cues).toEqual(['c0', 'c1']);
    track.down(pointer({ metaKey: true }), layer, x(1.5));
    expect(track.selection().cues).toEqual(['c1']);
    layer.lock = true;
    const before = drag();
    track.down(pointer(), layer, x(5.5));
    expect(drag()).toBe(before);
    expect(api.edit.begin).not.toHaveBeenCalled();
  });

  it('edits text in place and commits on Enter', () => {
    const { track, layer, api } = harness();
    expect(track.edit(layer, 'c0', 40, 30)).toBe(true);
    const field = document.querySelector<HTMLTextAreaElement>('.tl-caption-editor')!;
    expect(field.value).toBe('Cue 0');
    field.value = 'Edited';
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    expect(api.edit.apply).toHaveBeenCalledWith({ type: 'edit_captions', target: 'cap', op: 'update', cues: [{ id: 'c0', text: 'Edited' }] }, { label: 'Edit caption', origin: 'timeline' });
    expect(document.querySelector('.tl-caption-editor')).toBeNull();
  });

  it('adds a cue in a gap on double-click', () => {
    const { track, layer, api, x } = harness();
    track.doubleClick(layer, x(2.7), 40, 30);
    expect(api.edit.apply.mock.calls[0][0]).toEqual({ type: 'edit_captions', target: 'cap', op: 'insert', cues: [{ start: 2.7, end: 3, text: 'Caption' }] });
  });

  it('paints only the cues in view, even with thousands of cues', () => {
    const { track, layer, context, calls, T } = harness(5000);
    T.scrollT = 4000; T.pps = 50;
    track.draw(context, layer, 30, 40, 1100);
    // 1000px of lane at 50px/s shows 20s: about ten 2s cues.
    expect(calls.text.length).toBeGreaterThan(5);
    expect(calls.text.length).toBeLessThan(15);
    const started = performance.now();
    for (let i = 0; i < 200; i++) track.draw(context, layer, 30, 40, 1100);
    expect(performance.now() - started).toBeLessThan(200);
  });
});

describe('caption tracks in the track layout', () => {
  const layers = [
    { id: 'en', type: 'captions', from: 0, dur: 10 },
    { id: 'fr', type: 'captions', from: 0, dur: 10 },
    { id: 'v', type: 'video', from: 0, dur: 10 },
    { id: 'a', type: 'audio', from: 0, dur: 10 },
  ];
  const options = { duration: 10, collapsed: () => false };

  it('gives every captions layer its own caption track, top of the stack highest', () => {
    const layout = layoutTracks(layers, options);
    expect(layout.captions).toBe(2);
    expect(layout.byId.get('en')).toMatchObject({ area: 'captions', lane: 1 });
    expect(layout.byId.get('fr')).toMatchObject({ area: 'captions', lane: 0 });
    expect(layout.byId.get('v')).toMatchObject({ area: 'video', lane: 0 });
    expect(layout.video).toBe(1);
  });

  it('reorders caption tracks among themselves without touching picture or sound', () => {
    const plan = planTrackMove(layers, options, { ids: ['fr'], dt: 0, lanes: { video: 0, audio: 0, captions: 1 } });
    expect(plan.layers.map(layer => layer.id)).toEqual(['fr', 'en', 'v', 'a']);
    const after = layoutTracks(plan.layers, options);
    expect(after.byId.get('fr')?.lane).toBe(1);
  });
});
