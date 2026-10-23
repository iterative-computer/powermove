import { afterEach, expect, it, vi } from 'vitest';
import { EFFECTS } from '../../../../extensions/effects-basic/effects';
import { toLegacyFx } from '../../kernel/glsl';
import { Registry } from '../../kernel/registry';
import { trackStaticEffects } from './static-effects';
import { staticExportPlan, trackStaticExport } from './static-export';

const P = (v: unknown) => ({ v, kf: [] as any[] });
const mask = (shape = 'rect'): any => ({ id: 'mask', shape, mode: 'add', on: true,
  p: Object.fromEntries(Object.entries({ x: 0, y: 0, w: 81, h: 67, rotation: 11, feather: 3 }).map(([k, v]) => [k, P(v)])) });
const pathMask = (): any => ({ ...mask(), path: { p: { x: P(0), y: P(0), rotation: P(0), scaleX: P(100), scaleY: P(100) },
  vertices: [{ id: 'vertex', p: { x: P(-40), y: P(-33), inX: P(0), inY: P(0), outX: P(0), outY: P(0) } }] } });
afterEach(() => vi.unstubAllGlobals());

function fixture(paths = false) {
  let version = 0;
  const layer: any = { type: 'shape', from: 1, dur: 2, p: { rotation: P(7) }, masks: [paths ? pathMask() : mask()] };
  const progs = new Map<string, any>(['masks', 'maskApply', 'pathMaskApply'].map(name => [name, { pr: {}, u: new Map() }]));
  const gl = { isContextLost: vi.fn(() => false), getUniformLocation: vi.fn(() => null) };
  const PM: any = { proj: { layers: [layer], fps: 30, dur: 4, shutter: -.5 }, animVersion: () => version,
    quality: .5, FRAG_MASK: 'mask source', FRAG_MASK_APPLY: 'coverage source',
    GL: { gl, progs, errors: new Map(), compiling: 0 } };
  const tracker = trackStaticExport(PM, false);
  return { PM, layer, progs, gl, tracker, edit: () => version++, dispose: () => tracker.dispose() };
}

it('requires explicit mask approval in the pure planner while the runtime tracker validates built-ins', () => {
  const h = fixture();
  try {
    expect(staticExportPlan(h.PM.proj, false)).toBeNull();
    const plan = staticExportPlan(h.PM.proj, false, undefined, undefined, () => true)!;
    expect(plan(1.1)).toBe(plan(2)); expect(plan(.5)).not.toBe(plan(1));
    expect(h.tracker.key(1.1)).toBe(h.tracker.key(2)); expect(h.tracker.maxRepeat).toBe(1);
  } finally { h.dispose(); }
});

it.each([false, true])('uses only the programs that actually render a mixed list (path=%s)', paths => {
  const h = fixture(paths), get = vi.spyOn(h.progs, 'get');
  h.layer.masks.push(mask('ellipse'));
  try {
    if (paths) { h.progs.delete('masks'); h.progs.delete('maskApply'); }
    else h.progs.delete('pathMaskApply');
    expect(h.tracker.key(1.1)).not.toBeNull();
    expect(get.mock.calls.map(call => call[0])).toEqual(paths ? ['pathMaskApply'] : ['masks', 'maskApply']);
    expect(h.gl.getUniformLocation).toHaveBeenCalledTimes(paths ? 2 : 4);
  } finally { h.dispose(); }
});

it('deduplicates required programs across layers while preserving separate analytic/path branches', () => {
  const h = fixture(), get = vi.spyOn(h.progs, 'get');
  h.PM.proj.layers.push({ ...h.layer }, { ...h.layer, type: 'group', masks: [pathMask(), mask('ellipse')] });
  try {
    expect(h.tracker.key(1.1)).not.toBeNull();
    expect(get.mock.calls.map(call => call[0])).toEqual(['masks', 'maskApply', 'pathMaskApply']);
  } finally { h.dispose(); }
});

it.each(['masks', 'maskApply', 'pathMaskApply'] as const)('keeps a structural mask plan while %s is unavailable and recovers to a new key', name => {
  const h = fixture(name === 'pathMaskApply'), program = h.progs.get(name);
  try {
    h.progs.delete(name);
    expect(h.tracker.key(1.1)).toBeNull(); expect(h.tracker.hasStaticPlan).toBe(true); expect(h.tracker.maxRepeat).toBe(1);
    const revision = h.tracker.revision; expect(h.tracker.key(2)).toBeNull(); expect(h.tracker.revision).toBe(revision);
    h.progs.set(name, program); const ready = h.tracker.key(1.1); expect(ready).not.toBeNull();
    h.progs.set(name, null); expect(h.tracker.key(1.1)).toBeNull();
    h.progs.set(name, program); expect(h.tracker.key(1.1)).not.toBe(ready);
    const getGL = vi.fn(() => h.PM.GL), PM = h.PM, originalGL = PM.GL;
    getGL.mockImplementation(() => originalGL); Object.defineProperty(PM, 'GL', { get: getGL });
    for (let i = 0; i < 10; i++) expect(h.tracker.hasStaticPlan).toBe(true);
    expect(getGL).not.toHaveBeenCalled();
  } finally { h.dispose(); }
});

it.each(['record', 'program', 'context', 'wrapper', 'quality', 'FRAG_MASK', 'FRAG_MASK_APPLY'] as const)(
  'invalidates retained mask pixels after a live %s change without an edit event', change => {
    const h = fixture();
    try {
      const key = h.tracker.key(1.1), program = h.progs.get('maskApply'); expect(key).not.toBeNull();
      if (change === 'record') h.progs.set('maskApply', { ...program });
      if (change === 'program') program.pr = {};
      if (change === 'context') h.PM.GL.gl = { ...h.gl };
      if (change === 'wrapper') h.PM.GL = { ...h.PM.GL };
      if (change === 'quality') h.PM.quality = 1;
      if (change === 'FRAG_MASK' || change === 'FRAG_MASK_APPLY') h.PM[change] += ' changed';
      const next = h.tracker.key(1.1); expect(next).not.toBe(key); expect(next).not.toBeNull(); expect(h.tracker.key(2)).toBe(next);
    } finally { h.dispose(); }
  },
);

it('ignores unused analytic shader scalars for a path-only mask branch', () => {
  const h = fixture(true);
  try {
    const key = h.tracker.key(1.1); h.PM.FRAG_MASK += ' changed'; h.PM.FRAG_MASK_APPLY += ' changed';
    expect(h.tracker.key(2)).toBe(key);
  } finally { h.dispose(); }
});

it.each(['error', 'contextLost', 'isContextLost', 'compiling'] as const)('keeps %s mask failures serial and invalidates old pictures on recovery', change => {
  const h = fixture();
  try {
    const ready = h.tracker.key(1.1);
    if (change === 'error') h.PM.GL.errors.set('masks', 'Failed');
    if (change === 'contextLost') h.PM.GL.contextLost = true;
    if (change === 'isContextLost') h.gl.isContextLost.mockReturnValue(true);
    if (change === 'compiling') h.PM.GL.compiling = 1;
    expect(h.tracker.key(1.1)).toBeNull(); expect(h.tracker.hasStaticPlan).toBe(true);
    h.PM.GL.errors.clear(); h.PM.GL.contextLost = false; h.gl.isContextLost.mockReturnValue(false); h.PM.GL.compiling = 0;
    expect(h.tracker.key(1.1)).not.toBe(ready);
  } finally { h.dispose(); }
});

it.each(['vertex', 'mask', 'layer', 'quality'] as const)('refreshes the same timestamp after a held save and %s edit', async change => {
  const h = fixture(true);
  try {
    const key = h.tracker.key(1.1); let release!: () => void;
    const pendingSave = new Promise<void>(resolve => { release = resolve; });
    const afterAck = pendingSave.then(() => h.tracker.key(1.1));
    if (change === 'vertex') h.layer.masks[0].path.vertices[0].p.x.v += 31;
    if (change === 'mask') h.layer.masks[0].p.feather.v += 7;
    if (change === 'layer') h.PM.proj.layers = [{ ...h.layer }];
    if (change === 'quality') h.PM.quality = 1;
    else h.edit();
    release(); const fresh = await afterAck; expect(fresh).not.toBeNull(); expect(fresh).not.toBe(key);
    expect(h.tracker.key(2)).toBe(fresh); expect(h.tracker.maxRepeat).toBe(1);
  } finally { h.dispose(); }
});

it.each([
  { name: 'mask-list-object', masks: { length: 1 } }, { name: 'mask-list-string', masks: 'rect' },
  { name: 'null-mask', masks: [null] }, { name: 'array-mask', masks: [[]] },
  ...['shape', 'on', 'mode', 'p', 'path', 'vertices', 'vertex', 'vertex-p', 'path-p', 'expr', 'kf', 'vertex-expr', 'vertex-kf']
    .map(name => ({ name, masks: null })),
])('keeps unsupported or temporal $name masks per-frame without reading GL', ({ name, masks }) => {
  const h = fixture(true), m = h.layer.masks[0];
  try {
    if (masks) h.layer.masks = masks;
    if (name === 'shape') m.shape = 'triangle';
    if (name === 'on') m.on = false;
    if (name === 'mode') m.mode = { v: 'add', kf: [] };
    if (name === 'p') m.p = [];
    if (name === 'path') m.path = [];
    if (name === 'vertices') m.path.vertices = {};
    if (name === 'vertex') m.path.vertices[0] = [];
    if (name === 'vertex-p') m.path.vertices[0].p = [];
    if (name === 'path-p') m.path.p = [];
    if (name === 'expr') m.p.feather.expr = '1';
    if (name === 'kf') m.p.feather.kf = [{ t: 0, v: 3 }, { t: 1, v: 3 }];
    if (name === 'vertex-expr') m.path.vertices[0].p.x.expr = 'time';
    if (name === 'vertex-kf') m.path.vertices[0].p.x.kf = [{ t: 0, v: 0 }, { t: 1, v: 0 }];
    h.PM.GL = new Proxy({}, { get() { throw Error('Unsupported masks must not probe GL'); } });
    expect(h.tracker.key(1.1)).toBeNull(); expect(h.tracker.hasStaticPlan).toBe(false);
  } finally { h.dispose(); }
});

it.each([{ on: undefined }, { on: P(true) }, { mode: 'intersect' }, { shape: P('ellipse') }])('requires literal supported mask controls: %j', change => {
  const h = fixture(); Object.assign(h.layer.masks[0], change);
  try { expect(h.tracker.key(1.1)).toBeNull(); expect(h.tracker.hasStaticPlan).toBe(false); }
  finally { h.dispose(); }
});

it('preserves single keys and grouped negative-shutter boundary sampling for static paths', () => {
  const h = fixture(true);
  h.layer.masks[0].path.vertices[0].p.x.kf = [{ t: 0, v: -40 }];
  h.PM.proj.layers.unshift({ type: 'group', from: 0, dur: 4, mblur: true });
  const tracker = trackStaticExport(h.PM, true);
  try {
    expect(tracker.key(1.1)).toBe(tracker.key(2));
    expect(new Set([.99, .995, .999].map(time => tracker.key(time))).size).toBe(3);
    const forward = trackStaticExport({ ...h.PM, proj: { ...h.PM.proj, shutter: .5 } }, true);
    try { for (const time of [.1, .99, .995, .999, 1.1, 2]) expect(tracker.key(time)).toBe(forward.key(time)); }
    finally { forward.dispose(); }
  } finally { tracker.dispose(); h.dispose(); }
});

it('retains simultaneous mask and FX guards, including shared legacy default mutation', () => {
  const h = fixture(true), root = structuredClone(EFFECTS.find(effect => effect.id === 'blur')!);
  const definition = root;
  const registry = new Registry<any>(); registry.register('effects-basic', root);
  h.PM.Kernel = { effects: registry }; h.PM.FX = { blur: toLegacyFx(definition, '') };
  h.PM.effectDef = (effect: any) => effect.type === 'blur' && (effect.version ?? 1) === 1 ? h.PM.FX.blur : undefined;
  h.layer.fx = [{ type: 'blur', on: true, p: {} }]; h.progs.set('fx:blur', { pr: {}, u: new Map() });
  try {
    const key = h.tracker.key(1.1); expect(key).not.toBeNull();
    expect(h.gl.getUniformLocation).toHaveBeenCalledTimes(4);
    definition.params[0]!.def = 89; expect(h.tracker.key(1.1)).not.toBe(key);
    registry.register('custom', definition); expect(h.tracker.key(1.1)).toBeNull();
  } finally { h.dispose(); }
});

it('preserves image source freshness while retaining a static mask picture', () => {
  class Bitmap { width = 128; height = 64; }
  vi.stubGlobal('ImageBitmap', Bitmap);
  const h = fixture(), asset = { id: 'picture', kind: 'image', format: 'png', el: new Bitmap(), w: 128, h: 64, url: 'blob:picture' };
  h.layer.type = 'image'; h.layer.d = { asset: 'picture' }; h.PM.assets = { get: () => asset };
  try {
    const key = h.tracker.key(1.1); expect(key).not.toBeNull();
    asset.el = new Bitmap(); expect(h.tracker.key(1.1)).not.toBe(key);
    asset.el.width = 0; expect(h.tracker.key(2)).toBeNull();
  } finally { h.dispose(); }
});

it('does not require an effect registry for masks-only, and releases snapshots on disposal', () => {
  const h = fixture(), guard = trackStaticEffects(h.PM);
  h.PM.Kernel = { get effects(): never { throw Error('Masks have no provider'); } };
  try {
    const stamp = guard.capture([], ['masks', 'maskApply']); expect(stamp).not.toBeNull();
    expect(guard.capture([], ['masks', 'maskApply'])).toBe(stamp); expect(h.gl.getUniformLocation).toHaveBeenCalledTimes(4);
    expect(h.tracker.key(1.1)).not.toBeNull(); guard.dispose(); guard.dispose();
    expect(guard.capture([], ['masks'])).toBeNull(); expect(h.progs.size).toBe(3);
  } finally { guard.dispose(); h.dispose(); }
});

it.each(['masks', 'maskApply', 'pathMaskApply'].flatMap(name =>
  ['u_time', 'u_prog'].map(uniform => ({ name, uniform })),
))('does not retain a replacement $name program with a live $uniform input', ({ name, uniform }) => {
  const h = fixture(name === 'pathMaskApply');
  try {
    expect(h.tracker.key(1.1)).not.toBeNull();
    const program = { pr: {}, u: new Map([['u_time', null], ['u_prog', null]]) };
    h.progs.set(name, program);
    h.gl.getUniformLocation.mockImplementation((...args: unknown[]) => args[0] === program.pr && args[1] === uniform ? {} as any : null);
    expect(h.tracker.key(1.1)).toBeNull(); expect(h.tracker.hasStaticPlan).toBe(true);
  } finally { h.dispose(); }
});
