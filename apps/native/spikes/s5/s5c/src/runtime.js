// Deliberately small recorder, NOT a DOM/Canvas polyfill. Unsupported shapes throw.
function check(x,m='assertion'){if(!x)throw Error(m)}
function stats(a){a.sort((x,y)=>x-y);return{n:a.length,p50:a[Math.ceil(a.length*.5)-1],p95:a[Math.ceil(a.length*.95)-1],max:a.at(-1)}}
function bench(fn,n=30,warm=true){if(warm){fn();host('clear')}const a=[];for(let i=0;i<n;i++){host('clear');const t=hostNow();fn(i);a.push(hostNow()-t)}return stats(a)}
class Recorder {
 constructor(w=1920,h=1080,pattern=null){this.width=w;this.height=h;this.pattern=pattern;this.cmds=[];this.version=0;this.surface=0;this.materialized=-1;this.context=new Context(this)}
 getContext(kind){check(kind==='2d');return this.context}
 flush(){if(this.materialized!==this.version){this.surface=host('render',JSON.stringify({w:this.width,h:this.height,pattern:this.pattern,cmds:this.cmds}));this.materialized=this.version}return this.surface}
 toDataURL(mime){check(mime==='image/jpeg');return host('jpeg',this.flush())}
}
class Context {
 constructor(c){this.canvas=c;this.color=[0,0,0,255]}
 add(op,a=[]){this.canvas.cmds.push([op,a]);this.canvas.version++}
 set fillStyle(v){check(Array.isArray(v)&&v.length===4);this.color=v;this.add('color',v)}
 fillRect(...v){this.add('rect',v)}beginPath(){this.add('begin')}moveTo(...v){this.add('move',v)}lineTo(...v){this.add('line',v)}bezierCurveTo(...v){this.add('curve',v)}closePath(){this.add('close')}fill(){this.add('fill')}
 fillText(...v){this.add('text',v)}
 drawImage(source,x=0,y=0){this.add('image',[source.flush(),x,y])} // Captures immutable native version NOW.
 getImageData(x,y,w,h){check(Number.isInteger(x)&&Number.isInteger(y)&&w>0&&h>0&&x>=0&&y>=0&&x+w<=this.canvas.width&&y+h<=this.canvas.height,'bounded read');return{width:w,height:h,data:new Uint8ClampedArray(host('read',this.canvas.flush(),x,y,w,h))}}
 putImageData(im,x,y){check(x===0&&y===0&&im.width===this.canvas.width&&im.height===this.canvas.height,'full write only');this.canvas.surface=host('write',im.data,im.width,im.height);this.canvas.cmds=[['image',[this.canvas.surface,0,0]]];this.canvas.pattern=null;this.canvas.version++;this.canvas.materialized=this.canvas.version}
}
function pixels(c){return c.getContext('2d').getImageData(0,0,c.width,c.height).data}
function continuation(read=true,source=null){const job=host('begin');check(host('nested',job,7),'nested pinned');const nested=source??new Recorder(1920,1080,'pattern');const outer=new Recorder();const ctx=outer.getContext('2d');ctx.drawImage(nested);const times=[];for(let j=0;j<4;j++){const layer=new Recorder(16,16),lc=layer.context;const t=hostNow();lc.fillStyle=[35,180,90,255];lc.fillRect(0,0,16,16);times.push(hostNow()-t);ctx.drawImage(layer,j*16,0)}const surface=outer.flush();check(host('resume',job,7,surface)===surface,'single use resume');if(read){const p=pixels(outer);check(p[0]===35&&p[1]===180&&p[2]===90&&p[3]===255,'continuation pixel '+p.slice(0,4))}else outer.flush();return times}
function job(){const c=new Recorder();const x=c.getContext('2d');x.fillStyle=[23,40,57,255];x.fillRect(0,0,1920,1080);for(let j=0;j<100;j++){x.fillStyle=[j,180,90,255];x.fillRect(j*13%1900,j*7%1060,16,16)}const p=x.getImageData(0,0,1,1).data;check(p[3]===255);check(host('id')===7)}
// Same logical services, identities and callback stack, shared by V8 and A page.
const providers=[];const service={value:7,call(cb){return cb(this)}};providers.push(service);
function serviceProbe(){const old=providers.at(-1);check(old===service);const next={value:9};providers.push(next);check(providers.at(-1)===next&&old.call(x=>x)===old);providers.pop();check(providers.at(-1)===old);return old.call(x=>x.value)}
function verify(){host('clear');
 for(const kind of ['pattern','noise']){let n=0x13245678;const p=pixels(new Recorder(1920,1080,kind));for(let i=0;i<p.length;i+=4){if(kind==='noise'){n^=n<<13;n^=n>>>17;n^=n<<5}for(let k=0;k<4;k++){const want=k===3?255:kind==='noise'?(n>>>(k*8))&255:((i+k)*17+23)&255;check(p[i+k]===want,'all bytes '+kind)}}host('clear')}
 const a=new Recorder(16,16),ac=a.getContext('2d');ac.fillStyle=[35,180,90,255];ac.fillRect(0,0,16,16);const b=new Recorder(16,16);b.context.drawImage(a);ac.fillStyle=[255,0,0,255];ac.fillRect(0,0,16,16);check(pixels(b)[0]===35,'source version');check(pixels(a)[0]===255);const held=b.context;held.putImageData(held.getImageData(0,0,16,16),0,0);check(b.context===held&&pixels(b)[0]===35,'identity put read');
 const path=new Recorder(128,64);const p=path.context;p.fillStyle=[255,255,255,255];p.beginPath();p.moveTo(0,0);p.lineTo(30,0);p.bezierCurveTo(30,0,30,30,0,30);p.closePath();p.fill();p.fillText('S5c',45,25);const pp=pixels(path);check(pp[(5*128+5)*4+3]>0,'path ink');let textInk=0;for(let y=0;y<64;y++)for(let x=40;x<128;x++)textInk+=pp[(y*128+x)*4+3];check(textInk>0,'text ink');const asymmetric=new Recorder(2,2);asymmetric.context.fillStyle=[255,0,0,255];asymmetric.context.fillRect(0,0,2,1);asymmetric.context.fillStyle=[0,255,0,255];asymmetric.context.fillRect(0,1,2,1);const copy=new Recorder(2,2);copy.context.drawImage(asymmetric);const cp=pixels(copy);check(cp[0]===255&&cp[8]===0&&cp[9]===255,'drawImage orientation');check(copy.context.getImageData(0,1,1,1).data[1]===255,'small read');check(path.toDataURL('image/jpeg').startsWith('data:image/jpeg;base64,/9j/'));check(serviceProbe()===7);const job=host('begin'),surface=path.flush();check(host('nested',job,7));check(host('resume',job,7,surface)===surface);check(host('resume',job,7,surface)===0,'replay');const stale=host('begin');check(host('resume',stale,8,surface)===0,'stale');check(host('resume',999999,7,surface)===0,'unknown');host('clear');return{passed:true,bytes:8294400,versioned_source:true,held_context:true,jpeg:true,paths_text_smoke:true}}
function run(v){
 if(v.kind==='subset'){return{probe:'subset',path_text:bench(()=>{const c=new Recorder(),x=c.context;x.fillStyle=[35,180,90,255];x.fillRect(0,0,1920,1080);x.beginPath();x.moveTo(0,0);x.lineTo(100,0);x.bezierCurveTo(100,0,100,100,0,100);x.closePath();x.fill();x.fillText('S5c',150,50);check(pixels(c)[3]===255)}),put_read:bench(()=>{const c=new Recorder();const im=c.context.getImageData(0,0,1920,1080);c.context.putImageData(im,0,0);check(pixels(c)[0]===0)})}};
 if(v.kind==='verify')return{probe:'verify',...verify()};
 if(v.kind==='bench'){
  const out={probe:'bench',build:v.build};
  out.channel=bench(()=>check(host('id')===7),2000);
  out.services=bench(()=>check(serviceProbe()===7),2000);
  for(const kind of ['pattern','noise']){out['render_pixels_'+kind]=bench(()=>{const p=pixels(new Recorder(1920,1080,kind));check(p[3]===255)});out['snapshot_'+kind]=bench(()=>{const c=new Recorder(1920,1080,kind);check(c.toDataURL('image/jpeg').startsWith('data:image/jpeg;base64,'))})}
  out.raster_pixels=bench(()=>{const c=new Recorder();c.context.fillStyle=[35,180,90,255];c.context.fillRect(0,0,1920,1080);check(pixels(c)[0]===35,'raster pixel')});
  out.continuation_cold=bench(()=>continuation());
  host('clear');const source=new Recorder(1920,1080,'pattern');source.flush();const continuations=[];for(let j=0;j<100;j++){host('clear',source.surface);const t=hostNow();continuation(true,source);continuations.push(hostNow()-t)}out.continuation_warm=stats(continuations);
  out.override_path4=bench(()=>continuation(false));
  const layer=[];for(let i=0;i<100;i++){host('clear');layer.push(...continuation(false))}out.js_layer=stats(layer);
  host('clear');const c=new Recorder(1920,1080,'pattern');c.flush();const warm=[];for(let i=0;i<100;i++){const t=hostNow();check(pixels(c)[0]===23);warm.push(hostNow()-t)}out.warm_read=stats(warm);host('clear');return out;
 }
 if(v.kind==='phase'){const t=hostNow();let count=0;do{host('clear');job();count++}while(hostNow()-t<v.ms);host('clear');return{probe:'phase',phase:v.phase,count,ms:hostNow()-t,jobs_s:count*1000/(hostNow()-t)}}
 throw Error('unknown command')
}
// A shadow DOM identity model and mutation batching. No synchronous layout shim.
let nodes=new Map(),pending=[],nextNode=0;
class Node {constructor(tag){this.id=++nextNode;this.listeners={};nodes.set(this.id,this);pending.push(['create',this.id,tag])}set textContent(v){this.text=v;pending.push(['text',this.id,v])}setAttribute(k,v){pending.push(['attr',this.id,k,v])}appendChild(n){pending.push(['append',this.id,n.id]);return n}addEventListener(k,f){this.listeners[k]=f}}
const root=new Node('section'),input=new Node('input'),label=new Node('output');root.appendChild(input);root.appendChild(label);label.textContent='Ready';input.setAttribute('aria-label','S5c test input');input.addEventListener('input',e=>{input.value=e.value;label.textContent='Value: '+input.value});
function remoteEvent(v){const t=hostNow();if(v.kind==='input'){const n=nodes.get(v.id);n.listeners.input(v)}const mutations=pending;pending=[];return{seq:v.seq,kind:'batch',mutations,worker_ms:hostNow()-t}}
