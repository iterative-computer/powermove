import { afterEach, describe, expect, it, vi } from 'vitest';
import { staticExportPlan, trackStaticExport } from './static-export';
import { createEngine } from '../../player/player';

const solid = () => ({ type: 'solid', from: 1, dur: 2, p: { opacity: { v: 100, kf: [] } } });
afterEach(() => vi.unstubAllGlobals());

describe('static export planning', () => {
  it('retains a background and splits pictures at layer entrances and exits', () => {
    const background = staticExportPlan({ layers: [] }, false)!;
    expect(background(0)).toBe(background(10));
    const plan = staticExportPlan({ layers: [solid()] }, false)!;
    expect(plan(.5)).not.toBe(plan(1));
    expect(plan(1)).toBe(plan(2.9));
    expect(plan(2.9)).not.toBe(plan(3));
  });

  it('requires explicit image approval while preserving the ordinary static checks', () => {
    const image = { ...solid(), type: 'image', d: { asset: 'picture' } }, project = { layers: [image] };
    expect(staticExportPlan(project, false)).toBeNull();
    expect(staticExportPlan(project, false, () => false)).toBeNull();
    const plan = staticExportPlan(project, false, () => true)!;
    expect(plan(1)).toBe(plan(2));
    expect(plan(.5)).not.toBe(plan(1));
    for (const change of [{ fx: [{}] }, { masks: [{}] }, { threeD: true },
      { p: { rotation: { kf: [{ t: 0, v: 0 }, { t: 1, v: 20 }] } } }]) {
      expect(staticExportPlan({ layers: [{ ...image, ...change }] }, false, () => true)).toBeNull();
    }
  });

  it('preserves group image boundaries under negative-shutter motion blur', () => {
    const project = { fps: 30, dur: 4, shutter: -.5, layers: [
      { ...solid(), type: 'group', from: 0, dur: 4, mblur: true },
      { ...solid(), type: 'image', d: { asset: 'picture' } },
    ] };
    const plan = staticExportPlan(project, true, () => true)!;
    expect(plan(.99)).not.toBe(plan(.995));
    expect(plan(.995)).not.toBe(plan(.999));
    expect(plan(1.1)).toBe(plan(2));
    const forward = staticExportPlan({ ...project, shutter: .5 }, true, () => true)!;
    for (const time of [.99, .995, .999, 1.1, 2]) expect(plan(time)).toBe(forward(time));
  });

  it.each([
    { type: 'video' }, { type: 'image' }, { type: 'precomp' }, { type: 'shader' }, { type: 'extension' },
    { fx: [{}] }, { transitionIn: {} }, { threeD: true }, { masks: [{}] }, { matteSource: 'other' },
    { d: { animators: [{}] } }, { p: { rotation: { expr: 'time' } } },
    { p: { rotation: { kf: [{ t: 0, v: 0 }, { t: 1, v: 0, spring: true }] } } },
  ])('keeps temporal or unsupported artwork on the normal path: %j', change => {
    expect(staticExportPlan({ layers: [{ ...solid(), ...change }] }, false)).toBeNull();
  });

  it('keeps audio independent and renders each blur frame around a boundary', () => {
    const plan = staticExportPlan({ fps: 30, layers: [{ ...solid(), mblur: true }, { type: 'audio' }] }, true)!;
    expect(plan(.99)).not.toBe(plan(1));
    expect(plan(1)).not.toBe(plan(1.01));
    expect(plan(1.1)).toBe(plan(2));
  });

  it('matches the active-layer tolerance at fractional frame entrances and exits', () => {
    const plan = staticExportPlan({ dur: 1, layers: [{ ...solid(), from: 1 / 30 + .5e-6, dur: 1 / 30 }] }, false)!;
    expect(plan(0)).not.toBe(plan(1 / 30));
    expect(plan(1 / 30)).not.toBe(plan(2 / 30));
    expect(plan(2 / 30)).toBe(plan(3 / 30));
    const end = staticExportPlan({ dur: 1, layers: [{ ...solid(), from: 0, dur: 1 }] }, false)!;
    expect(end(1)).not.toBe(end(1 + 2e-6));
  });

  it('keeps changing group blur pictures distinct when a negative shutter reverses sample order', () => {
    const PM = createEngine(null);
    const project = PM.mkProject({ fps: 30, dur: 3 });
    project.shutter = -.5; PM.proj = project;
    const group = PM.mkLayer('group', { from: 0, dur: 3 }, project);
    const child = PM.mkLayer('solid', { from: 1, dur: 1 }, project);
    group.mblur = true; child.group = group.id;
    project.layers = [group, child];
    const plan = staticExportPlan(project, true)!;
    const times = [.99, .995, .999];
    // Match the compositor's twenty group-blur samples and real layer gate.
    const samples = times.map(time => Array.from({ length: 20 }, (_, sample) => {
      const offset = ((sample + .5) / 20 - .5) * project.shutter / project.fps;
      return Number(PM.active(child, time + offset));
    }).reduce((sum, visible) => sum + visible, 0));
    expect(samples).toEqual([0, 4, 9]);
    expect(new Set(times.map(time => plan(time))).size).toBe(times.length);
    expect(plan(.1)).toBe(plan(.2));
    const forward = staticExportPlan({ ...project, shutter: .5 }, true)!;
    for (const time of times) expect(plan(time)).toBe(forward(time));
  });
});

it('invalidates retained pictures after edits, font loads and renderer changes, and releases listeners', () => {
  const fonts = Object.assign(new EventTarget(), { status: 'loaded', size: 0 });
  const add = vi.spyOn(fonts, 'addEventListener'), remove = vi.spyOn(fonts, 'removeEventListener');
  const handlers = new Map<string, () => void>();
  const off = vi.fn();
  const PM = { proj: { layers: [solid()] }, animVersion: () => version,
    bus: { on: (event: string, handler: () => void) => { handlers.set(event, handler); return off; } },
    Kernel: { events: { on: (event: string, handler: () => void) => { handlers.set(event, handler); return { dispose: off }; } } } };
  let version = 0;
  vi.stubGlobal('document', { fonts });
  const retained = trackStaticExport(PM, false);
  try {
    let previous = retained.key(1.5);
    expect(retained.key(2)).toBe(previous);
    for (const event of ['fonts', 'layers', 'project', 'draw', 'extensions:changed', 'theme:changed']) {
      handlers.get(event)!();
      const next = retained.key(1.5); expect(next).not.toBe(previous); previous = next;
    }
    version++;
    expect(retained.key(1.5)).not.toBe(previous); previous = retained.key(1.5);
    fonts.size++;
    expect(retained.key(1.5)).not.toBe(previous); previous = retained.key(1.5);
    fonts.status = 'loading'; fonts.dispatchEvent(new Event('loading'));
    expect(retained.key(1.5)).toBeNull();
    fonts.status = 'loaded'; fonts.dispatchEvent(new Event('loadingdone'));
    expect(retained.key(1.5)).not.toBe(previous);
    expect(add).toHaveBeenCalledTimes(3);
  } finally { retained.dispose(); vi.unstubAllGlobals(); }
  expect(remove).toHaveBeenCalledTimes(3);
  expect(off).toHaveBeenCalledTimes(6);
});

class Bitmap {
  width = 128; height = 64;
  close = vi.fn(() => { this.width = this.height = 0; });
}

function imageFixture() {
  vi.stubGlobal('ImageBitmap', Bitmap);
  const layer = { ...solid(), id: 'image', type: 'image', d: { asset: 'picture' } };
  const asset = { id: 'picture', kind: 'image', format: 'png', el: new Bitmap(), w: 128, h: 64, url: 'blob:picture' };
  const assets = new Map<string, any>([['picture', asset]]), handlers = new Map<string, Set<() => void>>();
  const off = vi.fn();
  const on = (event: string, handler: () => void) => {
    const listeners = handlers.get(event) || new Set(); handlers.set(event, listeners); listeners.add(handler);
    return () => { listeners.delete(handler); off(); };
  };
  let version = 1;
  const PM: any = { proj: { layers: [layer], fps: 30, dur: 4 }, animVersion: () => version,
    assets: { get: (id: string) => assets.get(id) }, bus: { on }, Kernel: { events: { on } } };
  const tracker = trackStaticExport(PM, false);
  return { PM, layer, asset, assets, handlers, tracker, off, edit: () => version++,
    emit: (event: string) => handlers.get(event)?.forEach(handler => handler()) };
}

it('retains immutable images, changes keys on source replacement, and limits image repeats to one ACK', () => {
  const h = imageFixture();
  try {
    let previous = h.tracker.key(1.1);
    expect(previous).not.toBeNull(); expect(h.tracker.key(2)).toBe(previous);
    expect(h.tracker.maxRepeat).toBe(1);
    const revision = h.tracker.revision;
    h.asset.el = new Bitmap();
    expect(h.tracker.key(1.1)).not.toBe(previous); previous = h.tracker.key(1.1);
    expect(h.tracker.revision).toBe(revision + 1);
    h.assets.set('picture', { ...h.asset });
    expect(h.tracker.key(1.1)).not.toBe(previous); previous = h.tracker.key(1.1);
    h.assets.get('picture').w++;
    expect(h.tracker.key(1.1)).not.toBe(previous); previous = h.tracker.key(1.1);
    h.emit('assets');
    expect(h.tracker.key(1.1)).not.toBe(previous);
  } finally { h.tracker.dispose(); }
});

it('checks every image alias even when several assets share the same immutable bitmap', () => {
  const h = imageFixture();
  const alias = { ...h.asset, id: 'alias' };
  h.assets.set('alias', alias);
  h.PM.proj.layers.push({ ...h.layer, id: 'second', d: { asset: 'alias' } });
  try {
    const previous = h.tracker.key(1.1);
    expect(previous).not.toBeNull(); expect(h.tracker.key(2)).toBe(previous);
    alias.el = new Bitmap();
    expect(h.tracker.key(1.1)).not.toBe(previous);
  } finally { h.tracker.dispose(); }
});

it.each(['video', 'animated-image'])('replans a replaced layer array without an event or animation version: %s', kind => {
  const h = imageFixture();
  try {
    expect(h.tracker.key(1.1)).not.toBeNull();
    h.PM.proj.layers = kind === 'video' ? [{ ...h.layer, type: 'video' }] : [{ ...h.layer,
      p: { rotation: { kf: [{ t: 0, v: 0 }, { t: 1, v: 20 }] } } }];
    expect(h.tracker.key(1.1)).toBeNull();
  } finally { h.tracker.dispose(); }
});

it('replans changed layer counts and background identities without an edit event', () => {
  const h = imageFixture();
  try {
    const first = h.tracker.key(1.1);
    h.PM.proj.backgroundFill = { type: 'solid', stops: [{ color: '#123456' }] };
    expect(h.tracker.key(1.1)).not.toBe(first);
    h.PM.proj.layers.push({ ...h.layer, type: 'video' });
    expect(h.tracker.key(1.1)).toBeNull();
  } finally { h.tracker.dispose(); }
});

it('recovers unavailable and closed bitmaps without rescanning the structural graph', () => {
  const h = imageFixture();
  let scans = 0;
  Object.defineProperty(h.layer.p, 'expensive', { enumerable: true,
    get: () => { scans++; return { v: 1, kf: [{ t: 0, v: 1 }] }; } });
  h.asset.el = null as any;
  try {
    expect(h.tracker.key(1.1)).toBeNull();
    const revision = h.tracker.revision, firstScans = scans;
    expect(firstScans).toBeGreaterThan(0);
    for (let frame = 0; frame < 20; frame++) expect(h.tracker.key(1.1)).toBeNull();
    expect(h.tracker.revision).toBe(revision); expect(scans).toBe(firstScans);
    h.asset.el = new Bitmap();
    const available = h.tracker.key(1.1)!;
    expect(available).not.toBeNull(); expect(h.tracker.key(2)).toBe(available);
    h.asset.el.close();
    expect(h.tracker.key(1.1)).toBeNull();
    const closedRevision = h.tracker.revision;
    expect(h.tracker.key(1.1)).toBeNull(); expect(h.tracker.revision).toBe(closedRevision);
    h.asset.el = new Bitmap();
    expect(h.tracker.key(1.1)).not.toBe(available);
    h.emit('assets'); h.tracker.key(1.1);
    expect(scans).toBe(firstScans);
    h.edit(); h.tracker.key(1.1);
    expect(scans).toBeGreaterThan(firstScans);
  } finally { h.tracker.dispose(); }
});

it.each(['svg', 'mutable', 'sequence', 'property-id'])('keeps unsupported image %s sources uncached', kind => {
  const h = imageFixture();
  if (kind === 'svg') h.asset.format = 'svg';
  if (kind === 'mutable') h.asset.el = { width: 128, height: 64 } as any;
  if (kind === 'sequence') (h.asset as any).imageSequence = { fps: 30, frames: 10 };
  if (kind === 'property-id') h.layer.d.asset = { v: 'picture', kf: [] } as any;
  try { expect(h.tracker.key(1.1)).toBeNull(); }
  finally { h.tracker.dispose(); }
});

it('leaves animated image plans on the ordinary path without probing runtime sources', () => {
  const h = imageFixture();
  (h.layer.p as any).rotation = { kf: [{ t: 0, v: 0 }, { t: 1, v: 1 }] };
  h.PM.assets.get = vi.fn(() => { throw new Error('Should not inspect temporal image assets'); });
  try {
    expect(h.tracker.key(1.1)).toBeNull(); expect(h.tracker.key(2)).toBeNull();
    expect(h.PM.assets.get).not.toHaveBeenCalled();
  } finally { h.tracker.dispose(); }
});

it('keeps built-in repeat batching and avoids constructing a media tracker for ordinary artwork', () => {
  const PM = { proj: { layers: [solid()] }, bus: { on: vi.fn() },
    get assets(): never { throw new Error('Artwork does not need media'); } };
  const tracker = trackStaticExport(PM, false);
  try {
    expect(tracker.key(1.1)).toBe(tracker.key(2)); expect(tracker.maxRepeat).toBe(8);
    expect(PM.bus.on.mock.calls.some(call => call[0] === 'assets')).toBe(false);
  } finally { tracker.dispose(); }
});

it('disposes image subscriptions once without closing asset-owned pixels or reviving the tracker', () => {
  const h = imageFixture();
  h.tracker.key(1.1);
  expect(h.handlers.get('assets')?.size).toBe(1);
  h.tracker.dispose(); h.tracker.dispose();
  expect(h.off).toHaveBeenCalledTimes(7);
  expect(h.asset.el.close).not.toHaveBeenCalled();
  expect(h.handlers.get('assets')?.size).toBe(0);
  expect(h.tracker.key(1.1)).toBeNull();
  expect(h.handlers.get('assets')?.size).toBe(0);
});


it('distinguishes a structural static effects plan from missing program readiness without querying GL in its getter', () => {
  let version = 0;
  const layer = { ...solid(), fx: [{ type: 'blur', on: true, p: { amount: { v: 13, kf: [] } } }] };
  const PM: any = { proj: { layers: [layer] }, animVersion: () => version };
  const gl = vi.fn(() => undefined); Object.defineProperty(PM, 'GL', { get: gl });
  const tracker = trackStaticExport(PM, false);
  try {
    expect(tracker.key(1)).toBeNull(); expect(tracker.hasStaticPlan).toBe(true);
    const queries = gl.mock.calls.length;
    for (let i = 0; i < 10; i++) expect(tracker.hasStaticPlan).toBe(true);
    expect(gl).toHaveBeenCalledTimes(queries);
    layer.p.opacity.kf = [{ t: 0, v: 100 }, { t: 1, v: 50 }] as any; version++;
    expect(tracker.key(1)).toBeNull(); expect(tracker.hasStaticPlan).toBe(false);
  } finally { tracker.dispose(); }
});


it('keeps source owner processing on the general export path',()=>{
 expect(staticExportPlan({layers:[{type:'shape',effectSpace:'source'}]},false)).toBeNull();
});
