import { afterEach, describe, expect, it, vi } from 'vitest';
import { formatFontVariationSettings, textVariationEntries } from './raster';
import * as catalog from '../../typography/font-catalog';
import { createVariableFontRenderer } from '../../typography/font-renderer';

afterEach(()=>{vi.restoreAllMocks();vi.unstubAllGlobals();});
describe('variable font rendering',()=>{
  it('formats authored axis values deterministically',()=>{
    const content={'fontAxis.wght':620,'fontAxis.GRAD':40,ignored:1};
    expect(textVariationEntries(content)).toEqual([['GRAD',40],['wght',620]]);
    expect(formatFontVariationSettings(content)).toBe('"GRAD" 40, "wght" 620');
  });
  it('loads the verified variable face and isolates simultaneous layer settings',async()=>{
    vi.spyOn(catalog,'inspectFont').mockResolvedValue({family:'Test',status:'variable',axes:[],source:{family:'Test',postscriptName:'TestVF-Regular',blob:async()=>new Blob()}});
    const constructed:any[]=[];
    class Face {status='loaded';constructor(public family:string,public source:unknown,public descriptors:any){constructed.push(this);}async load(){return this;}}
    vi.stubGlobal('FontFace',Face);vi.stubGlobal('document',{fonts:{add:vi.fn(),delete:vi.fn()}});
    const invalidate=vi.fn(),resolve=createVariableFontRenderer(invalidate);
    const a={font:'Test','fontAxis.wght':300},b={font:'Test','fontAxis.wght':700};
    expect(resolve(a)).toBeNull();expect(resolve(b)).toBeNull();
    await vi.waitFor(()=>expect(invalidate).toHaveBeenCalledTimes(1));
    expect(resolve(a)).not.toBe(resolve(b));
    expect(constructed.map(f=>f.descriptors.variationSettings)).toEqual(['"wght" 300','"wght" 700']);
    expect(constructed.every(f=>f.source instanceof ArrayBuffer)).toBe(true);
  });
  it('renders every new animation sample synchronously after one source load', async () => {
    const blob = vi.fn(async () => new Blob([new Uint8Array([1, 2, 3])]));
    vi.spyOn(catalog, 'inspectFont').mockResolvedValue({ family: 'Animated', status: 'variable', axes: [], source: { family: 'Animated', blob } });
    class Face { status = 'loaded'; constructor(public family: string, public source: unknown, public descriptors: any) {} async load() { return this; } }
    vi.stubGlobal('FontFace', Face);
    vi.stubGlobal('document', { fonts: { add: vi.fn(), delete: vi.fn() } });
    const invalidate = vi.fn(), resolve = createVariableFontRenderer(invalidate);
    const content = { font: 'Animated', 'fontAxis.wdth': 75 };
    resolve(content);
    await vi.waitFor(() => expect(invalidate).toHaveBeenCalled());
    for (let frame = 0; frame < 120; frame++) {
      expect(resolve({ ...content, 'fontAxis.wdth': 75 + frame / 2 })).not.toBeNull();
    }
    expect(blob).toHaveBeenCalledTimes(1);
    expect(invalidate).toHaveBeenCalledTimes(1);
  });

});
