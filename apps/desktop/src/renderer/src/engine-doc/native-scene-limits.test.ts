import { describe, expect, it } from 'vitest';
import { ZodError } from 'zod';
import { parseScene } from '../core/scene3d/schema';
import { compositionScene, LAYER3D_DEFINITIONS } from '../core/scene3d/layers';
import { editScene } from '../core/scene3d/operations';
import { makePM } from '../legacy/__tests__/make-pm';
import { approvedSceneLimit, NativeApi } from './native';

function refusal(scene: unknown): unknown {
  try { parseScene(scene); } catch (error) { return error; }
  throw new Error('Expected parser refusal');
}

describe('the native legacy-scene parser bridge', () => {
  it('waives only the editor-reachable caps', () => {
    expect(approvedSceneLimit(refusal({ objects: Array.from({ length: 513 }, (_, i) => ({ id: `o${i}`, source: { primitive: 'box' } })) }))).toBe(true);
    expect(approvedSceneLimit(refusal({ lights: Array.from({ length: 17 }, (_, i) => ({ id: `l${i}` })) }))).toBe(true);
    expect(approvedSceneLimit(refusal({ objects: [{ id: 'long', name: 'x'.repeat(161), source: { primitive: 'box' } }] }))).toBe(true);
    expect(approvedSceneLimit(refusal({ objects: [{ id: 'a', source: { primitive: 'box' }, name: 'x'.repeat(8_000_001) }] }))).toBe(true);
  });

  it('keeps original parser refusals when a cap and foreign corruption coexist', () => {
    const scene = { objects: Array.from({ length: 513 }, (_, i) => ({ id: `o${i}`, source: { primitive: i === 7 ? 'knot' : 'box' } })) };
    const error = refusal(scene);
    expect(approvedSceneLimit(error)).toBe(false);
    expect(approvedSceneLimit(refusal({ objects: [{ id: 'a', source: { primitive: 'box', foreign: true } }] }))).toBe(false);
  });

  it('returns the exact composition parser ZodError only after a native refusal', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ name: '3D', fps: 30, dur: 5 }); PM.time = 0;
    PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
      ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
    expect(editScene(PM, { operation: 'add_object', object: { id: 'a', source: { primitive: 'box' } } }).ok).toBe(true);
    PM.L('a').d.data.object.source = { primitive: 'knot' };
    let exact: unknown;
    try { compositionScene(PM, 0); } catch (error) { exact = error; }
    expect(exact).toBeInstanceOf(ZodError);

    const wasm = { dead: false, query: () => ({ throw: { kind: 'ZodError', message: 'native schema refusal' } }) };
    const barrier = { revision: 1, flush() {} };
    const native = new NativeApi(PM, () => wasm as any, () => barrier as any, x => x, {}, null);
    const address = PM.proj.compId ?? PM.proj.id;
    const reply = (native as any).query('pick', { comp: address, t: 0 });
    expect(reply.threw).toBeInstanceOf(ZodError);
    expect(reply.threw.message).toBe((exact as Error).message);
    expect(reply.threw.issues).toEqual((exact as ZodError).issues);

    // The compatibility parser must stay off the successful native path.
    PM.active = () => { throw new Error('compositionScene was evaluated'); };
    const okWasm = { dead: false, query: () => ({ ok: 42 }) };
    const successful = new NativeApi(PM, () => okWasm as any, () => barrier as any, x => x, {}, null);
    expect((successful as any).query('pick', { comp: address, t: 0 })).toEqual({ value: 42 });
  });

  it('keeps an oversized corrupt scene a ZodError instead of a generic Error', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ name: '3D', fps: 30, dur: 5 }); PM.time = 0;
    PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
      ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
    expect(editScene(PM, { operation: 'add_object', object: { id: 'a', source: { primitive: 'box' } } }).ok).toBe(true);
    PM.L('a').name = 'x'.repeat(8_000_001);
    PM.L('a').d.data.object.source = { primitive: 'knot' };
    const wasm = { dead: false, query: () => ({ throw: { kind: 'ZodError', message: 'Invalid 3D scene: source' } }) };
    const barrier = { revision: 1, flush() {} };
    const native = new NativeApi(PM, () => wasm as any, () => barrier as any, x => x, {}, null);
    const reply = (native as any).query('pick', { comp: PM.proj.compId ?? PM.proj.id, t: 0 });
    expect(reply.threw).toBeInstanceOf(ZodError);
    expect(reply.threw.name).toBe('ZodError');
    expect(reply.threw.message).toBe('Invalid 3D scene: source');
  });
});
