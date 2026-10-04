import { describe, expect, it } from 'vitest';

import { compCarriesAudio, deriveStrips, stripsKey, type StripContext } from './strips';

const P = (v: number, kf: Array<{ t: number; v: number }> = []) => ({ v, kf });

function context(project: any, time = 0): StripContext {
  return {
    time,
    comps: project.comps,
    // Linear interpolation is enough to prove the strip reads the playhead value.
    evaluate: (layer: any, value: any, at: number) => {
      if (!value || typeof value !== 'object' || !Array.isArray(value.kf)) return value;
      if (!value.kf.length) return value.v;
      const local = at - (layer.from || 0);
      const [a, b] = value.kf;
      if (!b || local <= a.t) return a.v;
      if (local >= b.t) return b.v;
      return a.v + (b.v - a.v) * (local - a.t) / (b.t - a.t);
    },
    hasKeyAt: (layer: any, prop: any, at: number) => prop.kf.find((key: any) => Math.abs(key.t - (at - layer.from)) < 1e-6) ?? null,
    groupAncestors: (layer: any, layers: any[]) => {
      const out: any[] = [];
      let id = layer.group;
      while (id) { const group = layers.find((item) => item.id === id); if (!group) break; out.push(group); id = group.group; }
      return out;
    }
  };
}

function base(): any {
  return {
    layers: [
      { id: 'title', name: 'Title', type: 'text', on: true, from: 0, d: {} },
      { id: 'music', name: 'Music', type: 'audio', on: true, from: 0, d: { asset: 'a', gain: 0.5 } },
      { id: 'empty', name: 'Placeholder', type: 'audio', on: true, from: 0, d: { asset: null, gain: 1 } },
      { id: 'clip', name: 'Interview', type: 'video', on: true, from: 0, d: { asset: 'v', embeddedAudio: true } },
      { id: 'silent', name: 'B-roll', type: 'video', on: true, from: 0, d: { asset: 'v2', embeddedAudio: false } },
      { id: 'scene', name: 'Scene', type: 'precomp', on: true, from: 0, d: { comp: 'inner', audioGain: 2, audioMuted: true } },
      { id: 'graphics', name: 'Graphics', type: 'precomp', on: true, from: 0, d: { comp: 'pictures' } }
    ],
    comps: {
      inner: { id: 'inner', layers: [{ id: 'deep', type: 'precomp', d: { comp: 'deeper' } }] },
      deeper: { id: 'deeper', layers: [{ id: 'vo', type: 'audio', d: { asset: 'vo' } }] },
      pictures: { id: 'pictures', layers: [{ id: 'logo', type: 'image', d: { asset: 'img' } }] }
    }
  };
}

describe('mixer strips', () => {
  it('derives one strip per audible top-level layer in timeline order', () => {
    const project = base();
    const strips = deriveStrips(project, context(project));
    expect(strips.map((strip) => [strip.id, strip.kind, strip.gainKey])).toEqual([
      ['music', 'audio', 'gain'],
      ['clip', 'video', 'audioGain'],
      ['scene', 'precomp', 'audioGain']
    ]);
    expect(strips.map((strip) => strip.gain)).toEqual([0.5, 1, 2]);
    expect(strips.map((strip) => strip.muted)).toEqual([false, false, true]);
    expect(stripsKey(strips)).toBe('music\u0000clip\u0000scene');
  });

  it('finds sound at any depth and survives a composition cycle', () => {
    const project = base();
    expect(compCarriesAudio(project.comps.inner, project.comps)).toBe(true);
    expect(compCarriesAudio(project.comps.pictures, project.comps)).toBe(false);
    project.comps.loop = { id: 'loop', layers: [{ id: 'self', type: 'precomp', d: { comp: 'loop' } }] };
    expect(compCarriesAudio(project.comps.loop, project.comps)).toBe(false);
  });

  it('reads an animated level at the playhead and reports a key there', () => {
    const project = base();
    project.layers[1].from = 1;
    project.layers[1].d.gain = P(1, [{ t: 0, v: 1 }, { t: 2, v: 0 }]);
    const between = deriveStrips(project, context(project, 2))[0]!;
    expect(between).toMatchObject({ animated: true, keyAtTime: false });
    expect(between.gain).toBeCloseTo(0.5);
    const atKey = deriveStrips(project, context(project, 3))[0]!;
    expect(atKey).toMatchObject({ gain: 0, keyAtTime: true });
  });

  it('treats a switched-off audio layer as muted and an animated switch as not toggleable', () => {
    const project = base();
    project.layers[1].on = false;
    expect(deriveStrips(project, context(project))[0]).toMatchObject({ muted: true, muteAnimated: false, silencedBy: null });
    project.layers[1].on = P(1, [{ t: 0, v: 1 }]);
    expect(deriveStrips(project, context(project))[0]).toMatchObject({ muteAnimated: true });
  });

  it('explains silence from timeline solo, a hidden clip, or a switched-off group', () => {
    const project = base();
    project.layers[3].solo = true;
    expect(deriveStrips(project, context(project)).map((strip) => strip.silencedBy)).toEqual(['solo', null, 'solo']);
    project.layers[3].solo = false;
    project.layers[3].on = false;
    project.layers.push({ id: 'group', type: 'group', on: false, d: {} });
    project.layers[1].group = 'group';
    expect(deriveStrips(project, context(project)).map((strip) => strip.silencedBy)).toEqual(['off', 'off', null]);
  });

  it('marks locked strips, including through a locked group', () => {
    const project = base();
    project.layers.push({ id: 'group', type: 'group', on: true, lock: true, d: {} });
    project.layers[1].group = 'group';
    project.layers[3].lock = true;
    expect(deriveStrips(project, context(project)).map((strip) => strip.locked)).toEqual([true, true, false]);
  });

  it('is empty when nothing is audible', () => {
    const project = { layers: [{ id: 'title', type: 'text', d: {} }], comps: {} };
    expect(deriveStrips(project, context(project))).toEqual([]);
  });
});
