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
    expect(PM.compOf(layer)).toBe(sub);
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
