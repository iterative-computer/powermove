import { FloatType } from 'three';
import { HDRLoader } from 'three/addons/loaders/HDRLoader.js';
import { EXRLoader } from 'three/addons/loaders/EXRLoader.js';

const MAX_PIXELS=33554432;
/** Read dimensions before a compressed EXR can allocate its decoded image. */
export function validateEnvironmentImageHeader(bytes:ArrayBuffer,name:string):void{
  const data=new DataView(bytes),raw=new Uint8Array(bytes);
  const check=(w:number,h:number)=>{if(!Number.isSafeInteger(w)||!Number.isSafeInteger(h)||w<1||h<1||w*h>MAX_PIXELS)throw new Error('Environment image is too large');};
  if(/\.exr$/i.test(name)){
    let at=8;const string=()=>{const start=at;while(at<raw.length && raw[at]!==0)at++;if(at>=raw.length)throw new Error('Invalid EXR header');const value=new TextDecoder().decode(raw.subarray(start,at));at++;return value;};
    while(at<Math.min(raw.length,1024*1024)){
      const attribute=string();if(!attribute)return;const type=string();if(at+4>raw.length)throw new Error('Invalid EXR header');const size=data.getUint32(at,true);at+=4;
      if(size>raw.length-at)throw new Error('Invalid EXR attribute');
      if(attribute==='dataWindow' && type==='box2i' && size===16){check(data.getInt32(at+8,true)-data.getInt32(at,true)+1,data.getInt32(at+12,true)-data.getInt32(at+4,true)+1);return;}at+=size;
    }
    throw new Error('Missing EXR data window');
  }
  const header=new TextDecoder().decode(raw.subarray(0,Math.min(raw.length,65536))),size=/[+-]Y\s+(\d+)\s+[+-]X\s+(\d+)/.exec(header);
  if(size)check(Number(size[2]),Number(size[1]));
}
/** Keep linear HDR pixels for lighting; the media browser uses a small SDR poster. */
export async function decodeEnvironmentImage(blob:Blob,name:string){
  if(blob.size>128*1024*1024)throw new Error('Environment images must be smaller than 128 MB');
  const bytes=await blob.arrayBuffer();validateEnvironmentImageHeader(bytes,name);
  const decoded=/\.exr$/i.test(name)?new EXRLoader().setDataType(FloatType).parse(bytes):new HDRLoader().setDataType(FloatType).parse(bytes);
  if(!decoded || !decoded.width || !decoded.height || decoded.width*decoded.height>MAX_PIXELS)throw new Error('Environment image is too large');
  const scale=Math.min(1,2048/decoded.width,1024/decoded.height),width=Math.max(1,Math.round(decoded.width*scale)),height=Math.max(1,Math.round(decoded.height*scale));
  // r180 HDRLoader divides the RGBE mantissa by 255. Radiance and the
  // engine image codec use 256, so retain the same linear energy in both.
  const hdrScale=/\.exr$/i.test(name)?1:255/256;
  const data=decoded.data as Float32Array,stride=data.length/(decoded.width*decoded.height),pixels=[];
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){
    const at=(Math.min(decoded.height-1,Math.floor(y/scale))*decoded.width+Math.min(decoded.width-1,Math.floor(x/scale)))*stride;
    const read=(k:number)=>Number.isFinite(data[at+k])?Math.max(0,data[at+k]!*hdrScale):0;
    pixels.push({x:read(0),y:read(1),z:read(2)});
  }
  const el=new OffscreenCanvas(width,height),poster=new Uint8ClampedArray(width*height*4);
  const display=(v:number)=>Math.round(255*Math.pow(v/(1+v),1/2.2));
  pixels.forEach((p,i)=>poster.set([display(p.x),display(p.y),display(p.z),255],i*4));
  el.getContext('2d')!.putImageData(new ImageData(poster,width,height),0,0);
  return {el,w:width,h:height,environmentMap:{width,height,pixels},format:/\.exr$/i.test(name)?'exr':'hdr'};
}
