import { readFileSync } from 'node:fs';
import { afterEach,describe,expect,it,vi } from 'vitest';
import { StudioKtx2Loader,validateKtx2 } from './import-ktx';
afterEach(()=>vi.unstubAllGlobals());
describe('KTX2 input budgets',()=>{
  it('accepts the generated 246-byte Basis fixture and rejects malformed dimensions before creating a worker',()=>{
    const source=readFileSync('e2e/fixtures/tiny-basis.ktx2'),bytes=source.buffer.slice(source.byteOffset,source.byteOffset+source.byteLength);
    expect(()=>validateKtx2(bytes)).not.toThrow();
    const huge=bytes.slice(0);new DataView(huge).setUint32(20,1_000_000,true);expect(()=>validateKtx2(huge)).toThrow('budget');
    const cube=bytes.slice(0);new DataView(cube).setUint32(36,6,true);expect(()=>validateKtx2(cube)).toThrow('2D');
    expect(()=>validateKtx2(new ArrayBuffer(80))).toThrow('Invalid');
  });
  it('bounds simultaneous decoder workers and cancels queued images on disposal',async()=>{
    const workers:any[]=[];let active=0,peak=0;
    vi.stubGlobal('Worker',class{
      onmessage:any;onerror:any;closed=false;
      constructor(){workers.push(this);peak=Math.max(peak,++active);}
      postMessage(){}
      terminate(){if(!this.closed){this.closed=true;active--;}}
    });
    const source=readFileSync('e2e/fixtures/tiny-basis.ktx2'),loader=new StudioKtx2Loader();
    const results=Promise.allSettled(Array.from({length:5},()=> (loader as any).decode(source.buffer.slice(source.byteOffset,source.byteOffset+source.byteLength))));
    try {
      await vi.waitFor(()=>expect(workers).toHaveLength(2));
      workers[0].onmessage({data:{ok:true,pixels:new Uint8Array(4),width:1,height:1,colorSpace:'srgb'}});
      await vi.waitFor(()=>expect(workers).toHaveLength(3));expect(peak).toBe(2);
      loader.dispose();const settled=await results;
      expect(settled.filter(result=>result.status==='fulfilled')).toHaveLength(1);
      expect(settled.filter(result=>result.status==='rejected')).toHaveLength(4);expect(active).toBe(0);
    }finally{loader.dispose();}
  });
});
