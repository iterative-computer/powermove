// @ts-nocheck -- drives the legacy PM registry like the contracts suites.
import { beforeEach, describe, expect, it } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { exportCaptionsText } from './commands';
import { parseSrt } from './formats';

let PM: any;
const captions = () => PM.proj.layers.find((layer: any) => layer.type === 'captions');
const texts = () => captions().d.cues.map((cue: any) => cue.text);

beforeEach(() => {
  PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'core/capabilities', 'gl/shaders');
  PM.proj = PM.mkProject({ name: 'Captions', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 0;
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
  PM.hist.reset?.();
});

const SRT = '1\n00:00:01,000 --> 00:00:02,000\nHello there\n\n2\n00:00:03,000 --> 00:00:05,000\nGeneral Kenobi\n';

describe('caption commands', () => {
  it('adds a captions layer on top from SRT text, sized to the composition', () => {
    PM.proj.layers.push(PM.mkLayer('solid'));
    const result = PM.Edit.apply({ type: 'add_captions', text: SRT, style: { preset: 'boxed' } }, { label: 'Import captions', origin: 'agent' });
    expect(result.ok).toBe(true);
    const layer = PM.proj.layers[0];
    expect(layer.type).toBe('captions');
    expect(layer.from).toBe(0);
    expect(layer.dur).toBe(10);
    expect(layer.d.style.preset).toBe('boxed');
    expect(layer.d.style.box).toBe(true);
    expect(texts()).toEqual(['Hello there', 'General Kenobi']);
    expect(PM.proj.edits.at(-1).operations[0].type).toBe('add_captions');
  });

  it('uses composition time in commands and layer time in storage', () => {
    PM.Edit.apply({ type: 'add_captions', from: 2, cues: [{ start: 3, end: 4, text: 'At three' }] });
    const layer = captions();
    expect(layer.d.cues[0]).toMatchObject({ start: 1, end: 2 });
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'split', id: layer.d.cues[0].id, at: 3.5 });
    expect(layer.d.cues.map((cue: any) => [cue.start, cue.end])).toEqual([[1, 1.5], [1.5, 2]]);
    expect(parseSrt(exportCaptionsText(layer, 'srt')).cues.map(cue => [cue.start, cue.end])).toEqual([[3, 3.5], [3.5, 4]]);
  });

  it('edits cues as one undo step each and undoes them', () => {
    PM.Edit.apply({ type: 'add_captions', text: SRT });
    const layer = captions();
    const [a, b] = layer.d.cues;
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'update', cues: [{ id: a.id, text: 'Hi' }] }, { label: 'Edit caption' });
    expect(texts()).toEqual(['Hi', 'General Kenobi']);
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'merge', ids: [a.id, b.id] }, { label: 'Merge captions' });
    expect(texts()).toEqual(['Hi General Kenobi']);
    PM.hist.undo();
    expect(texts()).toEqual(['Hi', 'General Kenobi']);
    PM.hist.undo();
    expect(texts()).toEqual(['Hello there', 'General Kenobi']);
    PM.hist.redo();
    expect(texts()).toEqual(['Hi', 'General Kenobi']);
  });

  it('moves, deletes, inserts, restyles and re-imports', () => {
    PM.Edit.apply({ type: 'add_captions', text: SRT });
    const layer = captions();
    const [a, b] = layer.d.cues;
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'move', ids: [a.id], by: 5 });
    expect(layer.d.cues[0]).toMatchObject({ start: 2, end: 3 });
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'delete', ids: [b.id] });
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'insert', cues: [{ start: 12, end: 13, text: 'Late' }] });
    expect(texts()).toEqual(['Hello there', 'Late']);
    expect(layer.dur).toBe(13);
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'style', style: { preset: 'pop' } });
    expect(layer.d.style).toMatchObject({ preset: 'pop', textCase: 'upper', wordAnimation: 'pop' });
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'style', style: { fill: '#00FF00' } });
    expect(layer.d.style.preset).toBe('custom');
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'import', text: 'WEBVTT\n\n00:01.000 --> 00:02.000\nReplaced' });
    expect(texts()).toEqual(['Replaced']);
  });

  it('fails cleanly and leaves the project untouched', () => {
    PM.Edit.apply({ type: 'add_captions', text: SRT });
    const layer = captions();
    const before = JSON.stringify(PM.proj.layers);
    for (const command of [
      { type: 'edit_captions', target: layer.id, op: 'split', id: 'missing', at: 1.5 },
      { type: 'edit_captions', target: layer.id, op: 'update', cues: [{ id: 'missing', text: 'x' }] },
      { type: 'edit_captions', target: layer.id, op: 'import', text: 'not captions' },
      { type: 'add_captions', text: 'nothing here' }
    ]) {
      expect(PM.Edit.apply(command).ok).toBe(false);
    }
    const solid = PM.mkLayer('solid'); PM.addLayer(solid);
    expect(PM.Edit.apply({ type: 'edit_captions', target: solid.id, op: 'delete', ids: [] }).message).toMatch(/not a captions layer/);
    expect(JSON.stringify(PM.proj.layers.filter((item: any) => item.id !== solid.id))).toBe(before);
  });

  it('respects layer locks for generated origins', () => {
    PM.Edit.apply({ type: 'add_captions', text: SRT });
    const layer = captions();
    layer.lock = true;
    const result = PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'delete', ids: [layer.d.cues[0].id], overrideLock: true }, { origin: 'agent' });
    expect(result.ok).toBe(false);
    expect(result.message).toMatch(/locked/);
  });

  it('merges a live cue drag into one recorded command', () => {
    PM.Edit.apply({ type: 'add_captions', text: SRT });
    const layer = captions();
    const id = layer.d.cues[0].id;
    PM.Edit.begin('Move caption', { origin: 'timeline' });
    for (const start of [1.1, 1.2, 1.3]) PM.Edit.dispatch({ type: 'edit_captions', target: layer.id, op: 'update', cues: [{ id, start, end: start + 1 }] });
    PM.Edit.commit();
    const entry = PM.proj.edits.at(-1);
    expect(entry.operations).toHaveLength(1);
    expect(layer.d.cues[0]).toMatchObject({ start: 1.3, end: 2.3 });
    PM.hist.undo();
    expect(captions().d.cues[0]).toMatchObject({ start: 1, end: 2 });
  });
});
