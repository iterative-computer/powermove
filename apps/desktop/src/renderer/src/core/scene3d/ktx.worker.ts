import * as THREE from 'three';
import transcoderJs from 'three/examples/jsm/libs/basis/basis_transcoder.js?url';
import transcoderWasm from 'three/examples/jsm/libs/basis/basis_transcoder.wasm?url';

self.onmessage=async(event:MessageEvent<ArrayBuffer>)=>{
  let file:any;
  try {
    const [source,binary]=await Promise.all([fetch(transcoderJs).then(r=>r.text()),fetch(transcoderWasm).then(r=>r.arrayBuffer())]);
    // Evaluate only the installed Emscripten binding, never model contents.
    // Its isolated worker avoids function-to-string minifier hazards.
    const factory=new Function(`${source}\nreturn BASIS;`)();
    const basis:any=await new Promise((resolve,reject)=>{
      const module:any={wasmBinary:binary,onRuntimeInitialized:()=>resolve(module),onAbort:(reason:any)=>reject(new Error(String(reason)))};
      const started=factory(module);started?.catch?.(reject);
    });
    basis.initializeBasis();file=new basis.KTX2File(new Uint8Array(event.data));
    if(!file.isValid() || file.isHDR())throw new Error('KTX2 material images must use LDR ETC1S or UASTC Basis encoding');
    if(!file.startTranscoding())throw new Error('KTX2 texture transcoding failed');
    // Basis Universal cTFRGBA32; both Studio backends upload this format.
    const format=13,pixels=new Uint8Array(file.getImageTranscodedSizeInBytes(0,0,0,format));
    if(!file.transcodeImage(pixels,0,0,0,format,0,-1,-1))throw new Error('KTX2 texture transcoding failed');
    let width=file.getWidth(),height=file.getHeight();
    if(pixels.byteLength!==width*height*4){const info=file.getImageLevelInfo(0,0,0);width=info.width;height=info.height;}
    if(pixels.byteLength!==width*height*4)throw new Error('Invalid KTX2 decoded pixel count');
    const view=new DataView(event.data),dfd=view.getUint32(48,true);
    if(dfd+16>event.data.byteLength)throw new Error('Invalid KTX2 color descriptor');
    const flags=view.getUint8(dfd+15),transfer=view.getUint8(dfd+14);
    if((flags&1)!==0)for(let i=0;i<pixels.length;i+=4){const alpha=pixels[i+3]!;if(alpha>0)for(let c=0;c<3;c++)pixels[i+c]=Math.min(255,Math.round(pixels[i+c]!*255/alpha));}
    (self as any).postMessage({ok:true,pixels,width,height,colorSpace:transfer===2?THREE.SRGBColorSpace:THREE.LinearSRGBColorSpace},[pixels.buffer]);
  }catch(error){(self as any).postMessage({ok:false,message:error instanceof Error?error.message:String(error)});}
  finally {file?.close();file?.delete();}
};
