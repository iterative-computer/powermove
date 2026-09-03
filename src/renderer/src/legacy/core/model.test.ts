import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './model';

function projectModel(): PMRegistry {
  let nextId = 0;
  const PM: PMRegistry = {
    version: 'test',
    clamp: (v: number, a: number, b: number) => Math.max(a, Math.min(b, v)),
    uid: (p: string) => `${p}${++nextId}`,
    Ease: { handles: () => ({ eo: [.33, 0], ei: [.67, 1] }) },
    SHADER_TEMPLATE: 'void main(){}',
    bus: { emit() {} },
    invalidate() {},
    selectLayers() {},
  };
  install(PM);
  return PM;
}

describe('legacy model install', () => {
  it('creates a full-comp adjustment layer with editable transform source', () => {
    const PM = projectModel();
    const project = PM.mkProject({ name: 'T', w: 1920, h: 1080, fps: 30, dur: 10 });
    PM.proj = project;

    const layer = PM.mkLayer('adjustment', { name: 'Global Grade' }, project);

    expect(layer).toMatchObject({
      type: 'adjustment', name: 'Global Grade', d: {}, fx: [], masks: [],
      blend: 'normal', mblur: false, parent: null,
    });
    expect(layer.p['position.x'].v).toBe(0);
    expect(layer.p['position.y'].v).toBe(0);
    expect(layer.p.opacity.v).toBe(100);
    expect(PM.TYPE_META.adjustment).toMatchObject({ label: 'Adjustment', pickable: false });
  });

  it('precomposes layers with the original span and stack order', () => {
    const PM = projectModel();
    const project = PM.mkProject({ name: 'T', w: 1920, h: 1080, fps: 30, dur: 10 });
    PM.proj = project;
    const a = PM.mkLayer('shape', { name: 'A' }, project); a.from = 1; a.dur = 3;
    const b = PM.mkLayer('text', { name: 'B' }, project); b.from = 2; b.dur = 3;
    const keep = PM.mkLayer('solid', { name: 'Keep' }, project);
    project.layers.push(a, b, keep);

    const layer = PM.precompose([a.id, b.id], 'Group');
    const sub = project.comps[layer.d.comp];

    expect(layer.type).toBe('precomp');
    expect(sub).toBeTruthy();
    expect(sub.layers.map((item: any) => item.name)).toEqual(['A', 'B']);
    expect(project.layers.map((item: any) => item.name)).toEqual(['Group', 'Keep']);
    expect(layer.from).toBe(1);
    expect(Math.abs(layer.dur - 4) < 1e-9).toBe(true);
    expect(sub.dur).toBe(4);
    expect(sub.work).toEqual([0, 4]);
    expect(sub.layers.map((item: any) => item.from)).toEqual([0, 1]);
    expect(PM.compOf(layer)).toBe(sub);
  });

  it('refreshes keyframe ids in every cloned animation channel', () => {
    const PM = projectModel();
    const project = PM.mkProject({ name: 'T' });
    PM.proj = project;
    const layer: any = PM.mkLayer('shader', { name: 'Animated' }, project);
    const keyed = (id: string) => ({ v: 0, kf: [{ i: id, t: 0, v: 0 }], expr: null });
    layer.p.opacity = keyed('transform');
    layer.fx = [{ id: 'fx', type: 'blur', p: { amount: keyed('effect') } }];
    layer.masks = [{ id: 'mask', p: { x: keyed('mask') } }];
    layer.d.uniforms = { amount: keyed('uniform') };
    layer.transitionIn = { type: 'wipe', p: { angle: keyed('transition') } };

    const clone = PM.cloneLayer(layer);
    const ids = [
      clone.p.opacity.kf[0].i, clone.fx[0].p.amount.kf[0].i,
      clone.masks[0].p.x.kf[0].i, clone.d.uniforms.amount.kf[0].i,
      clone.transitionIn.p.angle.kf[0].i,
    ];
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).not.toContain('transform');
    expect(ids).not.toContain('effect');
    expect(ids).not.toContain('mask');
    expect(ids).not.toContain('uniform');
    expect(ids).not.toContain('transition');
  });

  it('garbage-collects a composition with its last precomp layer', () => {
    const PM = projectModel();
    const project = PM.mkProject({ name: 'T' });
    PM.proj = project;
    const source = PM.mkLayer('shape', { name: 'A' }, project);
    project.layers.push(source);
    const layer = PM.precompose([source.id], 'Group');
    const compId = layer.d.comp;

    PM.removeLayers([layer.id]);

    expect(project.comps[compId]).toBeUndefined();
  });
});
