/// <reference types="node" />

import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

import { TYPE_META, type Layer, type Project } from '../types/project';
import { sanitizeProject } from './project';

interface LegacyPM {
  proj?: unknown;
  mkProject(options?: Record<string, unknown>): Record<string, unknown>;
  mkLayer(type: string, options?: Record<string, unknown>, comp?: unknown): Record<string, unknown>;
  mkMask(shape?: string, comp?: unknown): Record<string, unknown>;
  KF(time: number, value: number, ease?: string): Record<string, unknown>;
}

// The js/ oracle is gone (Phase 6); build the fixture through the shipped TS
// module installs instead — same factories, same shapes.
import { makePM } from '../../legacy/__tests__/make-pm';

function legacyModel(): LegacyPM {
  let sequence = 0;
  const PM = makePM('core/easing', 'core/model') as unknown as LegacyPM;
  (PM as any).uid = (prefix: string) => `${prefix}-fixture-${++sequence}`;
  (PM as any).SHADER_TEMPLATE = 'uniform float uSpeed; // @param 1 0 4\nvoid main(){}';
  return PM;
}

function serializedLegacyDemo(): string {
  const PM = legacyModel();
  const project = PM.mkProject({
    name: 'Typed foundation fixture', w: 1280, h: 720, fps: 24, dur: 6, bg: '#0a0b0c',
    backgroundFill: {
      type: 'linear', angle: 24,
      stops: [
        { id: 'dark', color: '#0a0b0c', position: 0 },
        { id: 'light', color: '#ddefaa', position: 100 }
      ]
    }
  });
  PM.proj = project;

  const title = PM.mkLayer('text', { name: 'Title' }, project);
  title.id = 'title';
  const titleChannels = title.p as Record<string, { kf: unknown[] }>;
  titleChannels.opacity!.kf.push(PM.KF(0, 0, 'power'), PM.KF(1, 100, 'power'));
  title.masks = [PM.mkMask('ellipse', project)];
  title.fx = [{
    id: 'fx-glow', type: 'glow', on: true, open: false,
    p: { intensity: { v: 90, kf: [], expr: null } }
  }];

  const audio = PM.mkLayer('audio', {
    name: 'Soundtrack', d: { asset: 'audio-1', gain: 0.75, trim: 0.2, fadeIn: 0.1 }
  }, project);
  audio.id = 'soundtrack';

  const nested = PM.mkProject({ name: 'Nested', w: 1280, h: 720, fps: 24, dur: 6 });
  nested.id = 'comp-1';
  nested.layers = [PM.mkLayer('shape', { name: 'Nested shape' }, nested)];
  project.comps = { 'comp-1': nested };
  const precomp = PM.mkLayer('precomp', { name: 'Nested', d: { comp: 'comp-1' } }, project);
  precomp.id = 'nested-layer';

  project.layers = [title, audio, precomp];
  project.params = {
    Intensity: { name: 'Intensity', label: 'Glow intensity', control: 'num', min: 0, max: 100, value: 72 },
    Palette: {
      name: 'Palette', label: 'Palette', control: 'select',
      options: [{ v: 'warm', label: 'Warm' }, { v: 'cool', label: 'Cool' }], value: 'cool'
    }
  };
  project.markers = [{ t: 1.25, name: 'Reveal' }];

  return JSON.stringify({ v: '1.0.0', proj: project, ws: { id: 'ignored-workspace' } });
}

describe('sanitizeProject', () => {
  it('hydrates a VM-generated legacy model project and its nested composition', () => {
    const project = sanitizeProject(serializedLegacyDemo());

    expect(project.name).toBe('Typed foundation fixture');
    expect(project.w).toBe(1280);
    expect(project.h).toBe(720);
    expect(project.fps).toBe(24);
    expect(project.backgroundFill).toEqual({
      type: 'linear', angle: 24,
      stops: [
        { id: 'dark', color: '#0A0B0C', position: 0 },
        { id: 'light', color: '#DDEFAA', position: 100 }
      ]
    });
    expect(project.bg).toBe('#0A0B0C');
    expect(project.markers).toEqual([{ t: 1.25, name: 'Reveal' }]);

    const title = project.layers[0];
    expect(title?.type).toBe('text');
    if (!title || title.type !== 'text') throw new Error('expected text fixture');
    expect(title.d.text).toBe('Powermove');
    expect(title.p.opacity.kf.map(key => [key.t, key.v])).toEqual([[0, 0], [1, 100]]);
    expect(title.p.opacity.kf[0]).toMatchObject({ eo: [0.62, 0.05], ei: [0, 1] });
    expect(title.masks[0]?.shape).toBe('ellipse');
    expect(title.fx[0]).toMatchObject({ id: 'fx-glow', type: 'glow', on: true, open: false });
    expect(title.fx[0]?.p.intensity).toEqual({ v: 90, kf: [], expr: null });
    expect('solo' in title).toBe(false);

    const audio = project.layers[1];
    expect(audio?.type).toBe('audio');
    if (!audio || audio.type !== 'audio') throw new Error('expected audio fixture');
    expect(audio.d).toEqual({ asset: 'audio-1', gain: 0.75, trim: 0.2, fadeIn: 0.1, fadeOut: 0 });
    expect(audio.p).toEqual({});
    expect(audio.fx).toEqual([]);

    const nested = project.comps['comp-1'];
    expect(nested).toBeDefined();
    expect(nested?.id).toBe('comp-1');
    expect(nested?.layers[0]?.type).toBe('shape');
    const precomp = project.layers[2];
    expect(precomp?.type).toBe('precomp');
    if (!precomp || precomp.type !== 'precomp') throw new Error('expected precomp fixture');
    expect(precomp.d.comp).toBe('comp-1');
  });

  it('degrades corrupt and missing layer data to finite, typed source', () => {
    const project = sanitizeProject({
      name: 42,
      w: Number.NaN,
      h: 'bad',
      fps: Number.POSITIVE_INFINITY,
      dur: 0,
      bg: 'not-a-color',
      work: ['bad', Number.NaN],
      params: {
        Amount: { control: 'num', min: '10', max: '-2', value: '1000' },
        Enabled: { control: 'toggle', value: 1 },
        Broken: null
      },
      edits: [{
        id: 'edit-1', revision: '2', at: '3', origin: 'agent', label: 'Recovered',
        summary: ['kept', 42],
        operations: [{ type: 'add_marker', name: 'Recovered marker' }, { type: 'run_shell' }]
      }],
      comps: {
        broken: null,
        good: { w: '640', h: null, fps: '60', dur: '2', layers: [{ type: 'thing' }] }
      },
      layers: [
        {
          id: 'bad-layer', type: 'thing', name: null, from: -8, dur: 'bad', parent: 'missing',
          p: {
            opacity: {
              v: 'bad', expr: '   ',
              kf: [null, { t: '2', v: '40', hold: 1 }, { t: 2, v: 50 }, { t: 'bad', v: 1 }]
            },
            rogue: { v: 1, kf: [], expr: null }
          },
          fx: [{ type: 'made-up' }], masks: [{ shape: 'triangle', mode: 'x', p: {} }], d: null
        },
        {
          type: 'audio', d: { asset: 2, gain: 99, trim: -4, fadeIn: '1.5', fadeOut: 'bad' },
          p: { opacity: { v: 20 } }, fx: [{ type: 'glow' }], masks: [{}], parent: 'bad-layer',
          blend: 'screen', mblur: true
        },
        {}
      ]
    });

    expect(project).toMatchObject({ name: 'Untitled', w: 1920, h: 1080, fps: 30, dur: 10 });
    expect(project.work).toEqual([0, 10]);
    expect(Object.keys(project.comps)).toEqual(['good']);
    expect(project.comps.good).toMatchObject({ id: 'good', w: 640, h: 2, fps: 60, dur: 2 });
    expect(project.comps.good?.layers[0]?.type).toBe('null');
    expect(project.params.Amount).toMatchObject({
      name: 'Amount', label: 'Amount', control: 'num', min: -2, max: 10, value: 10
    });
    expect(project.params.Enabled).toMatchObject({ control: 'toggle', value: true });
    expect(project.params.Broken).toBeUndefined();
    expect(project.edits).toEqual([{
      id: 'edit-1', revision: 2, at: 3, origin: 'agent', label: 'Recovered', summary: ['kept'],
      operations: [
        { type: 'add_marker', name: 'Recovered marker' },
        { type: 'run_shell' }
      ]
    }]);

    const bad = project.layers[0];
    expect(bad?.type).toBe('null');
    expect(bad).toMatchObject({ name: 'Layer 1', from: 0, dur: 5, parent: null, fx: [] });
    expect(Object.keys(bad?.p ?? {})).toEqual(Object.keys((project.layers[2] as Exclude<Layer, { type: 'audio' }>).p));
    expect(bad && bad.type !== 'audio' ? bad.p.opacity.v : null).toBe(100);
    expect(bad && bad.type !== 'audio' ? bad.p.opacity.kf : []).toHaveLength(1);
    expect(bad && bad.type !== 'audio' ? bad.p.opacity.kf[0] : null)
      .toMatchObject({ eo: [0.62, 0.05], ei: [0, 1] });
    expect(bad?.masks[0]).toMatchObject({ shape: 'rect', mode: 'add', on: true });

    const audio = project.layers[1];
    expect(audio?.type).toBe('audio');
    if (!audio || audio.type !== 'audio') throw new Error('expected audio fixture');
    expect(audio.d).toEqual({ asset: null, gain: 4, trim: 0, fadeIn: 1.5, fadeOut: 0 });
    expect(audio).toMatchObject({ p: {}, fx: [], masks: [], parent: null, blend: 'normal', mblur: false });
    expect(project.layers[2]?.type).toBe('null');
    expect(project.layers.every(layer => layer.type in TYPE_META)).toBe(true);
  });

  it('turns an empty object into the mkProject/hydrate defaults', () => {
    const project: Project = sanitizeProject({});

    expect(project.id).toMatch(/^P-/);
    expect(project).toMatchObject({
      name: 'Untitled', w: 1920, h: 1080, fps: 30, dur: 10,
      bg: '#000000', revision: 0, shutter: 0.5
    });
    expect(project.backgroundFill).toEqual({
      type: 'solid', angle: 0,
      stops: [{ id: 'stop-1', color: '#000000', position: 0 }]
    });
    expect(project.layers).toEqual([]);
    expect(project.comps).toEqual({});
    expect(project.assets).toEqual({});
    expect(project.markers).toEqual([]);
    expect(project.work).toEqual([0, 10]);
    expect(project.params).toEqual({});
    expect(project.edits).toEqual([]);
    expect(Number.isFinite(project.created)).toBe(true);
  });

  it('preserves custom JSON content while repairing known per-type fields', () => {
    const content = JSON.parse(`{
      "text":"Hello",
      "customBrandField":"keep-me",
      "nested":{"safe":true,"prototype":"drop-me"},
      "constructor":"drop-me"
    }`) as Record<string, unknown>;
    content.runtimeOnly = () => 'not JSON';
    const project = sanitizeProject({ layers: [{ type: 'text', d: content }] });
    const layer = project.layers[0];
    if (layer?.type !== 'text') throw new Error('expected text layer');

    expect(layer.d).toMatchObject({
      text: 'Hello', customBrandField: 'keep-me', nested: { safe: true },
      font: 'SF Pro Display', size: 128
    });
    expect(Object.hasOwn(layer.d, 'constructor')).toBe(false);
    expect(Object.hasOwn(layer.d, 'runtimeOnly')).toBe(false);
  });

  it('sanitizes transition channels while preserving extension-defined types', () => {
    const transitionIn = JSON.parse(`{
      "type":"custom-reveal",
      "dur":"0.01",
      "missing":true,
      "p":{
        "amount":{"v":0.75,"kf":[{"t":"1","v":1,"eo":[0,0],"ei":[1,1],"hold":false,"i":"amount-1"}],"expr":"value"},
        "tint":{"v":"#aabbcc","kf":[],"expr":null},
        "badColor":{"v":"red","kf":[],"expr":null},
        "constructor":{"v":1,"kf":[],"expr":null}
      }
    }`) as Record<string, unknown>;
    const project = sanitizeProject({
      layers: [{ type: 'solid', transitionIn, transitionOut: { type: '', dur: 2, p: {} } }]
    });
    const layer = project.layers[0];

    expect(layer?.transitionIn).toMatchObject({
      type: 'custom-reveal', dur: 0.02, missing: true,
      p: {
        amount: { v: 0.75, expr: 'value' },
        tint: { v: '#aabbcc', kf: [], expr: null },
        badColor: { v: 0, kf: [], expr: null }
      }
    });
    expect(layer?.transitionIn?.p.amount?.kf[0]).toMatchObject({
      t: 1, v: 1, eo: [0, 0], ei: [1, 1], hold: false, i: 'amount-1'
    });
    expect(Object.hasOwn(layer?.transitionIn?.p ?? {}, 'constructor')).toBe(false);
    expect(layer?.transitionOut).toBeNull();
  });

  it('keeps the last 200 provenance entries and leaves operations opaque', () => {
    const operations = [
      {
        type: 'update_section', sectionId: 'section-1',
        layers: [{ intentionally: 'opaque rather than a complete Layer' }]
      },
      {
        type: 'set_property', target: 'L-1', path: 'position.x', value: 1040,
        preserveHandEdits: false, markIntent: 'human', overrideLock: true
      }
    ];
    const edits = Array.from({ length: 205 }, (_, index) => ({
      id: `edit-${index}`, revision: String(index), at: String(index),
      origin: index === 204 ? 'agent' : 'interface', label: `Edit ${index}`,
      summary: ['first operation', 'second operation'], operations
    }));
    const project = sanitizeProject({ edits });

    expect(project.edits).toHaveLength(200);
    expect(project.edits[0]?.id).toBe('edit-5');
    expect(project.edits.at(-1)).toEqual({
      id: 'edit-204', revision: 204, at: 204, origin: 'agent', label: 'Edit 204',
      summary: ['first operation', 'second operation'], operations
    });
  });

  it('drops unsafe project dictionary keys without changing object prototypes', () => {
    const project = sanitizeProject(JSON.parse(`{
      "params":{"__proto__":{"polluted":true}},
      "comps":{"constructor":{"layers":[]}},
      "layers":[{"type":"shader","d":{"code":"uniform float constructor; // @param 1","uniforms":{}}}]
    }`));

    expect(Object.hasOwn(project.params, '__proto__')).toBe(false);
    expect(Object.hasOwn(project.comps, 'constructor')).toBe(false);
    const shader = project.layers[0];
    expect(shader?.type).toBe('shader');
    if (shader?.type === 'shader') expect(Object.hasOwn(shader.d.uniforms, 'constructor')).toBe(false);
    expect((Object.prototype as { polluted?: boolean }).polluted).toBeUndefined();
  });

  it('preserves structured extension data when its renderer is not installed', () => {
    const project = sanitizeProject({
      w: 1920, h: 1080, dur: 5,
      layers: [{
        id: 'custom', type: 'extension', name: 'Custom', from: 0, dur: 5,
        d: {
          definition: 'demo.layer', version: 3, w: 800, h: 600,
          params: { amount: { v: 0.5, kf: [{ t: 1, v: 1, i: 'key-1' }], expr: null } },
          data: { objects: [{ id: 'cube' }] }
        }
      }]
    });
    const layer = project.layers[0];
    expect(layer?.type).toBe('extension');
    if (layer?.type !== 'extension') return;
    expect(layer.d.definition).toBe('demo.layer');
    expect(layer.d.version).toBe(3);
    expect(layer.d.data).toEqual({ objects: [{ id: 'cube' }] });
    expect(layer.d.params.amount?.kf[0]?.i).toBe('key-1');
  });
});
