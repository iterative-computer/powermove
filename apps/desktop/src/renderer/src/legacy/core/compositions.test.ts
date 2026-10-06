import { describe, expect, it } from 'vitest';

import { makePM } from '../__tests__/make-pm';
import { canonicalizePatches, normalizeCompositions, realizePatches, rotateProject } from './compositions';

function composer() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  PM.proj = PM.mkProject({ name: 'Film', compName: 'Main', w: 1920, h: 1080, fps: 30, dur: 10 });
  return PM;
}

const names = (PM: any) => PM.proj.layers.map((layer: any) => layer.name);

describe('compositions', () => {
  it('lists every composition, the open one included, and opens by rotating it to the top', () => {
    const PM = composer();
    const mainId = PM.proj.compId;
    PM.proj.layers.push(PM.mkLayer('solid', { name: 'Back' }, PM.proj));
    const id = PM.Comps.create({ name: 'Titles', w: 1080, h: 1080, dur: 4 }, { open: false });

    expect(PM.Comps.list().map((comp: any) => [comp.name, comp.open])).toEqual([['Main', true], ['Titles', false]]);
    PM.time = 2;
    expect(PM.Comps.open(id)).toBe(true);
    expect(PM.proj).toMatchObject({ compId: id, compName: 'Titles', w: 1080, h: 1080, dur: 4, name: 'Film' });
    expect(PM.proj.layers).toEqual([]);
    expect(PM.proj.comps[mainId]).toMatchObject({ id: mainId, name: 'Main', w: 1920 });
    expect(PM.proj.comps[mainId].layers.map((layer: any) => layer.name)).toEqual(['Back']);
    expect(PM.proj.comps[id]).toBeUndefined();
    expect(PM.time).toBe(0);

    PM.Comps.open(mainId);
    expect(names(PM)).toEqual(['Back']);
    expect(PM.time).toBe(2); // each composition keeps its own current time
    expect(PM.Comps.openTabs()).toEqual([mainId, id]);
  });

  it('keeps undo attached to the composition an edit was made in', () => {
    const PM = composer();
    const mainId = PM.proj.compId;
    const id = PM.Comps.create({ name: 'Inner' }, { open: true });
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid', { name: 'Inside' }, PM.proj)));
    PM.Comps.open(mainId);
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid', { name: 'Outside' }, PM.proj)));

    PM.hist.undo();
    expect(names(PM)).toEqual([]);
    PM.hist.undo();
    expect(PM.proj.compId).toBe(mainId); // undo does not switch compositions
    expect(PM.proj.comps[id].layers).toEqual([]);
    PM.hist.redo();
    expect(PM.proj.comps[id].layers.map((layer: any) => layer.name)).toEqual(['Inside']);
    PM.Comps.open(id);
    PM.hist.redo();
    expect(PM.proj.comps[mainId].layers.map((layer: any) => layer.name)).toEqual(['Outside']);
    expect(names(PM)).toEqual(['Inside']);
  });

  it('steps out of a composition whose creation is undone while it is open', () => {
    const PM = composer();
    const mainId = PM.proj.compId;
    const id = PM.Comps.create({ name: 'Temp' });
    expect(PM.proj.compId).toBe(id);
    PM.hist.undo();
    expect(PM.proj.compId).toBe(mainId);
    expect(PM.Comps.exists(id)).toBe(false);
  });

  it('nests a composition as a centred precomp layer and refuses cycles', () => {
    const PM = composer();
    const mainId = PM.proj.compId;
    const inner = PM.Comps.create({ name: 'Logo', w: 400, h: 200, dur: 3 }, { open: false });
    PM.time = 1;
    const layer = PM.Comps.addToTimeline(inner);
    expect(layer).toMatchObject({ type: 'precomp', name: 'Logo', from: 1, dur: 3, d: { comp: inner, w: 400, h: 200 } });
    expect([layer.p['anchor.x'].v, layer.p['anchor.y'].v, layer.p['position.x'].v, layer.p['position.y'].v]).toEqual([200, 100, 960, 540]);

    expect(PM.Comps.addToTimeline(mainId)).toBeNull();
    PM.Comps.open(inner);
    expect(PM.Comps.canNest(mainId)).toBe(false); // Main already contains Logo
    expect(PM.Comps.addToTimeline(mainId)).toBeNull();
  });

  it('keeps unused compositions when their last layer is deleted', () => {
    const PM = composer();
    const inner = PM.Comps.create({ name: 'Kept' }, { open: false });
    const layer = PM.Comps.addToTimeline(inner);
    PM.removeLayers([layer.id]);
    expect(PM.Comps.exists(inner)).toBe(true);
  });

  it('pre-composes by moving layers into a new composition', () => {
    const PM = composer();
    const a = PM.mkLayer('shape', { name: 'A' }, PM.proj); a.from = 2; a.dur = 3;
    const b = PM.mkLayer('text', { name: 'B' }, PM.proj); b.from = 3; b.dur = 4;
    const keep = PM.mkLayer('solid', { name: 'Keep' }, PM.proj);
    b.parent = keep.id;
    PM.proj.layers.push(a, b, keep);

    const id = PM.Comps.precompose([a.id, b.id], { name: 'Cards', adjustDuration: true });
    expect(names(PM)).toEqual(['Cards', 'Keep']);
    const layer = PM.proj.layers[0];
    expect(layer).toMatchObject({ type: 'precomp', from: 2, dur: 5, d: { comp: id } });
    const comp = PM.proj.comps[id];
    expect(comp).toMatchObject({ name: 'Cards', w: 1920, h: 1080, dur: 5 });
    expect(comp.layers.map((item: any) => [item.name, item.from])).toEqual([['A', 0], ['B', 1]]);
    expect(b.parent).toBeNull(); // parent stayed behind in the outer comp

    PM.hist.undo();
    expect(names(PM)).toEqual(['A', 'B', 'Keep']);
    expect(PM.Comps.exists(id)).toBe(false);
  });

  it('pre-composes one layer leaving its attributes in place', () => {
    const PM = composer();
    const solid = PM.mkLayer('solid', { name: 'Plate', d: { w: 800, h: 600 } }, PM.proj);
    solid.p.rotation.v = 30;
    solid.fx = [{ id: 'fx', type: 'blur', on: true, p: {} }];
    PM.proj.layers.push(solid);

    const id = PM.Comps.precompose([solid.id], { name: 'Plate Comp', mode: 'leave' });
    expect(solid).toMatchObject({ type: 'precomp', name: 'Plate', d: { comp: id, w: 800, h: 600 } });
    expect(solid.p.rotation.v).toBe(30);
    expect(solid.fx).toHaveLength(1);
    const inner = PM.proj.comps[id];
    expect(inner).toMatchObject({ w: 800, h: 600 });
    expect(inner.layers[0]).toMatchObject({ type: 'solid', name: 'Plate', d: { w: 800, h: 600 }, fx: [] });
  });

  it('applies composition settings to precomp layers and renames their default names', () => {
    const PM = composer();
    const inner = PM.Comps.create({ name: 'Lower third', w: 600, h: 100 }, { open: false });
    const layer = PM.Comps.addToTimeline(inner);
    PM.Comps.setSettings(inner, { name: 'Name tag', w: 800, h: 120, dur: 2 });
    expect(PM.proj.comps[inner]).toMatchObject({ name: 'Name tag', w: 800, h: 120, dur: 2 });
    expect(layer).toMatchObject({ name: 'Name tag', d: { w: 800, h: 120 } });
  });

  it('duplicates with fresh layer ids and deletes along with the layers that use it', () => {
    const PM = composer();
    const inner = PM.Comps.create({ name: 'Comp 1' }, { open: true });
    const parent = PM.mkLayer('null', { name: 'Rig' }, PM.proj);
    const child = PM.mkLayer('solid', { name: 'Card' }, PM.proj);
    child.parent = parent.id;
    PM.proj.layers.push(child, parent);
    const copy = PM.Comps.duplicate(inner);
    const copied = PM.proj.comps[copy].layers;
    expect(PM.proj.comps[copy].name).toBe('Comp 2');
    expect(copied.map((layer: any) => layer.name)).toEqual(['Card', 'Rig']);
    expect(copied[0].id).not.toBe(child.id);
    expect(copied[0].parent).toBe(copied[1].id);

    PM.Comps.open(copy);
    const use = PM.Comps.addToTimeline(inner);
    expect(PM.Comps.remove(inner)).toBe(true);
    expect(PM.proj.layers.some((layer: any) => layer.id === use.id)).toBe(false);
    expect(PM.Comps.exists(inner)).toBe(false);
  });
});

describe('composition patch paths', () => {
  it('records open-comp paths independently of which comp is open', () => {
    const forward = [{ path: ['layers', 0, 'name'], exists: true, value: 'B' }, { path: ['assets', 'a'], exists: false }];
    const backward = [{ path: ['layers', 0, 'name'], exists: true, value: 'A' }, { path: ['assets', 'a'], exists: true, value: 1 }];
    const canonical = canonicalizePatches(forward, backward, 'C1');
    expect(canonical.forward.map(patch => patch.path)).toEqual([['comps', 'C1', 'layers', 0, 'name'], ['assets', 'a']]);
    expect(realizePatches(canonical.forward, { compId: 'C1' })[0]!.path).toEqual(['layers', 0, 'name']);
    expect(realizePatches(canonical.forward, { compId: 'C2' })[0]!.path).toEqual(['comps', 'C1', 'layers', 0, 'name']);
  });

  it('splits whole-map comps patches so a switch never drops a composition', () => {
    const canonical = canonicalizePatches(
      [{ path: ['comps'], exists: true, value: { A: { n: 1 }, B: { n: 2 } } }],
      [{ path: ['comps'], exists: true, value: { A: { n: 1 } } }], 'Root');
    expect(canonical.forward).toEqual([{ path: ['comps', 'B'], exists: true, value: { n: 2 } }]);
    expect(canonical.backward).toEqual([{ path: ['comps', 'B'], exists: false }]);
  });

  it('never carries one composition\'s output level into another', () => {
    const project: any = { id: 'P', compId: 'Root', compName: 'Root', w: 10, h: 10, layers: [], audioGain: 0.5, comps: { Intro: { id: 'Intro', name: 'Intro', w: 10, h: 10, layers: [] } } };
    const inside = rotateProject(project, 'Intro');
    expect(Object.hasOwn(inside, 'audioGain')).toBe(false);
    expect(inside.comps.Root.audioGain).toBe(0.5);
    const back = rotateProject(inside, 'Root');
    expect(back.audioGain).toBe(0.5);
    expect(Object.hasOwn(back.comps.Intro, 'audioGain')).toBe(false);

    // The reverse direction: a level set inside the child stays in the child.
    const child: any = { ...inside, audioGain: 2 };
    const root = rotateProject(child, 'Root');
    expect(root.audioGain).toBe(0.5);
    expect(root.comps.Intro.audioGain).toBe(2);

    // Replacing the open composition wholesale clears a level the replacement lacks.
    const realized = realizePatches([{ path: ['comps', 'Root'], exists: true, value: { id: 'Root', name: 'Root', layers: [] } }], back);
    expect(realized).toContainEqual({ path: ['audioGain'], exists: false });
    // A whole-project patch recorded inside the child is rotated without leaking.
    const whole = realizePatches([{ path: [], exists: true, value: inside }], back);
    expect(whole[0]!.value.audioGain).toBe(0.5);
    expect(Object.hasOwn(whole[0]!.value.comps.Intro, 'audioGain')).toBe(false);
  });

  it('rotates without mutating and flattens nested comps from older files', () => {
    const project: any = { id: 'P', compId: 'A', compName: 'A', w: 10, h: 10, layers: [1], comps: { B: { id: 'B', name: 'B', w: 20, h: 20, layers: [2] } } };
    const rotated = rotateProject(project, 'B');
    expect(project.compId).toBe('A');
    expect(rotated).toMatchObject({ compId: 'B', w: 20, layers: [2], comps: { A: { id: 'A', w: 10, layers: [1] } } });

    let n = 0;
    const legacy: any = { name: 'Old', layers: [], comps: { X: { name: 'X', layers: [], comps: { Y: { name: 'Y', layers: [] } } } } };
    normalizeCompositions(legacy, prefix => `${prefix}${++n}`);
    expect(legacy.compName).toBe('Old');
    expect(Object.keys(legacy.comps).sort()).toEqual(['X', 'Y']);
    expect(legacy.comps.X.comps).toEqual({});
  });
});
