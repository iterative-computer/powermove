import { afterEach, describe, expect, it, vi } from 'vitest';
import { inspectFont, readFontAxes } from './font-catalog';
export function axisFont(tag = 'wght', min = 100, def = 400, max = 900): ArrayBuffer {
  const bytes = new Uint8Array(68), v = new DataView(bytes.buffer);
  v.setUint32(0, 0x10000); v.setUint16(4, 1);
  bytes.set([... 'fvar'].map(c => c.charCodeAt(0)), 12); v.setUint32(20, 32); v.setUint32(24, 36);
  v.setUint16(32, 1); v.setUint16(36, 16); v.setUint16(40, 1); v.setUint16(42, 20);
  bytes.set([...tag].map(c => c.charCodeAt(0)), 48);
  v.setInt32(52, min * 65536); v.setInt32(56, def * 65536); v.setInt32(60, max * 65536);
  return bytes.buffer;
}
afterEach(() => vi.unstubAllGlobals());
describe('font catalog', () => {
  it('reads actual bounds and custom axes and rejects damaged data', () => {
    expect(readFontAxes(axisFont())).toEqual([{ tag:'wght',label:'Weight',min:100,max:900,default:400 }]);
    expect(readFontAxes(axisFont('XTRA', -10, 0, 200))[0]).toMatchObject({tag:'XTRA',min:-10,max:200,default:0});
    expect(readFontAxes(new ArrayBuffer(3))).toEqual([]);
    expect(readFontAxes(axisFont('wght', 500, 400, 900))).toEqual([]);
  });
  it('reads a variable face after a static face in a font collection', () => {
    const bytes = new Uint8Array(120), v = new DataView(bytes.buffer);
    bytes.set([... 'ttcf'].map(c=>c.charCodeAt(0)));v.setUint32(8,2);v.setUint32(12,20);v.setUint32(16,40);
    v.setUint32(20,0x10000);bytes.set(new Uint8Array(axisFont()),40);v.setUint32(60,72);
    expect(readFontAxes(bytes.buffer)[0]?.tag).toBe('wght');
  });
  it('skips broken faces, matches font aliases and selects a real variable source', async () => {
    vi.stubGlobal('queryLocalFonts', vi.fn(async()=>[
      {family:'Mixed Test',style:'Regular',blob:async()=>{throw Error('unreadable');}},
      {family:'Mixed Test',fullName:'Mixed Test Variable',postscriptName:'MixedTestVF',blob:async()=>new Blob([axisFont('wdth',75,100,125)])}
    ]));
    expect(await inspectFont('Mixed Test')).toMatchObject({status:'variable',axes:[{tag:'wdth',min:75,max:125}]});
    expect(await inspectFont('MixedTestVF')).toMatchObject({status:'variable'});
  });
  it('does not permanently cache failed access and distinguishes static fonts', async () => {
    const query=vi.fn().mockRejectedValueOnce(Error('not ready')).mockResolvedValue([{family:'Retry Test',blob:async()=>new Blob([axisFont()])}]);
    vi.stubGlobal('queryLocalFonts',query);
    expect((await inspectFont('Retry Test')).status).toBe('unavailable');
    expect((await inspectFont('Retry Test')).status).toBe('variable');
    query.mockResolvedValue([{family:'Static Test',blob:async()=>new Blob([new ArrayBuffer(12)])}]);
    expect((await inspectFont('Static Test')).status).toBe('static');
  });
});
