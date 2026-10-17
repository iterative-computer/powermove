import { describe, expect, it } from 'vitest';

import { applyEngineStamp, nativeOnlyFeatures, REQUIRES_NATIVE_MESSAGE, refusesProject, requiresNativeEngine } from './engine-stamp';

describe('requiresEngine stamp', () => {
  it('stamps projects that link media or override interpretation, and unstamps them after', () => {
    const project: any = { assets: { a: { id: 'a' } } };
    expect(nativeOnlyFeatures(project)).toEqual([]);
    expect(applyEngineStamp(project)).toBe(false);
    expect('requiresEngine' in project).toBe(false);
    project.assets.b = { id: 'b', storage: 'linked', link: { path: '/x', hash: 'a'.repeat(64), size: 1 } };
    expect(nativeOnlyFeatures(project)).toEqual(['linkedMedia']);
    expect(applyEngineStamp(project)).toBe(true);
    expect(project.requiresEngine).toBe('native');
    expect(applyEngineStamp(project)).toBe(false);
    project.assets.b.interpret = { alpha: 'premultiplied' };
    expect(nativeOnlyFeatures(project)).toEqual(['linkedMedia', 'interpretation']);
    delete project.assets.b;
    expect(applyEngineStamp(project)).toBe(true);
    expect('requiresEngine' in project).toBe(false);
  });

  it('stamps non-default project color settings', () => {
    const project: any = { assets: {} };
    project.color = { mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe' };
    expect(nativeOnlyFeatures(project)).toEqual([]);
    project.color.linearBlending = true;
    expect(nativeOnlyFeatures(project)).toEqual(['projectColor']);
    expect(applyEngineStamp(project)).toBe(true);
    expect(project.requiresEngine).toBe('native');
    delete project.color;
    expect(applyEngineStamp(project)).toBe(true);
    expect('requiresEngine' in project).toBe(false);
  });

  it('stamps a clip whose pitch follows its speed, in any composition', () => {
    const project: any = { assets: {}, layers: [{ id: 'v', type: 'video', d: { pitch: 'preserve' } }], comps: { c2: { layers: [{ id: 'a', type: 'audio', d: {} }] } } };
    expect(nativeOnlyFeatures(project)).toEqual([]);
    project.comps.c2.layers[0].d.pitch = 'follow';
    expect(nativeOnlyFeatures(project)).toEqual(['clipPitch']);
    expect(applyEngineStamp(project)).toBe(true);
    project.comps.c2.layers[0].d.pitch = 'preserve';
    expect(applyEngineStamp(project)).toBe(true);
    expect('requiresEngine' in project).toBe(false);
  });

  it('leaves a stamp it does not know alone', () => {
    const project: any = { assets: {}, requiresEngine: 'future' };
    expect(applyEngineStamp(project)).toBe(false);
    expect(project.requiresEngine).toBe('future');
  });

  it('refuses stamped documents only in the legacy engine', () => {
    expect(REQUIRES_NATIVE_MESSAGE).toBe('This project uses the new engine. Turn on Use New Engine to open it.');
    for (const document of [{ proj: { requiresEngine: 'native' } }, { requiresEngine: 'native', proj: {} }, { requiresEngine: 'native', layers: [] }, { proj: { requiresEngine: 'other' } }]) {
      expect(requiresNativeEngine(document)).toBe(true);
      expect(refusesProject(document, false)).toBe(true);
      expect(refusesProject(document, true)).toBe(false);
    }
    expect(refusesProject({ proj: { layers: [] } }, false)).toBe(false);
    expect(refusesProject(null, false)).toBe(false);
  });
});
