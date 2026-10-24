import { describe, expect, it } from 'vitest';
import { createObject, parseScene } from './schema';

describe('reusable scene validation schemas', () => {
  it('gives every object and parse independent transform defaults', () => {
    const source = { objects: Array.from({ length: 500 }, (_, i) => ({ id: `object${i}`, source: { primitive: 'box' as const } })) };
    const first = parseScene(source), second = parseScene(source);
    expect(first.objects).toHaveLength(500);
    for (const [index, object] of first.objects.entries()) {
      expect(object.p).not.toBe(second.objects[index]!.p);
      expect(object.p.x).toEqual({ v: 0, kf: [], expr: null });
      expect(object.p.sx).toEqual({ v: 1, kf: [], expr: null });
      expect(object.p.visible).toEqual({ v: true, kf: [], expr: null });
      expect(object.p.x).not.toBe(object.p.y);
      expect(object.p.x!.kf).not.toBe(object.p.y!.kf);
      if (index) {
        expect(object.p).not.toBe(first.objects[0]!.p);
        expect(object.p.x!.kf).not.toBe(first.objects[0]!.p.x!.kf);
      }
    }
    first.objects[0]!.p.x!.v = 77;
    first.objects[0]!.p.x!.kf.push({ t: 1, v: 99 });
    expect(first.objects[1]!.p.x).toEqual({ v: 0, kf: [], expr: null });
    expect(second.objects[0]!.p.x).toEqual({ v: 0, kf: [], expr: null });
    expect(createObject('later').p.x).toEqual({ v: 0, kf: [], expr: null });
  });

  it('normalizes mixed transform channels without mutating or sharing the input', () => {
    const shared = { v: -2, kf: [{ t: 0, v: -2, easing: [0, 0, 1, 1] }], custom: 'retained' };
    const source = { objects: [
      { id: 'a', source: { primitive: 'box' as const }, p: { sx: shared, x: 20, visible: false } },
      { id: 'b', source: { primitive: 'sphere' as const }, p: { sx: shared } }
    ] };
    const original = JSON.stringify(source), scene = parseScene(source);
    expect(scene.objects[0]!.p.sx).toEqual({ ...shared, expr: null });
    expect(scene.objects[0]!.p.x).toEqual({ v: 20, kf: [], expr: null });
    expect(scene.objects[0]!.p.visible).toEqual({ v: false, kf: [], expr: null });
    expect(scene.objects[0]!.p.sx).not.toBe(shared);
    expect(scene.objects[0]!.p.sx).not.toBe(scene.objects[1]!.p.sx);
    expect(scene.objects[0]!.p.sx!.kf).not.toBe(scene.objects[1]!.p.sx!.kf);
    scene.objects[0]!.p.sx!.kf[0].v = 4;
    expect(scene.objects[1]!.p.sx!.kf[0].v).toBe(-2);
    expect(JSON.stringify(source)).toBe(original);
  });

  it.each([
    { x: Infinity }, { x: 100001 }, { rx: 360001 }, { sx: -10001 },
    { visible: 'true' }, { typo: 2 }, { x: { v: 0, kf: [{ t: -1, v: 2 }] } },
    { x: { v: 0, kf: [{ t: 1, v: 'bad' }] } }
  ])('continues to reject invalid transforms %#', p => {
    expect(() => parseScene({ objects: [{ id: 'invalid', source: { primitive: 'box' }, p }] })).toThrow();
    expect(createObject('valid').p.x).toEqual({ v: 0, kf: [], expr: null });
  });
});
