import {expect,it} from 'vitest';
import {StudioBinaryWriter} from './buffers';
it('streams all byte ranges in order, including offset typed-array views',()=>{
 const writer=new StudioBinaryWriter(),source=new Uint8Array([9,1,2,3,9]);
 expect(writer.append(source.subarray(1,4))).toEqual({offset:0,length:3});
 writer.append(new Uint8Array([4,5,6,7]));
 const chunks=[...writer.chunks(2)];
 expect(chunks.map(c=>c.offset)).toEqual([0,2,3,5]);
 expect(chunks.flatMap(c=>[...c.bytes])).toEqual([1,2,3,4,5,6,7]);expect(writer.byteLength).toBe(7);
 expect([...writer.finish()]).toEqual([1,2,3,4,5,6,7]);
});
