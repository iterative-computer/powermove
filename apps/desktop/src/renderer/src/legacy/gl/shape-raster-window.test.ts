import { describe, expect, it } from 'vitest';
import { previewShapeRaster, rasterIntersectsViewport, shapeRasterGeometry } from './shape-raster-window';

const rectangle = { shape: 'rect', w: 2000, h: 1000, radius: 40, stroke: 4, color: '#102030' };
describe('visible primitive raster planning', () => {
  it('skips fully offscreen sources but retains an antialiasing margin', () => {
    expect(previewShapeRaster(rectangle, 2, [2, 0, 0, 2, 5000, 5000], 800, 600).kind).toBe('outside');
    expect(previewShapeRaster(rectangle, 2, [2, 0, 0, 2, -1999, 300], 800, 600).kind).not.toBe('outside');
  });
  it('uses a flat fill only when the entire view lies safely inside a rectangle', () => {
    expect(previewShapeRaster(rectangle, 2, [2, 0, 0, 2, 400, 300], 800, 600).kind).toBe('solid');
    expect(previewShapeRaster({ ...rectangle, shape: 'ellipse' }, 2, [2, 0, 0, 2, 400, 300], 800, 600).kind).toBe('crop');
    expect(previewShapeRaster({ ...rectangle, color: '#10203080' }, 2, [2, 0, 0, 2, 400, 300], 800, 600).kind).toBe('crop');
    expect(previewShapeRaster(rectangle, 2, [2, 0, 0, 2, 2000, 1000], 800, 600).kind).toBe('crop');
  });
  it('limits cropped sources and reuses their pixel window through small pans', () => {
    const shape = { ...rectangle, shape: 'ellipse' };
    const a = previewShapeRaster(shape, 4, [4, 0, 0, 4, 405, 305], 800, 600);
    const b = previewShapeRaster(shape, 4, [4, 0, 0, 4, 406, 306], 800, 600);
    expect(a).toEqual(b);
    expect(a.kind).toBe('crop');
    if (a.kind !== 'crop') throw Error('Expected a cropped raster');
    expect(a.window.width).toBeLessThan(1400);
    expect(a.window.height).toBeLessThan(1200);
  });
  it('handles reflected and rotated source spaces and keeps degenerate transforms on the original path', () => {
    expect(previewShapeRaster(rectangle, 2, [-2, 0, 0, 2, 400, 300], 800, 600).kind).toBe('solid');
    expect(previewShapeRaster(rectangle, 2, [0, 2, -2, 0, 400, 300], 800, 600).kind).toBe('solid');
    expect(previewShapeRaster(rectangle, 2, [0, 0, 0, 0, 400, 300], 800, 600).kind).toBe('full');
  });
  it('retains full texture rounding and maximum density for cropped raster alignment', () => {
    const geometry = shapeRasterGeometry({ w: 100.1, h: 50.2, stroke: 0 }, 1.25);
    expect(geometry).toMatchObject({ w: 108.1, h: 58.2, width: 136, height: 73, density: 1.25 });
    expect(shapeRasterGeometry(rectangle, 32).width).toBe(8192);
  });
});

describe('complete text source visibility', () => {
  const source = { w: 100, h: 50, anchorX: 90, anchorY: 10 };
  it('keeps asymmetrically anchored, reflected and rotated sources conservative', () => {
    expect(rasterIntersectsViewport(source, [16, 0, 0, 16, 0, 0], 320, 240)).toBe(true);
    expect(rasterIntersectsViewport(source, [-16, 0, 0, 16, 0, 0], 320, 240)).toBe(true);
    expect(rasterIntersectsViewport(source, [0, 16, -16, 0, 320, 240], 320, 240)).toBe(true);
    expect(rasterIntersectsViewport(source, [16, 0, 0, 16, 3000, 0], 320, 240)).toBe(false);
    expect(rasterIntersectsViewport(source, [0, 0, 0, 0, 3000, 0], 320, 240)).toBe(true);
  });
  it('keeps sampling support at the viewport boundary', () => {
    expect(rasterIntersectsViewport(source, [1, 0, 0, 1, -11, 0], 320, 240)).toBe(true);
    expect(rasterIntersectsViewport(source, [1, 0, 0, 1, -13, 0], 320, 240)).toBe(false);
  });
});

describe('opaque coverage scissors',()=>{
  it('retains edge pixels and partitions the uncovered area without overlaps', async()=>{
    const {uncoveredRasterRegions}=await import('./shape-raster-window');
    const regions=uncoveredRasterRegions(10,8,{x:2.2,y:1.3,width:5.6,height:4.6});
    const counts=Array.from({length:80},()=>0);
    for(const r of regions)for(let y=r.y;y<r.y+r.height;y++)for(let x=r.x;x<r.x+r.width;x++)counts[y*10+x]!++;
    for(let y=0;y<8;y++)for(let x=0;x<10;x++)expect(counts[y*10+x]).toBe(x>=3&&x<7&&y>=2&&y<5?0:1);
    expect(uncoveredRasterRegions(10,8,{x:-2,y:-2,width:20,height:20})).toEqual([]);
    expect(uncoveredRasterRegions(10,8,{x:20,y:20,width:2,height:2})).toEqual([{x:0,y:0,width:10,height:8}]);
  });
});
