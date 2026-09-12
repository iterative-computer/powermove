import { describe, it, expect } from 'vitest';
import { planeMatrix, projectPoint, planeContains, depthOrderedLayers, CHANNELS_3D } from './space-3d';
const layer = (id = 'a', p: any = {}) => ({ id, threeD: true, p: { ...CHANNELS_3D, 'position.x': 320, 'position.y': 180, 'scale.x': 100, 'scale.y': 100, ...p } });
const pm = (layers: any[] = []) => ({ proj: { w: 640, h: 360 }, L: (id: string) => layers.find(l => l.id === id), ev: (l: any, k: string) => l.p[k] ?? 0, worldMatrix: () => [1, 0, 0, 1, 320, 180], localMatrix: () => [1, 0, 0, 1, 320, 180] });
describe('2.5D planes', () => {
    it('adjusts focal length without an upper cap and keeps picking aligned', () => {
        const PM = pm(), l = layer('a', { 'position.z': 640 * 50 / 36 });
        for (const [strength, expectedX] of [[50,370],[25,320+100/3],[50000,320+100/1.001]]) {
            l.p.perspective = strength;
            const p = projectPoint(planeMatrix(PM,l,0), {x:100,y:0});
            expect(p.x).toBeCloseTo(expectedX!);
            expect(planeContains(PM,l,0,p.x,p.y,{x0:0,y0:-10,x1:110,y1:10})).toBe(true);
        }
    });
    it('enabling a neutral 3D layer preserves its 2D pose', () => { const PM = pm(), l = layer(); expect(planeMatrix(PM, l, 0)).toEqual(planeMatrix(PM, { ...l, threeD: false }, 0)); });
    it('positive depth makes a plane smaller and negative depth makes it larger', () => { const PM = pm(); expect(projectPoint(planeMatrix(PM, layer('a', { 'position.z': 640 * 50 / 36 }), 0), { x: 100, y: 0 })).toEqual({ x: 370, y: 180 }); });
    it('rotates about the anchor and uses perspective-correct inverse picking', () => { const PM = pm(), l = layer('a', { 'rotation.y': 55, 'rotation.x': 20 }); const p = projectPoint(planeMatrix(PM, l, 0), { x: 45, y: 35 }); expect(planeContains(PM, l, 0, p.x, p.y, { x0: -50, x1: 50, y0: -40, y1: 40 })).toBe(true); expect(planeContains(PM, l, 0, 0, 0, { x0: -50, x1: 50, y0: -40, y1: 40 })).toBe(false); });
    it('does not pick behind the camera or edge-on planes', () => { expect(planeContains(pm(), layer('a', { 'position.z': -2000 }), 0, 320, 180, { x0: -50, x1: 50, y0: -50, y1: 50 })).toBe(false); expect(planeContains(pm(), layer('a', { 'rotation.y': 90 }), 0, 320, 180, { x0: -50, x1: 50, y0: -50, y1: 50 })).toBe(false); });
    it('inherits parent depth and rotations', () => { const parent = layer('p', { 'rotation.y': 90 }), child = { ...layer('c', { 'position.x': 100, 'position.y': 0 }), parent: 'p' }, PM = pm([parent, child]); const p = projectPoint(planeMatrix(PM, child, 0), { x: 0, y: 0 }); expect(p.x).toBeCloseTo(320); expect(p.y).toBeCloseTo(180); });
    it('sorts each depth group without crossing 2D compositing barriers or changing input', () => { const a = layer('a', { 'position.z': 100 }), b = layer('b', { 'position.z': -100 }), barrier = { ...layer('2d'), threeD: false }, c = layer('c', { 'position.z': -200 }); const layers = [a, b, barrier, c]; expect(depthOrderedLayers(pm(), layers, 0).map(l => l.id)).toEqual(['b', 'a', '2d', 'c']); expect(layers[0]).toBe(a); });
});
