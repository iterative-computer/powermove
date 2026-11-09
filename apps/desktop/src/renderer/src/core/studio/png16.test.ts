import {it,expect} from 'vitest';
import {inflateSync} from 'node:zlib';
import {png16} from './png16';
it('writes actual 16-bit samples, preserves precision and labels display color only when requested',async()=>{
  const pixels=new Float32Array([.12345,.5,1,1,0,0,0,0]);
  const bytes=new Uint8Array(await (await png16(2,1,pixels)).arrayBuffer());
  expect([...bytes.slice(0,8)]).toEqual([137,80,78,71,13,10,26,10]);expect(bytes[24]).toBe(16);expect(bytes[25]).toBe(6);
  let at=8;const chunks=[];while(at<bytes.length){const length=new DataView(bytes.buffer).getUint32(at),type=new TextDecoder().decode(bytes.slice(at+4,at+8));chunks.push(type);if(type==='IDAT'){
    const raw=inflateSync(bytes.slice(at+8,at+8+length));expect(raw.readUInt16BE(1)).toBe(Math.round(.12345*65535));expect(raw.readUInt16BE(3)).toBe(32768);
  }at+=length+12;}
  expect(chunks).not.toContain('sRGB');const display=new Uint8Array(await (await png16(2,1,pixels,true)).arrayBuffer());expect(new TextDecoder().decode(display)).toContain('sRGB');
});
