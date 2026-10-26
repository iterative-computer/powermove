import {expect,it} from 'vitest';
import {parseCube,MAX_CUBE_BYTES} from './cube-lut';
it('parses red-fastest cube rows, non-default domains and overrange outputs without quantization',()=>{
 const rows=Array.from({length:8},(_,i)=>`${i&1?2:-.5} ${i&2?1:0} ${i&4?1:0}`).join('\n');const lut=parseCube(`\uFEFF# portable table\nTITLE "Test #1"\nLUT_3D_SIZE 2\nDOMAIN_MIN -1 0 0\nDOMAIN_MAX 2 1 1\n${rows}`);
 expect(lut.width).toBe(4);expect(lut.height).toBe(2);expect(lut.domainMin).toEqual([-1,0,0]);expect([...lut.data.slice(0,8)]).toEqual([-.5,0,0,1,2,0,0,1]);
});
it('accepts 1D channel tables and preserves exact float samples',()=>{const lut=parseCube('LUT_1D_SIZE 2\n0 0 0\n1e0 .5 -0.25');expect(lut.dimensions).toBe(1);expect(lut.height).toBe(1);expect([...lut.data.slice(4)]).toEqual([1,.5,-.25,1]);});
it.each(['LUT_3D_SIZE 65','LUT_1D_SIZE 4097','LUT_3D_SIZE 2\n0 0 0','LUT_1D_SIZE 2\n0 0 0\n1 1 1\n2 2 2','LUT_1D_SIZE 2\n0 NaN 0\n1 1 1','LUT_1D_SIZE 2\n0 0 0\n1e10 1 1','LUT_1D_SIZE 2\nLUT_3D_SIZE 2','LUT_1D_SIZE 2\nDOMAIN_MIN 1 0 0\n0 0 0\n1 1 1','INCLUDE "http://example.test/a"','LUT_1D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MIN 0 0 0\n0 0 0\n1 1 1','LUT_1D_SIZE 2\n0 0 0\nTITLE "Late"\n1 1 1'])('rejects malformed or unsupported tables: %s',text=>{expect(()=>parseCube(text)).toThrow();});
it('rejects text before unbounded splitting or float allocation',()=>{expect(()=>parseCube(' '.repeat(MAX_CUBE_BYTES+1))).toThrow('text limit');});

it('rejects domains that collapse or underflow at GPU uniform precision',()=>{
 for(const header of ['DOMAIN_MIN 65503.9999 0 0\nDOMAIN_MAX 65504 1 1','DOMAIN_MAX 1e-40 1 1'])expect(()=>parseCube(`LUT_1D_SIZE 2\n${header}\n0 0 0\n1 1 1`)).toThrow('GPU precision');
});
