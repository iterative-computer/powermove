import { MAP_SIZE } from './extract';
import type { StudioTextureSource } from '../scene3d/studio-backend';

export function mapCanvas(source:any,flipY:boolean,size=MAP_SIZE):OffscreenCanvas {
  const canvas=new OffscreenCanvas(size,size),ctx=canvas.getContext('2d')!;
  if(flipY){ctx.translate(0,size);ctx.scale(1,-1);}
  if(source.data && source.width && source.height){
    const image=new OffscreenCanvas(source.width,source.height);
    const data=source.data instanceof Uint8ClampedArray?source.data:new Uint8ClampedArray(source.data);
    image.getContext('2d')!.putImageData(new ImageData(data,source.width,source.height),0,0);
    ctx.drawImage(image,0,0,size,size);
  }else ctx.drawImage(source,0,0,size,size);
  return canvas;
}
export function sizeStudioTexture(source:StudioTextureSource,size:number):StudioTextureSource {
  if(source instanceof Uint8Array){
    if(source.length===size*size*4)return source;
    const side=Math.sqrt(source.length/4);
    if(!Number.isInteger(side))throw new Error('Invalid Studio texture pixel count');
    return mapCanvas({data:source,width:side,height:side},false,size);
  }
  return source.width===size && source.height===size?source:mapCanvas(source,false,size);
}
