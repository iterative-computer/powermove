/** Minimal lossless RGBA16 PNG writer. Float source values are never quantized to
 * RGBA8 on the way to disk. Data passes omit display color-space metadata. */
function crc32(bytes:Uint8Array):number{let crc=0xffffffff;for(const byte of bytes){crc^=byte;for(let k=0;k<8;k++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}return (crc^0xffffffff)>>>0;}
function chunk(type:string,bytes:Uint8Array):Uint8Array<ArrayBuffer>{
  const result=new Uint8Array(bytes.length+12),view=new DataView(result.buffer);view.setUint32(0,bytes.length);
  result.set(new TextEncoder().encode(type),4);result.set(bytes,8);view.setUint32(bytes.length+8,crc32(result.subarray(4,bytes.length+8)));return result;
}
export async function png16(width:number,height:number,pixels:Float32Array,display=false):Promise<Blob>{
  if(!Number.isInteger(width)||!Number.isInteger(height)||width<1||height<1||pixels.length!==width*height*4)throw new Error('Invalid PNG dimensions');
  const stride=width*8+1,raw=new Uint8Array(stride*height),data=new DataView(raw.buffer);
  for(let y=0;y<height;y++)for(let x=0;x<width*4;x++){
    const value=pixels[(y*width*4)+x]!;data.setUint16(y*stride+1+x*2,Math.round(Math.max(0,Math.min(1,Number.isFinite(value)?value:0))*65535));
  }
  const header=new Uint8Array(13),view=new DataView(header.buffer);view.setUint32(0,width);view.setUint32(4,height);header.set([16,6,0,0,0],8);
  const compressed=new Uint8Array(await new Response(new Blob([raw]).stream().pipeThrough(new CompressionStream('deflate'))).arrayBuffer());
  const parts:BlobPart[]=[new Uint8Array([137,80,78,71,13,10,26,10]),chunk('IHDR',header)];
  if(display)parts.push(chunk('sRGB',new Uint8Array([0])));
  parts.push(chunk('IDAT',compressed),chunk('IEND',new Uint8Array()));return new Blob(parts,{type:'image/png'});
}
