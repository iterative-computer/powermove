import {prepareEnvironment} from './environment-preparation';
self.onmessage=(event:MessageEvent)=>{
 try{const result=prepareEnvironment(event.data);const buffers=result.flatMap(part=>Object.values(part).filter(v=>ArrayBuffer.isView(v)).map(v=>(v as ArrayBufferView).buffer));
  (self as any).postMessage({result},buffers);
 }catch(error){(self as any).postMessage({error:String(error)});}
};
