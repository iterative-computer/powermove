import {it,expect,vi} from 'vitest';
import {decodeEnvironmentImage,validateEnvironmentImageHeader} from './environment-image';
import {tinyExr,tinyHdr} from './__fixtures__/environment-images';
it('decodes HDR and EXR to floating illumination without clamping highlights',async()=>{
  vi.stubGlobal('OffscreenCanvas',class{constructor(public width:number,public height:number){}getContext(){return {putImageData(){}};}});
  vi.stubGlobal('ImageData',class{constructor(public data:Uint8ClampedArray,public width:number,public height:number){}});
  try{
    for(const [name,bytes] of [['map.hdr',tinyHdr()],['map.exr',tinyExr()]] as const){const result=await decodeEnvironmentImage(new Blob([bytes as Uint8Array<ArrayBuffer>]),name);expect(result.w).toBe(2);expect(result.h).toBe(1);expect(result.environmentMap.pixels[0]!.x).toBeCloseTo(8,3);}
  }finally{vi.unstubAllGlobals();}
});
it('rejects oversized image headers before full decoding',()=>{
  const hdr=new TextEncoder().encode('#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 32768 +X 32768\n');
  expect(()=>validateEnvironmentImageHeader(hdr.buffer,'large.hdr')).toThrow('too large');
  const exr=tinyExr();const name=new TextEncoder().encode('dataWindow\0box2i\0');let at=0;
  for(let i=0;i<exr.length-name.length;i++)if(name.every((v,k)=>exr[i+k]===v)){at=i+name.length+4;break;}
  new DataView(exr.buffer).setInt32(at+8,100000,true);new DataView(exr.buffer).setInt32(at+12,100000,true);
  expect(()=>validateEnvironmentImageHeader(exr.buffer as ArrayBuffer,'large.exr')).toThrow('too large');
});
