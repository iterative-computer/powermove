import React, {createRef} from 'react';
import {createRoot} from 'react-dom/client';
import {flushSync} from 'react-dom';
import {DistortionWave} from '@upstream/DistortionWave';
const ref=createRef<any>();
const root=createRoot(document.getElementById('root')!);
const images=await Promise.all(['from','to'].map(name=>new Promise<HTMLImageElement>(resolve=>{const img=new Image();img.onload=()=>resolve(img);img.src=`${name}.png`;})));
let props:any={};
(window as any).setFrame=async(type:string,palette:string,scrub:number)=>{
 const source= type==='vertical'?{x:235,y:330,w:150,h:44}:type==='horizontal'?{x:12,y:120,w:44,h:160}:{x:280,y:170,w:60,h:60};
 props={type,palette,scrub,source,cornerRadius:28,live:false,prewarm:false,resolution:1,reducedMotion:'never',capture:async({slot}:any)=>images[slot==='content'?0:1]};
 flushSync(()=>root.render(<DistortionWave key={`${type}/${palette}`} ref={ref} {...props} style={{width:620,height:400,borderRadius:28,overflow:'hidden'}} revealedContent={<img src="to.png"/>}><img src="from.png"/></DistortionWave>));
 await ref.current.prepare();
 await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
};
