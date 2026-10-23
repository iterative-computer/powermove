import { expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';

function fixture() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 3 }); PM.time = 0;
  expect(PM.Edit.apply([
    { type: 'add_layer', id: 'shape', layerType: 'shape', select: true },
    { type: 'replace_keyframes', target: 'shape', path: 'opacity', keyframes: [{ time: 0, value: 20 }, { time: 1, value: 80 }] }
  ]).ok).toBe(true);
  PM.hist.clear();
  PM.sel.keys = [PM.L('shape').p.opacity.kf[0].i]; PM.sel.chan = 'opacity';
  const state = () => ({ project: JSON.stringify(PM.proj), selection: JSON.stringify(PM.sel),
    keys: PM.L('shape').p.opacity.kf.map((key: any) => key.i), revision: PM.proj.revision });
  return { PM, state, startX: PM.L('shape').p['position.x'].v };
}

const rejected = { type: 'set_property', target: 'missing-layer', path: 'position.x', value: 1 };
const edits = [
  { name: 'property', command: { type: 'set_property', target: 'shape', path: 'position.x', value: 200 } },
  { name: 'keyframe', command: { type: 'replace_keyframes', target: 'shape', path: 'opacity', replace: false, keyframes: [{ time: .5, value: 50, ease: 'ease' }] } },
  { name: 'structural selection', command: { type: 'add_layer', id: 'temporary', layerType: 'text', select: true } }
];

it.each(edits)('rolls back a rejected live $name batch before it can commit an unrecorded mutation', ({ command }) => {
  const f = fixture(); const before = f.state();
  f.PM.Edit.begin('Gesture', { origin: 'inspector' });
  const result = f.PM.Edit.apply([command, rejected], { origin: 'inspector' });
  expect(result.ok).toBe(false);
  expect(result.message).toMatch(/Layer not found/);
  expect(f.state()).toEqual(before);
  expect(f.PM.hist.busy()).toBe(true);
  expect(f.PM.Edit.commit().ok).toBe(true);
  expect(f.state()).toEqual(before);
  expect(f.PM.hist.undo()).toBe(false);
});

it('can retry a rejected update to the same property without losing command coalescing or the gesture baseline', () => {
  const f = fixture(); const original = f.state();
  f.PM.Edit.begin('Gesture', { origin: 'inspector' });
  const position = (value: number) => ({ type: 'set_property', target: 'shape', path: 'position.y', value });
  expect(f.PM.Edit.apply(position(40), { origin: 'inspector' }).ok).toBe(true);
  const beforeFailure = f.state();
  expect(f.PM.Edit.apply([position(90), rejected], { origin: 'inspector' }).ok).toBe(false);
  expect(f.state()).toEqual(beforeFailure);
  expect(f.PM.Edit.apply(position(50), { origin: 'inspector' }).ok).toBe(true);
  expect(f.PM.Edit.commit().ok).toBe(true);
  const operations = f.PM.proj.edits.at(-1).operations;
  expect(operations).toHaveLength(1); expect(operations[0].value).toBe(50);
  expect(f.PM.hist.undo()).toBe(true); expect(f.state().project).toBe(original.project);
});

it('restores a failed batch with known layer targets without replacing unrelated layers', () => {
  const f = fixture();
  expect(f.PM.Edit.apply({ type: 'add_layer', id: 'other', layerType: 'solid', select: false }).ok).toBe(true);
  f.PM.hist.clear();
  const other = f.PM.L('other'), before = f.state();
  f.PM.Edit.begin('Gesture', { origin: 'inspector' });
  expect(f.PM.Edit.apply([
    { type: 'set_property', target: 'shape', path: 'position.x', value: 200 },
    { type: 'set_property', target: 'shape', path: 'unknown.property', value: 1 }
  ], { origin: 'inspector' }).ok).toBe(false);
  expect(f.state()).toEqual(before);
  expect(f.PM.L('other')).toBe(other);
  f.PM.Edit.cancel(); expect(f.PM.hist.undo()).toBe(false);
});

it('restores partial mutations from one rejected composition command', () => {
  const f = fixture(), before = f.state();
  f.PM.Edit.begin('Composition', { origin: 'inspector' });
  expect(f.PM.Edit.apply({ type: 'set_composition', patch: { name: 'Rejected rename', w: 'not a number' } }, { origin: 'inspector' }).ok).toBe(false);
  expect(f.state()).toEqual(before);
  f.PM.Edit.cancel(); expect(f.PM.hist.undo()).toBe(false);
});

it('removes optional source fields created by a rejected live batch', () => {
  const f = fixture(), before = f.state();
  f.PM.Edit.begin('Parameters', { origin: 'inspector' });
  expect(f.PM.Edit.apply([
    { type: 'set_scene_parameter', name: 'temporary', value: 2 },
    { type: 'set_property', target: 'shape', path: 'unknown.property', value: 1 }
  ], { origin: 'inspector' }).ok).toBe(false);
  expect(f.state()).toEqual(before);
  f.PM.Edit.cancel(); expect(f.PM.hist.undo()).toBe(false);
});

it.each(['update', 'style', 'language'].flatMap(kind => ['cue', 'style'].map(earlier => ({ kind, earlier }))))('restores failed caption $kind batches while preserving earlier successful $earlier updates', ({ kind, earlier }) => {
  const f = fixture();
  expect(f.PM.Edit.apply({ type: 'add_captions', id: 'captions', cues: [
    { id: 'first', start: 0, end: 1, text: 'First', words: [{ text: 'First', start: 0, end: 1 }] },
    { id: 'second', start: 1, end: 2, text: 'Second' }
  ] }).ok).toBe(true);
  f.PM.hist.clear();
  const original = f.state();
  f.PM.Edit.begin('Captions', { origin: 'timeline' });
  const update = (cues: any[]) => ({ type: 'edit_captions', target: 'captions', op: 'update', cues });
  const first = earlier === 'cue' ? update([{ id: 'first', text: 'Successful' }])
    : { type: 'edit_captions', target: 'captions', op: 'style', style: { fill: '#123456' } };
  expect(f.PM.Edit.apply(first, { origin: 'timeline' }).ok).toBe(true);
  const successful = f.state();
  const command = kind === 'style'
    ? { type: 'edit_captions', target: 'captions', op: 'style', style: { fill: '#00FF00' } }
    : { ...update([{ id: 'second', start: 1.1, end: 2.1 }]), ...(kind === 'language' ? { language: 'fr' } : {}) };
  expect(f.PM.Edit.apply([command, update([{ id: 'missing', text: 'Rejected' }])], { origin: 'timeline' }).ok).toBe(false);
  expect(f.state()).toEqual(successful);
  expect(f.PM.Edit.commit().ok).toBe(true);
  expect(f.PM.hist.list()).toHaveLength(1);
  expect(f.PM.proj.edits.at(-1).operations).toHaveLength(1);
  expect(f.PM.hist.undo()).toBe(true);
  expect(f.state().project).toBe(original.project);
});

it('uses defensive caption snapshots for mixed batches that mutate cue objects through layer changes', () => {
  const f = fixture();
  expect(f.PM.Edit.apply({ type: 'add_captions', id: 'captions', cues: [{ id: 'first', start: 0, end: 1, text: 'First' }] }).ok).toBe(true);
  f.PM.hist.clear(); const original = f.state();
  f.PM.Edit.begin('Captions', { origin: 'timeline' });
  expect(f.PM.Edit.apply([
    { type: 'edit_captions', target: 'captions', op: 'update', cues: [{ id: 'first', text: 'Changed' }] },
    { type: 'set_layer', target: 'captions', patch: { from: 2 } },
    rejected
  ], { origin: 'timeline' }).ok).toBe(false);
  expect(f.state()).toEqual(original);
  f.PM.Edit.cancel(); expect(f.PM.hist.undo()).toBe(false);
});

it.each(['captions', 'large SVG path'])('does not serialize unrelated %s content during warmed native property scrubs', kind => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 20000 }); PM.time = 0;
  if (kind === 'captions') {
    expect(PM.Edit.apply({ type: 'add_captions', id: 'target', cues: Array.from({ length: 3000 }, (_, index) => ({
      id: `cue${index}`, start: index * 3, end: index * 3 + 2.5, text: 'word '.repeat(10),
      words: Array.from({ length: 10 }, (_, word) => ({ text: 'word', start: index * 3 + word * .25, end: index * 3 + word * .25 + .2 }))
    })) }).ok).toBe(true);
  } else {
    const layer = PM.mkLayer('shape'); layer.id = 'target';
    layer.d.paths = [{ id: 'path', p: { closed: PM.P(false) }, vertices: Array.from({ length: 5000 }, (_, index) => ({
      id: `vertex${index}`, p: Object.fromEntries(Object.entries({ x: index, y: index % 10, inX: 0, inY: 0, outX: 0, outY: 0 }).map(([key, value]) => [key, PM.P(value)]))
    })) }];
    PM.addLayer(layer);
  }
  PM.hist.clear(); const original = JSON.stringify(PM.proj), content = PM.L('target').d;
  let serializations = 0;
  Object.defineProperty(content, 'toJSON', { configurable: true, value() { serializations++; return this; } });
  PM.Edit.begin('Opacity', { origin: 'inspector' });
  const opacity = (value: number) => ({ type: 'set_property', target: 'target', path: 'properties.opacity', value, preserveHandEdits: false, markIntent: 'human' });
  expect(PM.Edit.dispatch(opacity(99)).ok).toBe(true); // Warm the enclosing gesture's one history baseline.
  serializations = 0;
  for (let step = 0; step < 40; step++) expect(PM.Edit.dispatch(opacity(98 - step)).ok).toBe(true);
  expect(serializations).toBe(0);
  expect(PM.L('target').d).toBe(content);
  expect(PM.Edit.commit().ok).toBe(true);
  expect(PM.proj.edits.at(-1).operations).toHaveLength(1);
  expect(PM.hist.undo()).toBe(true); expect(JSON.stringify(PM.proj)).toBe(original);
});

it('rolls back native channel values, easing keys and newly marked intent while keeping earlier coalesced commands', () => {
  const f = fixture(), original = f.state();
  f.PM.Edit.begin('Native gesture', { origin: 'inspector' });
  const position = (value: unknown) => ({ type: 'set_property', target: 'shape', path: 'transform.position.y', value, preserveHandEdits: false, markIntent: 'human' });
  expect(f.PM.Edit.apply(position(40), { origin: 'inspector' }).ok).toBe(true);
  const successful = f.state();
  expect(f.PM.Edit.apply([
    position(90),
    { type: 'set_property', target: 'shape', path: 'opacity', value: 50, time: .5, mode: 'keyframe', ease: 'ease', markIntent: 'human' },
    { type: 'set_property', target: 'shape', path: 'position.x', value: 'not a number' }
  ], { origin: 'inspector' }).ok).toBe(false);
  expect(f.state()).toEqual(successful);
  expect(f.PM.Edit.apply(position(50), { origin: 'inspector' }).ok).toBe(true);
  expect(f.PM.Edit.commit().ok).toBe(true);
  expect(f.PM.proj.edits.at(-1).operations).toHaveLength(1);
  expect(f.PM.proj.edits.at(-1).operations[0].value).toBe(50);
  expect(f.PM.hist.undo()).toBe(true); expect(f.state().project).toBe(original.project);
});

it.each(['font', 'size'])('restores native anchor edits and %s anchor bounds if a later native value is rejected', kind => {
  const f = fixture();
  const layer = f.PM.L('shape');
  if (kind === 'font') layer.type = 'text';
  const key = kind === 'font' ? 'fontAnchorBounds' : 'sizeAnchorBounds';
  layer.d[key] = { x0: -100, x1: 100, y0: -50, y1: 50 };
  if (kind === 'font') f.PM.raster = () => ({ selection: { x0: -50, x1: 50, y0: -25, y1: 25 }, fontOffset: { x: 0, y: 0 } });
  const before = f.state();
  f.PM.Edit.begin('Anchor', { origin: 'canvas' });
  expect(f.PM.Edit.apply([
    { type: 'set_property', target: 'shape', path: 'anchor.x', value: 20, markIntent: 'human' },
    { type: 'set_property', target: 'shape', path: 'position.x', value: 'not a number' }
  ], { origin: 'canvas' }).ok).toBe(false);
  expect(f.state()).toEqual(before);
  f.PM.Edit.cancel(); expect(f.PM.hist.undo()).toBe(false);
});

it.each(edits)('preserves earlier successful edits when a later live $name batch fails, with one Undo', ({ command }) => {
  const f = fixture(); const original = f.state();
  f.PM.Edit.begin('Gesture', { origin: 'inspector' });
  expect(f.PM.Edit.apply({ type: 'set_property', target: 'shape', path: 'position.y', value: 40 }, { origin: 'inspector' }).ok).toBe(true);
  const successful = f.state();
  expect(f.PM.Edit.apply([command, rejected], { origin: 'inspector' }).ok).toBe(false);
  expect(f.state()).toEqual(successful);
  expect(f.PM.Edit.commit().ok).toBe(true);
  expect(f.PM.hist.list()).toHaveLength(1);
  expect(f.PM.hist.undo()).toBe(true);
  expect(f.state().project).toBe(original.project);
  expect(f.state().keys).toEqual(original.keys);
  expect(f.PM.hist.undo()).toBe(false);
  expect(f.PM.hist.redo()).toBe(true);
  expect(f.PM.L('shape').p['position.y'].v).toBe(40);
  expect(f.PM.L('shape').p['position.x'].v).toBe(f.startX);
  expect(f.PM.L('temporary')).toBeNull();
});
