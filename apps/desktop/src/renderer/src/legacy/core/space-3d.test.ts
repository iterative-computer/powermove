import { describe, it, expect } from 'vitest';
import { planeMatrix, projectPoint, planeContains, depthOrderedLayers, CHANNELS_3D, affinePlane, local3D } from './space-3d';
const layer = (id = 'a', p: any = {}) => ({ id, threeD: true, p: { ...CHANNELS_3D, 'position.x': 320, 'position.y': 180, 'scale.x': 100, 'scale.y': 100, ...p } });
const pm = (layers: any[] = []) => ({ proj: { w: 640, h: 360 }, L: (id: string) => layers.find(l => l.id === id), ev: (l: any, k: string) => l.p[k] ?? 0, worldMatrix: () => [1, 0, 0, 1, 320, 180], localMatrix: () => [1, 0, 0, 1, 320, 180] });
describe('2.5D planes', () => {
    it('preserves full matrix composition for every rotation axis, skew, anchors and reflections', () => {
        const identity = () => [1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1];
        const multiply = (a: number[], b: number[]) => Array.from({ length: 16 }, (_, index) => {
            const row = index % 4, column = index - row;
            return a[row]! * b[column]! + a[row + 4]! * b[column + 1]!
                + a[row + 8]! * b[column + 2]! + a[row + 12]! * b[column + 3]!;
        });
        const translate = (x: number, y: number, z: number) => {
            const matrix = identity(); matrix[12] = x; matrix[13] = y; matrix[14] = z; return matrix;
        };
        const reference = (properties: Record<string, number>) => {
            let matrix = translate(properties['position.x']!, properties['position.y']!, properties['position.z']!);
            for (const [axis, key] of [[2,'rotation'],[1,'rotation.y'],[0,'rotation.x'],[2,'orientation.z'],[1,'orientation.y'],[0,'orientation.x']] as const) {
                const rotation = identity(), a = (axis + 1) % 3, b = (axis + 2) % 3;
                const angle = properties[key]! * Math.PI / 180;
                rotation[a * 4 + a] = rotation[b * 4 + b] = Math.cos(angle);
                rotation[a * 4 + b] = Math.sin(angle); rotation[b * 4 + a] = -Math.sin(angle);
                if (angle !== 0) matrix = multiply(matrix, rotation);
            }
            const scale = identity();
            scale[0] = properties['scale.x']! / 100; scale[5] = properties['scale.y']! / 100; scale[10] = properties['scale.z']! / 100;
            scale[4] = Math.tan(properties.skew! * Math.PI / 180) * scale[5]!;
            return multiply(multiply(matrix, scale), translate(-properties['anchor.x']!, -properties['anchor.y']!, -properties['anchor.z']!));
        };
        for (let sample = 0; sample < 128; sample++) {
            const properties = {
                'position.x': sample * 11 - 500, 'position.y': sample * 3 - 250, 'position.z': sample - 80,
                'anchor.x': sample % 7 - 3, 'anchor.y': 30 - sample, 'anchor.z': sample % 11,
                'scale.x': sample % 5 ? sample * 7 - 400 : 0, 'scale.y': sample % 9 ? 250 - sample * 4 : 0, 'scale.z': sample * 3 - 200,
                rotation: sample % 4 ? sample * 17 - 270 : 0, 'rotation.y': sample % 5 ? sample * 3 - 60 : 0,
                'rotation.x': sample % 6 ? 80 - sample * 5 : 0, 'orientation.z': sample % 7 ? sample * 11 - 90 : 0,
                'orientation.y': sample % 8 ? sample * 7 - 120 : 0, 'orientation.x': sample % 9 ? 65 - sample * 9 : 0,
                skew: sample % 161 - 80,
            };
            const expected = reference(properties), actual = local3D(pm(), layer('a', properties), sample / 30);
            actual.forEach((value, index) => expect(value).toBeCloseTo(expected[index]!, 10));
            // WebGL receives float32 matrix values. Preserve the exact numeric
            // input at that boundary in addition to the double-precision check.
            expect(Array.from(Float32Array.from(actual), value => value || 0))
                .toEqual(Array.from(Float32Array.from(expected), value => value || 0));
        }
    });
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


describe('affine 3D source bounds', () => {
    it('matches perspective projection for flat planes at different depths', () => {
        for (const depth of [-200, 0, 400]) {
            const m = planeMatrix(pm(), layer('a', { 'position.z': depth, rotation: 32 }), 0);
            const a = affinePlane(m)!;
            expect(a).not.toBeNull();
            for (const p of [{x:0,y:0}, {x:-400,y:180}, {x:250,y:-70}]) {
                const projected = projectPoint(m, p);
                expect(a[0]*p.x+a[2]*p.y+a[4]).toBeCloseTo(projected.x);
                expect(a[1]*p.x+a[3]*p.y+a[5]).toBeCloseTo(projected.y);
            }
        }
    });
    it('keeps tilted, behind-camera and invalid projections on full sources', () => {
        expect(affinePlane(planeMatrix(pm(),layer('a',{'rotation.y':0.001}),0))).toBeNull();
        expect(affinePlane(planeMatrix(pm(),layer('a',{'position.z':-2000}),0))).toBeNull();
        expect(affinePlane([1,0,0,0,1,0,NaN,0,1])).toBeNull();
    });
});
