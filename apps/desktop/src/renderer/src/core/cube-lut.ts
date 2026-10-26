/** Bounded standalone IRIDAS .cube data. No file includes or executable config. */
export const MAX_CUBE_BYTES=16*1024*1024;
export interface CubeLut {size:number;dimensions:1|3;domainMin:[number,number,number];domainMax:[number,number,number];data:Float32Array;width:number;height:number}
export function parseCube(text:string):CubeLut{
 if(text.length>MAX_CUBE_BYTES)throw new Error('LUT exceeds the 16 MiB text limit');
 let size=0,dimensions:1|3=3,domainMin:[number,number,number]=[0,0,0],domainMax:[number,number,number]=[1,1,1],row=0,data:Float32Array|null=null;const seen=new Set<string>();
 const numbers=(s:string,n:number)=>{const words=s.trim().split(/\s+/);if(words.length!==n||words.some(w=>!/^[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?$/.test(w)))throw new Error('Invalid LUT numeric row');const values=words.map(Number);if(values.some(v=>!Number.isFinite(v)||Math.abs(v)>65504))throw new Error('LUT values must be finite and within ±65504');return values;};
 for(const line of text.replace(/^\uFEFF/,'').split(/\r?\n/)){const s=(line.match(/^(?:"[^"]*"|[^"#])*/)?.[0]??'').trim();if(!s)continue;const match=/^([A-Z][A-Z0-9_]*)\s+(.*)$/.exec(s);
  if(match){const [,key,value]=match;if(seen.has(key!))throw new Error(`Duplicate LUT directive ${key}`);seen.add(key!);if(row)throw new Error('LUT headers must precede data');
   if(key==='TITLE'){if(!/^"[^"\r\n]*"$/.test(value!))throw new Error('Invalid LUT title');continue;}
   if(key==='DOMAIN_MIN'||key==='DOMAIN_MAX'){const v=numbers(value!,3) as [number,number,number];if(key==='DOMAIN_MIN')domainMin=v;else domainMax=v;continue;}
   if(key==='LUT_1D_SIZE'||key==='LUT_3D_SIZE'){if(size)throw new Error('Combined 1D and 3D LUTs are not supported');const n=numbers(value!,1)[0]!;dimensions=key==='LUT_1D_SIZE'?1:3;if(!Number.isInteger(n)||n<2||n>(dimensions===1?4096:64))throw new Error('LUT size must be 2–4096 (1D) or 2–64 (3D)');size=n;data=new Float32Array((dimensions===1?n:n**3)*4);continue;}
   throw new Error(`Unsupported LUT directive ${key}`);
  }
  if(!data)throw new Error('LUT size must precede data');const v=numbers(s,3);if(row>=data.length/4)throw new Error('Too many LUT rows');data.set([...v,1],row++*4);
 }
 if(!data||row!==data.length/4)throw new Error('LUT row count does not match its declared size');domainMin=domainMin.map(Math.fround) as [number,number,number];domainMax=domainMax.map(Math.fround) as [number,number,number];if(domainMin.some((v,i)=>domainMax[i]!-v<1e-30))throw new Error('LUT domain must increase on every channel at GPU precision');
 return{size,dimensions,domainMin,domainMax,data,width:dimensions===1?size:size*size,height:dimensions===1?1:size};
}
