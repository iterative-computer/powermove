import { expect, it, vi } from 'vitest';
import { EFFECTS } from '../../../../extensions/effects-basic/effects';
import type { EffectDefinition } from '../../kernel/api';
import { toLegacyFx } from '../../kernel/glsl';
import { Registry } from '../../kernel/registry';
import { staticEffectSupported, trackStaticEffects } from './static-effects';
import { staticExportPlan, trackStaticExport } from './static-export';

function fixture(ids = ['blur']) {
  const registry = new Registry<EffectDefinition>(), legacy: Record<string, any> = {};
  const progs = new Map<string, any>(), locations = new Map<object, Set<string>>();
  for (const id of ids) {
    const definition = structuredClone(EFFECTS.find(effect => effect.id === id)!);
    registry.register('effects-basic', definition); legacy[id] = toLegacyFx(definition, '');
    progs.set('fx:' + id, { pr: {}, u: new Map() });
  }
  const gl = { isContextLost: vi.fn(() => false),
    getUniformLocation: vi.fn((program: object, name: string) => locations.get(program)?.has(name) ? {} : null) };
  const PM: any = { Kernel: { effects: registry }, FX: legacy, quality: .5,
    GL: { gl, progs, errors: new Map(), compiling: 0 },
    proj: { fps: 30, dur: 4, layers: [{ type: 'shape', from: 1, dur: 2,
      p: { opacity: { v: 100, kf: [] } }, fx: ids.map(type => ({ type, version: registry.get(type)?.version ?? 1, on: { v: true }, p: {} })) }] } };
  const guard = trackStaticEffects(PM), tracker = trackStaticExport(PM, false);
  return { PM, gl, registry, legacy, progs, locations, guard, tracker,
    dispose() { guard.dispose(); tracker.dispose(); } };
}

it('requires explicit effect approval in the pure planner and preserves constant/edge rules', () => {
  const h = fixture();
  try {
    expect(staticExportPlan(h.PM.proj, false)).toBeNull();
    const plan = staticExportPlan(h.PM.proj, false, undefined, fx => staticEffectSupported(fx.type))!;
    expect(plan(1.1)).toBe(plan(2)); expect(plan(.5)).not.toBe(plan(1));
    h.PM.proj.layers[0].fx[0].p.amount = { kf: [{ t: 0, v: 24 }, { t: 1, v: 24 }] };
    expect(staticExportPlan(h.PM.proj, false, undefined, () => true)).toBeNull();
    h.PM.proj.layers[0].fx[0].p.amount = { expr: '1' };
    expect(staticExportPlan(h.PM.proj, false, undefined, () => true)).toBeNull();
  } finally { h.dispose(); }
});

it.each([{ length: 1 }, { length: 0 }, 'blur', [null]].map(fx => ({ fx })))('keeps malformed FX serial without invoking array methods: $fx', ({ fx }) => {
  const h = fixture();
  try {
    h.PM.proj.layers[0].fx = fx;
    expect(h.tracker.key(1.1)).toBeNull();
  } finally { h.dispose(); }
});

it('preserves grouped negative-shutter boundary sampling with an approved constant FX graph', () => {
  const h = fixture();
  try {
    h.PM.proj.shutter = -.5;
    h.PM.proj.layers.unshift({ type: 'group', from: 0, dur: 4, mblur: true });
    const fx = staticExportPlan(h.PM.proj, true, undefined, effect => staticEffectSupported(effect.type))!;
    const ordinary = staticExportPlan({ ...h.PM.proj,
      layers: h.PM.proj.layers.map((layer: any) => ({ ...layer, fx: [] })) }, true)!;
    const times = [.1, .99, .995, .999, 1.1, 2, 2.995, 3.01, 4];
    for (const time of times) expect(fx(time)).toBe(ordinary(time));
    expect(new Set([.99, .995, .999].map(time => fx(time))).size).toBe(3);
  } finally { h.dispose(); }
});

it('accepts twenty-six reviewed stationary IDs and keeps both temporal built-ins per-frame', () => {
  const ids = EFFECTS.map(effect => effect.id).filter(staticEffectSupported);
  expect(ids).toHaveLength(26);
  const h = fixture(ids);
  try {
    const stamp = h.guard.capture(ids);
    expect(stamp).not.toBeNull(); expect(h.guard.capture(ids)).toBe(stamp);
    expect(h.gl.getUniformLocation).toHaveBeenCalledTimes(52);
    expect(h.tracker.key(1.1)).toBe(h.tracker.key(2)); expect(h.tracker.maxRepeat).toBe(1);
    for (const id of ['grain', 'displace', 'custom', '__proto__']) {
      expect(staticEffectSupported(id)).toBe(false); expect(h.guard.capture([id])).toBeNull();
    }
  } finally { h.dispose(); }
});

it('leaves cold or pending programs uncached, then changes generation when real pictures become ready', () => {
  const h = fixture(), program = h.progs.get('fx:blur');
  try {
    h.progs.delete('fx:blur');
    expect(h.tracker.key(1.1)).toBeNull(); const coldRevision = h.tracker.revision;
    expect(h.tracker.key(2)).toBeNull(); expect(h.tracker.revision).toBe(coldRevision);
    h.progs.set('fx:blur', program); h.PM.GL.compiling = 1;
    expect(h.tracker.key(1.1)).toBeNull();
    h.PM.GL.compiling = 0;
    const ready = h.tracker.key(1.1);
    expect(ready).not.toBeNull(); expect(h.tracker.key(2)).toBe(ready);
    h.progs.delete('fx:blur'); expect(h.tracker.key(1.1)).toBeNull();
    h.progs.set('fx:blur', program);
    expect(h.tracker.key(1.1)).not.toBe(ready);
    expect(h.tracker.revision).toBeGreaterThan(coldRevision);
  } finally { h.dispose(); }
});

it.each(['u_time', 'u_prog'])('rejects a same-owner replacement whose actual linked program uses %s', uniform => {
  const h = fixture();
  try {
    const ready = h.tracker.key(1.1), replacement = structuredClone(h.registry.get('blur')!);
    replacement.frag += '\n// replacement'; h.registry.register('effects-basic', replacement);
    h.PM.FX.blur = toLegacyFx(replacement, '');
    const program = { pr: {}, u: new Map() }; h.locations.set(program.pr, new Set([uniform]));
    h.progs.set('fx:blur', program);
    expect(h.tracker.key(1.1)).toBeNull();
    h.progs.set('fx:blur', { pr: {}, u: new Map() });
    expect(h.tracker.key(1.1)).not.toBe(ready);
  } finally { h.dispose(); }
});

it('checks the actual replacement program even when a reused record retains stale uniform locations', () => {
  const h = fixture();
  try {
    expect(h.tracker.key(1.1)).not.toBeNull();
    const record = h.progs.get('fx:blur'); record.u.set('u_time', null); record.u.set('u_prog', null);
    record.pr = {}; h.locations.set(record.pr, new Set(['u_time']));
    expect(h.tracker.key(1.1)).toBeNull();
  } finally { h.dispose(); }
});

it('rejects context loss during uniform inspection and recovers to a new key', () => {
  const h = fixture();
  try {
    const ready = h.tracker.key(1.1);
    h.progs.set('fx:blur', { pr: {}, u: new Map() });
    h.gl.getUniformLocation.mockImplementation(() => { h.gl.isContextLost.mockReturnValue(true); return null; });
    expect(h.tracker.key(1.1)).toBeNull();
    h.gl.isContextLost.mockReturnValue(false);
    h.gl.getUniformLocation.mockImplementation(() => null);
    expect(h.tracker.key(1.1)).not.toBe(ready);
    expect(h.gl.getUniformLocation).toHaveBeenCalledTimes(6);
  } finally { h.dispose(); }
});

it('invalidates static FX pictures after quality changes during an outstanding save without an edit event', async () => {
  const h = fixture();
  try {
    const captured = h.tracker.key(1.1); expect(captured).not.toBeNull(); expect(h.PM.quality).toBe(.5);
    let release!: () => void; const ack = new Promise<void>(resolve => { release = resolve; });
    const afterSave = ack.then(() => h.tracker.key(1.1));
    h.PM.quality = 1; release();
    const fresh = await afterSave; expect(fresh).not.toBe(captured); expect(fresh).not.toBeNull();
    expect(h.tracker.key(2)).toBe(fresh); expect(h.tracker.maxRepeat).toBe(1);
  } finally { h.dispose(); }
});

it.each(['record', 'program', 'context', 'wrapper', 'legacy', 'frag', 'passes', 'keepOrig', 'params', 'default'])(
  'invalidates an existing capture when its live %s changes without an event', change => {
    const h = fixture();
    try {
      const key = h.tracker.key(1.1); expect(key).not.toBeNull();
      const program = h.progs.get('fx:blur'), legacy = h.PM.FX.blur;
      if (change === 'record') h.progs.set('fx:blur', { ...program });
      if (change === 'program') program.pr = {};
      if (change === 'context') h.PM.GL.gl = { ...h.gl };
      if (change === 'wrapper') h.PM.GL = { ...h.PM.GL };
      if (change === 'legacy') h.PM.FX.blur = { ...legacy };
      if (change === 'frag') legacy.frag += '\n// changed';
      if (change === 'passes') legacy.passes++;
      if (change === 'keepOrig') legacy.keepOrig = !legacy.keepOrig;
      if (change === 'params') legacy.params = legacy.params.map((param: any) => ({ ...param }));
      if (change === 'default') legacy.params[0].def = 89;
      expect(h.tracker.key(1.1)).not.toBe(key);
      if (change === 'params') expect(h.tracker.key(2)).toBeNull();
      else expect(h.tracker.key(2)).toBe(h.tracker.key(1.1));
    } finally { h.dispose(); }
  },
);

it.each(['backdrop', 'error', 'context-loss', 'missing-legacy', 'missing-program'])(
  'keeps %s failures serial and recovers without retaining a stale picture', change => {
    const h = fixture();
    try {
      const ready = h.tracker.key(1.1), legacy = h.PM.FX.blur, program = h.progs.get('fx:blur');
      if (change === 'backdrop') legacy.backdrop = true;
      if (change === 'error') h.PM.GL.errors.set('fx:blur', 'compile failed');
      if (change === 'context-loss') h.gl.isContextLost.mockReturnValue(true);
      if (change === 'missing-legacy') delete h.PM.FX.blur;
      if (change === 'missing-program') h.progs.set('fx:blur', null);
      expect(h.tracker.key(1.1)).toBeNull();
      legacy.backdrop = false; h.PM.GL.errors.clear(); h.gl.isContextLost.mockReturnValue(false);
      h.PM.FX.blur = legacy; h.progs.set('fx:blur', program);
      expect(h.tracker.key(1.1)).not.toBe(ready);
    } finally { h.dispose(); }
  },
);

it('observes custom and same-owner provider edits even inside suppressed registry batches', () => {
  const h = fixture();
  try {
    let ready = h.tracker.key(1.1);
    const batch = h.registry.batchChanges(), item = h.registry.get('blur')!;
    const override = h.registry.register('custom', item);
    expect(h.tracker.key(1.1)).toBeNull(); override.dispose();
    expect(h.tracker.key(1.1)).not.toBe(ready); ready = h.tracker.key(1.1);
    h.registry.register('effects-basic', item); batch.dispose();
    expect(h.tracker.key(1.1)).not.toBe(ready);
  } finally { h.dispose(); }
});

it('rejects cached legacy parameter divergence before capture and default mutations afterward', () => {
  const h = fixture();
  try {
    const item = h.registry.get('blur')!, oldParams = h.legacy.blur.params;
    item.params = item.params.map(param => ({ ...param }));
    expect(h.tracker.key(1.1)).toBeNull();
    oldParams[0].def = 89; expect(h.tracker.key(2)).toBeNull();
    h.PM.FX.blur = { ...h.legacy.blur, params: item.params };
    const ready = h.tracker.key(1.1); expect(ready).not.toBeNull();
    item.params[0]!.def = 72; expect(h.tracker.key(1.1)).not.toBe(ready);
  } finally { h.dispose(); }
});

it('scopes uniform checks and resources to one export and does not add registry subscriptions', () => {
  const h = fixture(), subscribe = vi.spyOn(h.registry, 'onChange');
  const stamp = h.guard.capture(['blur']);
  expect(stamp).not.toBeNull(); h.guard.capture(['blur']);
  expect(h.gl.getUniformLocation).toHaveBeenCalledTimes(2);
  expect(subscribe).not.toHaveBeenCalled(); h.dispose(); h.dispose();
  expect(h.guard.capture(['blur'])).toBeNull(); expect(h.tracker.key(1.1)).toBeNull();
  // GL owns its cache. Disposing the export never deletes a live program.
  expect(h.progs.size).toBe(1);
});

it('releases a previous FX snapshot on structural replanning and checks programs again if FX returns', () => {
  const h = fixture();
  try {
    const project = h.PM.proj;
    expect(h.tracker.key(1.1)).not.toBeNull(); expect(h.gl.getUniformLocation).toHaveBeenCalledTimes(2);
    h.PM.proj = { layers: [{ type: 'solid', from: 0, dur: 3 }] };
    expect(h.tracker.key(1.1)).not.toBeNull(); expect(h.tracker.maxRepeat).toBe(8);
    h.PM.proj = project;
    expect(h.tracker.key(1.1)).not.toBeNull(); expect(h.gl.getUniformLocation).toHaveBeenCalledTimes(4);
  } finally { h.dispose(); }
});

it('does not probe programs or providers for artwork without FX or a nonconstant FX graph', () => {
  const h = fixture();
  try {
    h.PM.proj.layers[0].p.opacity.kf = [{ t: 0, v: 100 }, { t: 1, v: 100 }];
    h.PM.GL = new Proxy({}, { get() { throw Error('No temporal graph should query GL'); } });
    expect(h.tracker.key(1.1)).toBeNull();
    h.PM.proj = { layers: [{ type: 'solid', from: 0, dur: 3 }] };
    Object.defineProperty(h.PM.Kernel, 'effects', { get() { throw Error('No FX should query providers'); } });
    expect(h.tracker.key(1.1)).toBe(h.tracker.key(2)); expect(h.tracker.maxRepeat).toBe(8);
  } finally { h.dispose(); }
});

it('retains and invalidates the exact historical program when a source changes revision without a project event', () => {
  const h = fixture(['invert']);
  try {
    const root = h.registry.get('invert')!, original = root.legacy![0]!;
    const historical = toLegacyFx(original, '');
    h.PM.effectDef = ({ version }: any) => version === 1 ? historical : h.legacy.invert;
    h.progs.set('fx:invert@1', { pr: {}, u: new Map() });
    const source = h.PM.proj.layers[0].fx[0]; source.version = 1;
    const first = h.tracker.key(1.1);
    expect(first).not.toBeNull(); expect(h.tracker.key(2)).toBe(first);
    source.version = 2;
    expect(h.tracker.key(1.1)).not.toBe(first);
    const second = h.tracker.key(2);
    expect(second).not.toBeNull();
    h.progs.delete('fx:invert');
    expect(h.tracker.key(1.1)).toBeNull();
  } finally { h.dispose(); }
});
