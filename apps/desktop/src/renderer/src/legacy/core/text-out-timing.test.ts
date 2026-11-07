import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { pasteLayers, splitLayers } from '../ui/shortcuts';
import { migrateProjectTextTiming, staggerTimingControl, staggerEndOffset, staggerWindow, textAnimationState, unitWeights } from './text-animation';
import { packProjectFile, unpackProjectFile } from './project-file';

afterEach(() => vi.unstubAllGlobals());
function editor() {
  vi.stubGlobal('window', { addEventListener() {}, removeEventListener() {} });
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'ui/shortcuts');
  PM.proj = PM.mkProject({ w: 640, h: 360, fps: 30, dur: 12 });
  const animator = (direction: string) => ({ id: direction, name: direction, mode: 'stagger', unit: 'characters', direction, easing: 'linear',
    p: Object.fromEntries(Object.entries({ delay: 0, duration: 1, stagger: .1, opacity: 0 }).map(([k, v]) => [k, PM.P(v)])) });
  const layer = PM.mkLayer('text', { from: 2, dur: 6, d: { text: 'EXIT', animators: [animator('in'), animator('out')] } });
  PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch(); PM.hist.clear();
  return { PM, layer, exit: layer.d.animators[1], entrance: layer.d.animators[0] };
}
const values = (animator: any) => Object.fromEntries(Object.entries<any>(animator.p).map(([k, p]) => [k, p.v]));
const weights = (layer: any, animator: any, time: number) => unitWeights({}, layer, animator, values(animator), 4, time);

describe('end-anchored text timing', () => {
  it('finishes at Out before and after shortening, extending, rippling/moving, slipping and stretching', () => {
    const { layer, exit, entrance } = editor();
    for (const [from, dur] of [[2, 6], [2, 3], [2, 8], [5, 8], [4, 9], [-2, .5]] as const) {
      Object.assign(layer, { from, dur });
      expect(weights(layer, exit, from + dur)).toEqual([1, 1, 1, 1]);
      expect(weights(layer, exit, from + dur - .5)).toEqual([.8, .7, .6, .5].map(v => expect.closeTo(v, 8)));
      expect(weights(layer, entrance, from)).toEqual([1, 1, 1, 1]);
      expect(staggerWindow(values(exit), 4, exit.order, layer, exit).end).toBeCloseTo(dur);
    }
  });

  it('handles early, late, zero-duration, explicit anchors, all orders and split glyph sequences', () => {
    const { layer, exit } = editor();
    for (const offset of [-.5, 0, .5]) for (const order of ['forward', 'reverse', 'center', 'edges', 'random']) {
      Object.assign(exit, { order, unitOffset: 1, unitTotal: 9 }); exit.p.delay.v = offset;
      expect(weights(layer, exit, layer.from + layer.dur - offset)).toEqual([1, 1, 1, 1]);
    }
    exit.p.delay.v = 0; exit.p.duration.v = 0; exit.p.stagger.v = 0;
    expect(weights(layer, exit, 8)).toEqual([1, 1, 1, 1]);
    expect(weights(layer, exit, 7.9)).toEqual([0, 0, 0, 0]);
    exit.anchor = 'in'; exit.p.delay.v = 1;
    expect(weights(layer, exit, 3)).toEqual([1, 1, 1, 1]);
  });

  it('keeps a right trim as one Undo/Redo step without rewriting animator channels', () => {
    const { PM, layer, exit } = editor(); const original = JSON.stringify(exit);
    PM.Edit.begin('Trim clip', { origin: 'timeline' });
    for (const duration of [5, 4, 3]) PM.Edit.dispatch({ type: 'set_layer', target: layer.id, patch: { duration } });
    PM.Edit.commit('Trim clip');
    expect(PM.hist.list()).toHaveLength(1);
    expect(JSON.stringify(PM.proj.layers[0].d.animators[1])).toBe(original);
    expect(weights(PM.proj.layers[0], exit, 5)).toEqual([1, 1, 1, 1]);
    PM.hist.undo(); expect(PM.proj.layers[0].dur).toBe(6);
    PM.hist.redo(); expect(PM.proj.layers[0].dur).toBe(3);
  });

  it('preserves v1 pixels algebraically at every sample, including animated timing, Unicode and wrapping; save/reopen is stable', async () => {
    const { PM, layer, exit } = editor(); delete exit.timingVersion;
    exit.p.delay.v = 2; exit.p.delay.kf = [PM.KF(0, 2), PM.KF(6, 3)];
    exit.p.duration.kf = [PM.KF(0, .5), PM.KF(6, 1.5)];
    exit.p.stagger.expr = '.05 + t * .01';
    const old = JSON.parse(JSON.stringify(layer)); old.d.animators[1].anchor = 'in';
    const project = PM.deserialize(PM.serialize());
    const restored = project.layers[0], migrated = restored.d.animators[1];
    expect(migrated).toMatchObject({ timingVersion: 2, legacyStartDuration: 6 });
    const layout = (count: number) => ({ characters: Array.from({ length: count }), words: [], lines: [] });
    for (let t = 2; t <= 8; t += .125) for (const count of [1, 4, 9]) {
      const state = (l: any) => textAnimationState(PM, l, t, layout(count)).animators[1]!.weights;
      expect(state(restored)).toEqual(state(old).map(w => expect.closeTo(w, 8)));
    }
    const archive = await packProjectFile({ proj: project }, { get: async () => null, put: async () => true });
    const reopened = PM.deserialize(unpackProjectFile(archive));
    expect(reopened.layers[0].d.animators[1]).toEqual(migrated);
    migrateProjectTextTiming(reopened); expect(reopened.layers[0].d.animators[1]).toEqual(migrated);
    const v = values(migrated), offset = staggerEndOffset(migrated, v, 4);
    restored.dur = 3;
    expect(staggerWindow(v, 4, undefined, restored, migrated).end).toBeCloseTo(3 - offset);
  });

  it('migrates nested compositions and reusable library versions independently', () => {
    const { layer, exit } = editor(); delete exit.timingVersion;
    const clone = (dur: number) => ({ ...JSON.parse(JSON.stringify(layer)), dur });
    const project: any = { layers: [clone(6)], comps: { c: { layers: [clone(3)] } }, library: { sections: [{ layers: [clone(9)], versions: [{ layers: [clone(4)] }] }] } };
    migrateProjectTextTiming(project);
    const basis = (l: any) => l.d.animators[1].p.delay.v;
    expect([basis(project.layers[0]), basis(project.comps.c.layers[0]), basis(project.library.sections[0].layers[0]), basis(project.library.sections[0].versions[0].layers[0])]).toEqual([4.7, 1.7, 7.7, 2.7].map(v => expect.closeTo(v, 8)));
  });

  it('converts a static legacy start into a real offset that stays fixed when duration changes', () => {
    const { PM, layer, exit } = editor(); delete exit.timingVersion; exit.p.delay.v = 4.7;
    const restored = PM.deserialize(PM.serialize()).layers[0], a = restored.d.animators[1];
    expect(a.legacyStartDuration).toBeUndefined(); expect(a.p.delay.v).toBeCloseTo(0);
    expect(staggerWindow(values(a), 4, undefined, restored, a).start).toBeCloseTo(4.7);
    a.p.duration.v = 2;
    expect(staggerWindow(values(a), 4, undefined, restored, a).end).toBeCloseTo(6);
    expect(PM.deserialize(JSON.stringify({ layers: [restored] })).layers[0].d.animators[1]).toEqual(a);
    expect(layer.d.animators[1].p.delay.v).toBe(4.7);
  });

  it('keeps In on the left and Out on the right when splitting, with one Undo', () => {
    const { PM, layer } = editor(); PM.selectLayers(layer.id); PM.time = 5;
    splitLayers(PM);
    const head = PM.proj.layers.find((l: any) => l.from === 2), tail = PM.proj.layers.find((l: any) => l.from === 5);
    expect(head.d.animators.map((a: any) => a.direction)).toEqual(['in']);
    expect(tail.d.animators.map((a: any) => a.direction)).toEqual(['out']);
    expect(weights(tail, tail.d.animators[0], 8)).toEqual([1, 1, 1, 1]);
    PM.hist.undo(); expect(PM.proj.layers).toHaveLength(1); expect(PM.proj.layers[0].d.animators).toHaveLength(2);
    PM.hist.redo(); expect(PM.proj.layers).toHaveLength(2);
  });

  it('retains anchor timing through copy/paste, duplicate and precompose', () => {
    const { PM, layer, exit } = editor(); exit.p.delay.v = .5;
    PM.time = 4;
    pasteLayers(PM, () => [layer]); const pasted = PM.proj.layers.find((l: any) => l.id !== layer.id);
    const ids = new Set(PM.proj.layers.map((l: any) => l.id));
    pasteLayers(PM, () => [layer], { atPlayhead: false }); const duplicate = PM.proj.layers.find((l: any) => !ids.has(l.id));
    for (const copy of [pasted, duplicate]) {
      expect(copy.d.animators[1]).toMatchObject({ timingVersion: 2 });
      expect(weights(copy, copy.d.animators[1], copy.from + copy.dur - .5)).toEqual([1, 1, 1, 1]);
    }
    const id = PM.Comps.precompose([layer.id], { adjustDuration: true });
    const nested = PM.proj.comps[id].layers.find((l: any) => l.type === 'text');
    expect(weights(nested, nested.d.animators[1], nested.from + nested.dur - .5)).toEqual([1, 1, 1, 1]);
  });

  it('stamps agent add/set-content commands so undo/reopen does not mistake them for legacy timing', () => {
    const { PM, exit } = editor(); const raw = JSON.parse(JSON.stringify(exit)); delete raw.timingVersion;
    const added = PM.Edit.apply({ type: 'add_layer', layerType: 'text', from: 0, duration: 4, content: { animators: [raw] } });
    expect(added.ok).toBe(true);
    const l = PM.proj.layers[0]; expect(l.d.animators[0].timingVersion).toBe(2);
    expect(PM.Edit.apply({ type: 'set_content', target: l.id, patch: { animators: [raw] } }).ok).toBe(true);
    const reopened = PM.deserialize(PM.serialize()).layers[0];
    expect(reopened.d.animators[0].legacyStartDuration).toBeUndefined();
    expect(weights(reopened, reopened.d.animators[0], 4)).toEqual([1, 1, 1, 1]);
  });
});

describe('inspector stagger timing adapter', () => {
  it('shows migrated exits as End offset and edits the original animated channel with undo', () => {
    const {PM,layer,exit}=editor();exit.legacyStartDuration=6;
    exit.p.delay.kf=[PM.KF(0,1),PM.KF(2,2)];exit.p.delay.expr='value';
    PM.time=layer.from+2;PM.touch();
    const prop=exit.p.delay;
    const evaluated=()=>Object.fromEntries(Object.entries(exit.p).map(([k,p])=>[k,PM.evP(layer,p,PM.time,`ta.${exit.id}.${k}`)]));
    const control=staggerTimingControl(exit,evaluated(),4),before=JSON.parse(JSON.stringify(prop));
    expect(control.label).toBe('End offset');
    expect(control.value).toBeCloseTo(staggerEndOffset(exit,evaluated(),4));
    const value=control.toChannelValue(.5),path=`ta.${exit.id}.delay`;
    expect(PM.Edit.apply({type:'set_property',target:layer.id,path,value,time:PM.time}).ok).toBe(true);
    expect(exit.p.delay).toBe(prop);expect(prop.expr).toBe('value');
    expect(prop.kf.map((k:any)=>k.i)).toEqual(before.kf.map((k:any)=>k.i));
    expect(prop.kf[0]).toEqual(before.kf[0]);expect(prop.kf[1].v).toBeCloseTo(value);
    expect(staggerTimingControl(exit,evaluated(),4).value).toBeCloseTo(.5);
    PM.hist.undo();expect(PM.L(layer.id).d.animators[1].p.delay).toEqual(before);
    PM.hist.redo();expect(PM.L(layer.id).d.animators[1].p.delay.expr).toBe('value');
  });
});
