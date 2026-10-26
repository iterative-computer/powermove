import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './editing';
import { makePM } from '../__tests__/make-pm';

describe('legacy editing install', () => {
  it('can create a layer before zero through the same editing commands', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ fps: 30, dur: 10 });
    expect(PM.Edit.apply({ type: 'add_layer', id: 'early', layerType: 'solid', from: -3, duration: 5 }).ok).toBe(true);
    expect(PM.L('early')).toMatchObject({ from: -3, dur: 5 });
  });

  it('moves a layer before zero without changing its duration or local keys, with undo and redo', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ fps: 30, dur: 10 });
    const layer = PM.mkLayer('solid', { from: 1, dur: 3 });
    PM.addLayer(layer, 0);
    PM.setKey(layer, 'opacity', 2, 50);
    PM.hist.clear();

    expect(PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { from: -7 } }).ok).toBe(true);
    expect(PM.L(layer.id)).toMatchObject({ from: -7, dur: 3 });
    expect(PM.L(layer.id).p.opacity.kf[0].t).toBe(1);
    PM.hist.undo();
    expect(PM.L(layer.id).from).toBe(1);
    PM.hist.redo();
    expect(PM.L(layer.id).from).toBe(-7);
  });

  it('edits composition settings without touching layer or keyframe state, with exact undo and redo', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ fps: 30, dur: 10 });
    const layer = PM.mkLayer('solid', { from: 0, dur: 10 });
    PM.addLayer(layer, 0);
    PM.setKey(layer, 'opacity', 2, 50);
    PM.hist.clear();
    const events: string[] = [];
    for (const name of ['layers', 'project']) PM.bus.on(name, () => events.push(name));
    const evaluation = PM.evaluationVersion(), version = PM.animVersion();
    expect(PM.Edit.apply({ type: 'set_composition', patch: { workArea: [1, 4] } }).ok).toBe(true);
    expect(PM.proj.work).toEqual([1, 4]);
    // An edit like any other (autosave and saves see it), but no layer
    // announcement and no re-preparation of every keyframe track.
    expect(PM.animVersion()).toBeGreaterThan(version);
    expect(PM.evaluationVersion()).toBe(evaluation);
    expect(events).toEqual(['project']);
    // The history entry holds the work area and the edit's provenance only.
    const entry = PM.hist.export({ copy: false }).entries.at(-1);
    expect(entry.forward.map((patch: any) => patch.path.join('.')).sort()).toEqual([`comps.${PM.proj.compId}.work`, 'edits', 'revision']);
    PM.hist.undo();
    expect(PM.proj.work).toEqual([0, 10]);
    PM.hist.redo();
    expect(PM.proj.work).toEqual([1, 4]);
  });

  it('re-evaluates and redraws after a composition setting when an expression reads the composition', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ fps: 30, dur: 10 });
    const layer = PM.mkLayer('solid', { from: 0, dur: 10 });
    PM.addLayer(layer, 0);
    layer.p['position.x'].expr = 'comp.work[0] * 100';
    PM.touch();
    PM.hist.clear();
    PM.beginEval(2);
    expect(PM.worldMatrix(layer, 2)[4]).toBe(0);
    const invalidated: unknown[] = [];
    PM.invalidate = (what = 'all') => { invalidated.push(what); };
    const evaluation = PM.evaluationVersion();
    expect(PM.Edit.apply({ type: 'set_composition', patch: { workArea: [1, 4] } }).ok).toBe(true);
    // Paused on the same frame: the expression's new answer, drawn again.
    expect(PM.evaluationVersion()).toBeGreaterThan(evaluation);
    PM.beginEval(2);
    expect(PM.worldMatrix(layer, 2)[4]).toBe(100);
    expect(invalidated).toContain('all');
  });

  it('records the exact frame rate a composition patch writes, so undo restores both forms', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ fps: 30, dur: 10 });
    const before = { fps: PM.proj.fps, fpsRational: PM.proj.fpsRational };
    PM.hist.clear();
    expect(PM.Edit.apply({ type: 'set_composition', patch: { fps: 23.976 } }).ok).toBe(true);
    const after = { fps: PM.proj.fps, fpsRational: PM.proj.fpsRational };
    expect(after).not.toEqual(before);
    PM.hist.undo();
    expect({ fps: PM.proj.fps, fpsRational: PM.proj.fpsRational }).toEqual(before);
    PM.hist.redo();
    expect({ fps: PM.proj.fps, fpsRational: PM.proj.fpsRational }).toEqual(after);
  });

  it('installs the frozen source-edit vocabulary', () => {
    const PM: PMRegistry = {};
    install(PM);

    expect(PM.Edit.operations.set_property).toEqual({
      target: 'layer', fields: ['path', 'value', 'time', 'mode', 'ease'],
    });
    expect(PM.Edit.operations.set_content).toEqual({ target: 'layer', fields: ['patch'] });
    expect(PM.Edit.operations.transform_layers).toEqual({
      target: 'layer-collection', fields: ['transform', 'state'],
    });
    expect(PM.Edit.operations.set_transition).toEqual({
      target: 'layer', fields: ['layer', 'edge', 'transition'],
    });
    expect(Object.keys(PM.Edit.operations)).toHaveLength(26);
  });
});
