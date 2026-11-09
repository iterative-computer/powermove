/** Two-pixel, uncompressed scanline EXR in linear Rec.709, with HDR red values. */
export function tinyExr():Uint8Array{
  const bytes:number[]=[],text=(s:string)=>{bytes.push(...new TextEncoder().encode(s),0);},u32=(v:number)=>{const b=new Uint8Array(4);new DataView(b.buffer).setUint32(0,v,true);bytes.push(...b);};
  const float=(v:number)=>{const b=new Uint8Array(4);new DataView(b.buffer).setFloat32(0,v,true);return [...b];};
  const attribute=(name:string,type:string,value:number[])=>{text(name);text(type);u32(value.length);bytes.push(...value);};
  u32(20000630);u32(2);
  const channels:number[]=[];
  for(const name of ['B','G','R'])channels.push(name.charCodeAt(0),0,2,0,0,0,0,0,0,0,1,0,0,0,1,0,0,0);
  channels.push(0);attribute('channels','chlist',channels);attribute('compression','compression',[0]);
  const box=[0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0];attribute('dataWindow','box2i',box);attribute('displayWindow','box2i',box);
  attribute('lineOrder','lineOrder',[0]);attribute('pixelAspectRatio','float',float(1));attribute('screenWindowCenter','v2f',[...float(0),...float(0)]);attribute('screenWindowWidth','float',float(1));bytes.push(0);
  const offset=bytes.length+8;u32(offset);u32(0);u32(0);u32(24);
  for(const value of [.5,1,2,3,8,16])bytes.push(...float(value));
  return new Uint8Array(bytes);
}
export function tinyHdr():Uint8Array{return new Uint8Array([...new TextEncoder().encode('#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n'),128,64,32,132,32,64,128,130]);}
