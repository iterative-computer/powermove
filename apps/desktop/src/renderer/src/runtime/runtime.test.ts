// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';

import { doc } from '../state/document.svelte';
import { sel } from '../state/selection.svelte';
import { perf, transport } from '../state/transport.svelte';
import { frameBus } from './frame-bus';
import { installLegacyRuntime } from './install-legacy';
import { frame, invalidate, invalidateView, mutationSequence, onInvalidate } from './invalidate';
import { ports, resetPortsForTests } from './ports';

/** Minimal legacy PM: a bus, a project, selection, transport hooks. */
function fakePM() {
  const listeners = new Map<string, Set<(...a: any[]) => void>>();
  const bus = {
    on(ev: string, fn: (...a: any[]) => void) {
      let s = listeners.get(ev);
      if (!s) listeners.set(ev, (s = new Set()));
      s.add(fn);
      return () => s!.delete(fn);
    },
    emit(ev: string, ...a: any[]) {
      for (const fn of [...(listeners.get(ev) ?? [])]) fn(...a);
    }
  };
  const PM: Record<string, any> = {
    bus,
    proj: { id: 'p1', layers: [{ id: 'L1', p: { opacity: { kf: [{ i: 'k1' }] } } }], dur: 10 },
    sel: { layers: ['L1'], keys: ['k1'], chan: null },
    time: 2,
    playing: false,
    quality: 1,
    tool: 'select',
    perf: { ms: 4.2, fps: 60 },
    GL: { stats: { draws: 3, passes: 1, progs: 2 } },
    rasterStats: () => ({ size: 7 }),
    selectLayers(ids: string[]) {
      PM.sel.layers = ids;
    },
    setTime(t: number) {
      PM.time = t;
      bus.emit('time', t);
    }
  };
  return PM;
}

describe('document store', () => {
  it('bumps the right ticks per mutation kind and every tick on replace', () => {
    const before = { ...doc.tick };
    doc.bumpFor('structure');
    expect(doc.tick.structure).toBe(before.structure + 1);
    expect(doc.tick.values).toBe(before.values + 1);
    expect(doc.tick.project).toBe(before.project);
    const gen = doc.generation;
    doc.replace({ layers: [] } as any);
    expect(doc.generation).toBe(gen + 1);
    for (const k of Object.keys(before) as (keyof typeof before)[]) expect(doc.tick[k]).toBeGreaterThan(before[k]);
  });
});

describe('invalidate edge', () => {
  it('clears registered caches synchronously and sets frame flags', () => {
    const cleared = vi.fn();
    const off = onInvalidate(cleared);
    frame.render = false;
    frame.timeline = false;
    const seq = mutationSequence();
    invalidate('values');
    expect(cleared).toHaveBeenCalledTimes(1);
    expect(frame.render).toBe(true);
    expect(frame.timeline).toBe(true);
    expect(mutationSequence()).toBe(seq + 1);
    off();
    invalidate('values');
    expect(cleared).toHaveBeenCalledTimes(1);
  });

  it('view-only invalidation never bumps document ticks', () => {
    const before = { ...doc.tick };
    invalidateView('all');
    expect(doc.tick).toEqual(before);
  });
});

describe('legacy → runes bridge', () => {
  beforeEach(() => {
    resetPortsForTests(null);
    frameBus.clear();
  });

  it('mirrors project, selection and transport, and maps bus events to ticks', () => {
    const PM = fakePM();
    const off = installLegacyRuntime(PM);
    expect(doc.proj).toBe(PM.proj);
    expect(sel.layers).toEqual(['L1']);
    expect(sel.keys).toEqual(['k1']);
    expect(transport.time).toBe(2);

    const t = { ...doc.tick };
    PM.bus.emit('layers');
    expect(doc.tick.structure).toBe(t.structure + 1);
    PM.bus.emit('draw:ui');
    expect(doc.tick.values).toBe(t.values + 2); // structure implies values, plus draw:ui

    PM.sel.layers = ['L1', 'L2'];
    PM.bus.emit('sel');
    expect(sel.layers).toEqual(['L1', 'L2']);

    PM.setTime(5);
    expect(transport.time).toBe(5);

    const status = vi.fn();
    frameBus.on('status', status);
    PM.bus.emit('draw:status');
    expect(perf).toMatchObject({ ms: 4.2, fps: 60, draws: 3, passes: 1, progs: 2, raster: 7 });
    expect(status).toHaveBeenCalledTimes(1);

    // A project swap on the legacy side is reflected as a generation bump.
    const gen = doc.generation;
    PM.proj = { id: 'p2', layers: [] };
    PM.bus.emit('project');
    expect(doc.proj).toBe(PM.proj);
    expect(doc.generation).toBe(gen + 1);

    // ports delegate to the legacy app
    expect(ports().getProject()).toBe(PM.proj);
    ports().setTime(7);
    expect(PM.time).toBe(7);

    off();
    const after = { ...doc.tick };
    PM.bus.emit('layers');
    expect(doc.tick).toEqual(after);
    flushSync();
  });
});
