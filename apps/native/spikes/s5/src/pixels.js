// All fixtures use real WebKit buffers/canvases. The native renderer is synthetic.
const frameBytes=1920*1080*4;
const expectedByte=i=>i%4===3?255:(i*17+23)&255;
function readPixels(path='pixels'){
 const x=new XMLHttpRequest();x.open('GET',base+path,false);
 x.overrideMimeType('text/plain; charset=x-user-defined');x.send();
 if(x.status!==200 || x.responseText.length!==frameBytes)throw Error('pixel payload size');
 const data=new Uint8ClampedArray(frameBytes);
 for(let i=0;i<data.length;i++)data[i]=x.responseText.charCodeAt(i)&255;
 return data;
}
function verifyPixels(bytes){if(bytes.length!==frameBytes)throw Error('wrong size');for(let i=0;i<bytes.length;i++)if(bytes[i]!==expectedByte(i))throw Error('pixel mismatch '+i);}
async function measured(name,fn,n=30){const a=[];fn();for(let i=0;i<n;i++){const t=performance.now();fn();a.push(performance.now()-t);}await report({test:name,status:'pass',...stats(a)});}
async function binary(){
 await measured('binary_download_text_decode_verify',()=>verifyPixels(readPixels()));
 const pixels=new Uint8Array(frameBytes);for(let i=0;i<frameBytes;i++)pixels[i]=expectedByte(i);
 await measured('binary_upload_arraybuffer_verify',()=>{if(!JSON.parse(sync('pixels-upload',pixels)).ok)throw Error('upload corruption');});
 let text='';const chunks=[];for(let i=0;i<frameBytes;i+=8192)chunks.push(String.fromCharCode(...pixels.subarray(i,i+8192)));text=chunks.join('');
 await measured('binary_upload_text_utf8_verify',()=>{if(!JSON.parse(sync('pixels-text-upload',text)).ok)throw Error('text upload corruption');});
 await measured('engine_ref_download',()=>{const r=JSON.parse(sync('frame-ref'));if(!r.ok||r.bytes!==frameBytes)throw Error('reference');},2000);
 await measured('engine_ref_upload',()=>{if(!JSON.parse(sync('consume-ref','frame0')).ok)throw Error('reference consumption');},2000);
}
function canvasFor(id){if(id!=='frame0'&&id!=='outer0')throw Error('unknown frame');const c=document.createElement('canvas');c.width=1920;c.height=1080;c.getContext('2d').putImageData(new ImageData(readPixels(id==='outer0'?'outer-pixels':'pixels'),1920,1080),0,0);return c;}
function capture(depth=0,trace=[]){
 if(depth>4)throw Error('dependency cycle');
 const r=JSON.parse(sync(depth?'render-nested':'render-begin'));trace.push({depth,status:r.status});
 if(r.status==='needs_dependency'){
  const nested=capture(depth+1,trace); // same-stack nested capture, no event-loop callback
  const pixel=nested.getContext('2d').getImageData(0,0,1,1).data;
  if(pixel[0]!==23)throw Error('nested capture pixels');
  const override=document.createElement('canvas');override.width=16;override.height=16;
  const ctx=override.getContext('2d');ctx.fillStyle='#23b45a';ctx.fillRect(0,0,16,16);
  const ack=JSON.parse(sync('pixels-upload',nested.getContext('2d').getImageData(0,0,1920,1080).data));
  if(!ack.ok)throw Error('nested capture upload failed');
  const uploaded=JSON.parse(sync('render-override?'+r.render+','+r.revision,ctx.getImageData(0,0,16,16).data));
  if(!uploaded.ok)throw Error('override rejected');
  const done=JSON.parse(sync('render-resume',JSON.stringify({render:r.render,revision:r.revision,dependency:ctx.getImageData(0,0,1,1).data[0]===35})));
  trace.push({depth,status:done.status});if(done.status!=='rendered')throw Error('resume failed');return canvasFor(done.id);
 }
 if(r.status!=='rendered')throw Error('render failed');return canvasFor(r.id);
}
async function continuations(){
 const trace=[];const c=capture(0,trace);
 if(!(c instanceof HTMLCanvasElement)||c.width!==1920)throw Error('real canvas required');
 const output=c.getContext('2d').getImageData(0,0,1920,1080).data;if(output[0]!==35||output[1]!==180||output[64]!==expectedByte(64))throw Error('override output not applied');
 const jpeg=c.toDataURL('image/jpeg');if(!jpeg.startsWith('data:image/jpeg;base64,'))throw Error('JPEG required');
 await report({test:'continuation_trace',status:'pass',trace,canvas:true,jpeg:true});
 await measured('render_continuation_nested_capture_canvas_fill',()=>capture(),30);
 const stale=JSON.parse(sync('render-resume',JSON.stringify({render:999999,revision:7,dependency:true})));
 if(stale.status!=='failed')throw Error('unknown render accepted');
 await report({test:'continuation_cycle_or_stale_rejection',status:'pass'});
}
