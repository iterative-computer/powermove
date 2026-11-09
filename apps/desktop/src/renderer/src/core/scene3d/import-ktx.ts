import * as THREE from 'three';

export function validateKtx2(bytes:ArrayBuffer):void {
  const magic=[0xab,0x4b,0x54,0x58,0x20,0x32,0x30,0xbb,0x0d,0x0a,0x1a,0x0a];
  if(bytes.byteLength<80 || bytes.byteLength>64*1024*1024 || !magic.every((v,i)=>new Uint8Array(bytes)[i]===v))throw new Error('Invalid or oversized KTX2 texture');
  const view=new DataView(bytes),width=view.getUint32(20,true),height=view.getUint32(24,true);
  if(!width || !height || width>16384 || height>16384 || width*height>64_000_000 || view.getUint32(28,true)>1 || view.getUint32(32,true)>1 || view.getUint32(36,true)!==1 || view.getUint32(40,true)>16)throw new Error('KTX2 texture exceeds the 2D image budget');
}

type Decoded={ok:true;pixels:Uint8Array;width:number;height:number;colorSpace:string}|{ok:false;message:string};
/** At most two isolated Basis decoders run at once for one model. */
export class StudioKtx2Loader extends THREE.Loader<THREE.DataTexture> {
  private workers=new Set<Worker>();
  private rejectPending=new Set<(error:Error)=>void>();
  private closed=false;
  private active=0;
  private waiting:{resolve:()=>void;reject:(error:Error)=>void}[]=[];
  private acquire():Promise<void>{
    if(this.active<2){this.active++;return Promise.resolve();}
    return new Promise((resolve,reject)=>this.waiting.push({resolve,reject}));
  }
  private release():void{this.active--;if(!this.closed && this.waiting.length){this.active++;this.waiting.shift()!.resolve();}}
  load(url:string,onLoad:(texture:THREE.DataTexture)=>void,onProgress?:((event:ProgressEvent)=>void),onError?:(error:unknown)=>void):THREE.DataTexture {
    const texture=new THREE.DataTexture(),file=new THREE.FileLoader(this.manager).setResponseType('arraybuffer');
    file.load(url,data=>{
      void this.decode(data as ArrayBuffer).then(result=>{
        texture.image={data:result.pixels,width:result.width,height:result.height};texture.colorSpace=result.colorSpace as THREE.ColorSpace;
        texture.flipY=false;texture.needsUpdate=true;onLoad(texture);
      }).catch(error=>{onError?.(error);this.manager.itemError(url);});
    },onProgress,onError);
    return texture;
  }
  private async decode(bytes:ArrayBuffer):Promise<Extract<Decoded,{ok:true}>> {
    validateKtx2(bytes);if(this.closed)throw new Error('KTX2 importer is closed');
    await this.acquire();
    try {
    if(this.closed)throw new Error('KTX2 importer is closed');
    return await new Promise<Extract<Decoded,{ok:true}>>((resolve,reject)=>{
      const worker=new Worker(new URL('./ktx.worker.ts',import.meta.url),{type:'module'});this.workers.add(worker);
      const finish=()=>{clearTimeout(timer);worker.terminate();this.workers.delete(worker);this.rejectPending.delete(cancel);};
      const cancel=(error:Error)=>{finish();reject(error);};this.rejectPending.add(cancel);
      const timer=setTimeout(()=>cancel(new Error('KTX2 texture decoding timed out')),30_000);
      worker.onmessage=(event:MessageEvent<Decoded>)=>{const result=event.data;finish();if(result.ok)resolve(result);else reject(new Error(result.message));};
      worker.onerror=event=>cancel(new Error(event.message || 'KTX2 texture decoding failed'));
      try {worker.postMessage(bytes,[bytes]);}catch(error){cancel(error instanceof Error?error:new Error(String(error)));}
    });
    }finally{this.release();}
  }
  dispose():void {this.closed=true;for(const job of this.waiting.splice(0))job.reject(new Error('KTX2 importer is closed'));for(const reject of [...this.rejectPending])reject(new Error('KTX2 importer is closed'));for(const worker of this.workers)worker.terminate();this.workers.clear();}
}
