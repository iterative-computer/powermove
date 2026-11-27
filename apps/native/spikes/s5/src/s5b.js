// S5b keeps the control plane pixel-free. Only explicit promotion requests an image.
function idSync(path){const x=new XMLHttpRequest();x.open('GET','s5probe://kernel/s5b/'+path,false);x.send();return x.responseText;}
async function s5bMeasure(name,fn,n=30){const a=[];await fn(-1);for(let i=0;i<n;i++){const t=performance.now();await fn(i);a.push(performance.now()-t);}await report({test:name,...stats(a)});}
function target(){const c=document.createElement('canvas');c.width=1920;c.height=1080;return c;}
function checkCanvas(c,kind){const p=c.getContext('2d').getImageData(0,0,1,1).data;if(kind==='outer'){if(p[0]!==35||p[1]!==180||p[2]!==90||p[3]!==255)throw Error('outer pixel');}else if(kind==='pattern'&&(p[0]!==23||p[1]!==40||p[2]!==57||p[3]!==255))throw Error('pattern pixel');else if(kind==='noise'){let seed=0x13245678;seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;if(p[0]!==((seed>>>0)&255)||p[1]!==((seed>>>8)&255)||p[2]!==((seed>>>16)&255)||p[3]!==255)throw Error('noise pixel');}}
async function withDeadline(promise){let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('decode deadline')),3000);})]);}finally{clearTimeout(timer);}}
async function loadImage(url){const img=new Image();img.crossOrigin="anonymous";img.src=url;await withDeadline(img.decode());return img;}
async function s5bImages(){
 const filter=testCase.split('-').slice(2).join('_');
 for(const kind of ['pattern','noise'])for(const format of ['bmp','png','tiff'])for(const transport of ['scheme','stream','http']){
  if(filter && filter!==`${transport}_${kind}_${format}`)continue;
  const root=transport==='scheme'?'s5probe://kernel/s5b/':transport==='stream'?'s5probe://kernel/s5b/stream/':base+'s5b/';const url=root+kind+'.'+format;
  try{
   const c=target(),ctx=c.getContext('2d');
   await s5bMeasure(`image_${transport}_${kind}_${format}_decode_fill`,async i=>{const image=await loadImage(url+'?fresh='+i);ctx.drawImage(image,0,0);checkCanvas(c,kind);});
   const all=ctx.getImageData(0,0,1920,1080).data;let seed=0x13245678;
   for(let p=0;p<1920*1080;p++){
    if(kind==='noise'){seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;}
    for(let k=0;k<4;k++){const want=k===3?255:kind==='noise'?(seed>>>(k*8))&255:expectedByte(p*4+k);if(all[p*4+k]!==want)throw Error('full pixel mismatch '+p+':'+k);}
   }
   await report({test:`integrity_${transport}_${kind}_${format}`,bytes:all.length,passed:true});
   const img=await loadImage(url+'?cached');
   await s5bMeasure(`image_${transport}_${kind}_${format}_warm_fill`,()=>{ctx.drawImage(img,0,0);checkCanvas(c,kind);});
   await s5bMeasure(`bitmap_${transport}_${kind}_${format}_decode_fill`,async i=>{const blob=await withDeadline(fetch(url+'?bitmap='+i).then(r=>r.blob()));const bitmap=await withDeadline(createImageBitmap(blob));ctx.drawImage(bitmap,0,0);checkCanvas(c,kind);bitmap.close();});
  }catch(e){await report({test:`image_candidate_${transport}_${kind}_${format}`,unsupported:String(e)});}
 }
 // A fresh image cannot acquire new pixels while a synchronous caller blocks
 // WebContent's event loop. Record the actual cold same-stack behavior.
 const c=target(),img=new Image();img.src='s5probe://kernel/s5b/pattern.bmp?cold-sync';
 let error=null;try{c.getContext('2d').drawImage(img,0,0);}catch(e){error=e.name;}
 await report({test:'cold_sync_image',complete:img.complete,width:img.naturalWidth,alpha:c.getContext('2d').getImageData(0,0,1,1).data[3],error});
 const glc=target(),gl=glc.getContext('webgl2',{preserveDrawingBuffer:true});
 if(!gl){await report({test:'webgl_image',unsupported:'no WebGL2 context in this host'});return;}
 const image=await loadImage(base+'s5b/noise.bmp?gl');
 const vs=gl.createShader(gl.VERTEX_SHADER);gl.shaderSource(vs,'#version 300 es\nin vec2 p;out vec2 uv;void main(){uv=(p+1.)*.5;gl_Position=vec4(p,0,1);}');gl.compileShader(vs);
 const fs=gl.createShader(gl.FRAGMENT_SHADER);gl.shaderSource(fs,'#version 300 es\nprecision highp float;in vec2 uv;uniform sampler2D tex;out vec4 color;void main(){color=texture(tex,uv);}');gl.compileShader(fs);
 const prog=gl.createProgram();gl.attachShader(prog,vs);gl.attachShader(prog,fs);gl.linkProgram(prog);if(!gl.getProgramParameter(prog,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(prog));gl.useProgram(prog);
 const buf=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,buf);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([-1,-1,1,-1,-1,1,1,1]),gl.STATIC_DRAW);const at=gl.getAttribLocation(prog,'p');gl.enableVertexAttribArray(at);gl.vertexAttribPointer(at,2,gl.FLOAT,false,0,0);
 gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL,true);
 const tex=gl.createTexture();gl.bindTexture(gl.TEXTURE_2D,tex);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);const dst=target(),ctx=dst.getContext('2d');
 await s5bMeasure('webgl_warm_image_upload_draw_canvas_fill',()=>{gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA,gl.RGBA,gl.UNSIGNED_BYTE,image);gl.drawArrays(gl.TRIANGLE_STRIP,0,4);gl.finish();ctx.drawImage(glc,0,0);ctx.getImageData(0,0,1,1);if(gl.getError())throw Error('GL error');});
 const pixel=ctx.getImageData(0,0,1,1).data;let seed=0x13245678;seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;if(pixel[0]!==((seed>>>0)&255)||pixel[1]!==((seed>>>8)&255)||pixel[2]!==((seed>>>16)&255)||pixel[3]!==255)throw Error('GL output pixel');
 await report({test:'webgl_canvas_integrity',passed:true});
}
function rasterOverrideId(source){
 // Recorder-side callback: preserve versioned drawImage source plus native fill.
 // A fixed four-command fixture, not the full Canvas2D recorder implementation.
 return {source,fill:[35,180,90,255],rect:[0,0,16,16]};
}
function idCapture(){
 const r=idSync('begin').split(':');if(r[0]!=='need'||r[4]!=='4')throw Error('begin batch');
 const nested=idSync('nested/7');if(nested!=='surface:pattern:7')throw Error('nested');
 const recordings=Array.from({length:4},()=>rasterOverrideId(nested));
 if(recordings.some(c=>c.source!==nested||c.fill[0]!==35))throw Error('recording');
 const out=idSync(`resume/${r[1]}/${r[2]}/fill-35-180-90-255x${recordings.length}`);if(out!=='surface:outer:7')throw Error('resume');return out;
}
async function s5bContinuation(){
 await measured('s5b_scheme_id_call',()=>{if(idSync('id')!=='surface:pattern:7')throw Error('id');},2000);
 await measured('s5b_batched_4_overrides_nested_ids',idCapture,2000);
 const c=target(),ctx=c.getContext('2d'),img=await loadImage(base+'s5b/outer.bmp?preload');
 await measured('s5b_batched_continuation_warm_canvas',()=>{idCapture();ctx.drawImage(img,0,0);checkCanvas(c,'outer');},100);
 await s5bMeasure('s5b_batched_continuation_cold_canvas',async i=>{idCapture();const im=await loadImage(base+'s5b/outer.bmp?render='+i);ctx.drawImage(im,0,0);checkCanvas(c,'outer');},30);
 await measured('s5b_getImageData_1080p',()=>ctx.getImageData(0,0,1920,1080),30);
 await measured('s5b_snapshot_jpeg',()=>{if(!c.toDataURL('image/jpeg').startsWith('data:image/jpeg;base64,'))throw Error('jpeg');},30);
}
async function s5bProfile(){
 // Reproduce the old exact fixture and split each cost; measured phases do not
 // include report HTTP requests. Same-stack sync calls only.
 const times={};const step=(name,fn)=>{const t=performance.now(),r=fn();(times[name]??=[]).push(performance.now()-t);return r;};
 for(let i=0;i<31;i++){
  const r=step('control_begin',()=>JSON.parse(sync('render-begin')));
  step('control_nested',()=>JSON.parse(sync('render-nested')));
  const pixels=step('nested_download_decode',()=>readPixels());
  const c=target(),ctx=c.getContext('2d');step('nested_canvas_fill',()=>ctx.putImageData(new ImageData(pixels,1920,1080),0,0));
  const bytes=step('nested_readback',()=>ctx.getImageData(0,0,1920,1080).data);
  step('nested_upload_verify',()=>sync('pixels-upload',bytes));
  step('override_valid',()=>sync('render-override?'+r.render+','+r.revision,new Uint8Array(Array.from({length:256},()=>[35,180,90,255]).flat())));
  step('control_resume_native_render',()=>sync('render-resume',JSON.stringify({render:r.render,revision:r.revision,dependency:true})));
  const outer=step('outer_download_decode',()=>readPixels('outer-pixels'));
  step('outer_canvas_fill',()=>ctx.putImageData(new ImageData(outer,1920,1080),0,0));
 }
 for(const [name,a]of Object.entries(times))await report({test:'profile_'+name,...stats(a.slice(1))});
}
async function s5b(){if(testCase.startsWith('s5b-images'))await s5bImages();else if(testCase==='s5b-profile')await s5bProfile();else if(testCase==='s5b-continuation')await s5bContinuation();else throw Error('unknown s5b case');}
