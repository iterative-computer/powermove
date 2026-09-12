import { makeVectorPath, makeVertex, pathValues, groupMatrix, tracePath, pathTargets } from 'powermove';
function targetTransform(PM:any,layer:any,target:any){
  if(target.prefix.startsWith('g.'))return groupMatrix(PM,layer,target.path,PM.time);
  const mask=layer.masks.find((m:any)=>m.path===target.path),v=pathValues(PM,layer,target.path,PM.time,target.prefix);
  const read=(key:string)=>Number(PM.evP(layer,mask.p[key],PM.time,`m.${mask.id}.${key}`))||0;
  const r=read('rotation')*Math.PI/180,c=Math.cos(r),s=Math.sin(r),pr=v.rotation*Math.PI/180,pc=Math.cos(pr),ps=Math.sin(pr);
  return PM.mul([c,s,-s,c,read('x'),read('y')],[pc*v.scaleX/100,ps*v.scaleX/100,-ps*v.scaleY/100,pc*v.scaleY/100,v.x,v.y]);
}
function inverse(m:[number,number,number,number,number,number],p:any){const d=m[0]*m[3]-m[1]*m[2];return Math.abs(d)<1e-9?null:{x:(m[3]*(p.x-m[4])-m[2]*(p.y-m[5]))/d,y:(-m[1]*(p.x-m[4])+m[0]*(p.y-m[5]))/d};}
export function drawEditablePaths(PM:any,c:CanvasRenderingContext2D,scale:number) {
  if(PM.tool!=='pen')return;
  for(const layer of PM.selLayers())for(const target of pathTargets(layer)){
    const m=PM.mul(PM.worldMatrix(layer,PM.time),targetTransform(PM,layer,target));
    const v=pathValues(PM,layer,target.path,PM.time,target.prefix);
    const point=(x:number,y:number)=>[m[0]*x+m[2]*y+m[4],m[1]*x+m[3]*y+m[5]];
    c.save();c.strokeStyle='#70b7ff';c.fillStyle='#152c46';c.lineWidth=1/scale;
    c.save();c.transform(...m as [number,number,number,number,number,number]);tracePath(c,v);c.restore();c.stroke();
    for(const vertex of v.vertices){const p=point(vertex.x,vertex.y);for(const side of ['in','out']){const h=point(vertex.x+vertex[side+'X'],vertex.y+vertex[side+'Y']);c.beginPath();c.moveTo(p[0],p[1]);c.lineTo(h[0],h[1]);c.stroke();c.beginPath();c.arc(h[0],h[1],3/scale,0,Math.PI*2);c.fill();c.stroke();}c.fillRect(p[0]-4/scale,p[1]-4/scale,8/scale,8/scale);c.strokeRect(p[0]-4/scale,p[1]-4/scale,8/scale,8/scale);}
    c.restore();
  }
}
export function startPathEdit(PM:any,event:any,pointer:(e:any)=>any,beginDrag:any,scale:number) {
  let layer=PM.firstSel();if(layer?.lock)return;
  if(!layer || !['shape','text','solid','image','video','precomp','shader'].includes(layer.type)){
    const result=PM.Edit.apply({type:'add_layer',layerType:'shape',name:'Path',content:{paths:[]},properties:{'position.x':0,'position.y':0},select:true},{label:'New path layer',origin:'canvas'});
    layer=result?.data?.results?.[0]?.data?.layer;if(!layer)return;
  }
  const world=pointer(event), targets=pathTargets(layer);let hit:any=null;
  for(const target of targets){const matrix=PM.mul(PM.worldMatrix(layer,PM.time),targetTransform(PM,layer,target)),p=inverse(matrix,world);if(!p)continue;
    const v=pathValues(PM,layer,target.path,PM.time,target.prefix),radius=8/scale/Math.max(.01,Math.hypot(matrix[0],matrix[1]));
    v.vertices.forEach((vertex:any,i:number)=>{for(const side of ['in','out','point']){const x=vertex.x+(side==='point'?0:vertex[side+'X']),y=vertex.y+(side==='point'?0:vertex[side+'Y']);if(Math.hypot(p.x-x,p.y-y)<radius)hit={...target,matrix,p,vertex:target.path.vertices[i],values:vertex,side,index:i};}});
  }
  if(hit && (event.detail>1 || (hit.index===0 && hit.side==='point' && hit.path.vertices.length>=3 && PM.activePath===hit.path.id && !event.altKey && !event.metaKey && !event.ctrlKey))){PM.Edit.mutate('Close path',()=>{hit.path.p.closed.v=true;PM.activePath=null;},{origin:'canvas'});PM.invalidate();return;}
  if(hit && (event.metaKey||event.ctrlKey)){PM.Edit.mutate('Delete path vertex',()=>hit.path.vertices.splice(hit.index,1),{origin:'canvas'});PM.invalidate();return;}
  if(!hit){
    let target=targets.find((x:any)=>x.path.id===PM.activePath && !PM.evP(layer,x.path.p.closed,PM.time,x.prefix+'.closed'));
    PM.hist.begin('Draw path vertex');
    if(!target){const path=makeVectorPath(PM,layer.type==='shape'?'Path':'Mask path');if(layer.type==='shape'){(layer.d.paths||=[]).push(path);target={path,prefix:`g.${path.id}`};}else{const mask=PM.mkMask('rect');mask.path=path;mask.p.feather.v=0;(layer.masks||=[]).push(mask);target={path,prefix:`mp.${mask.id}`};}PM.activePath=path.id;}
    const matrix=PM.mul(PM.worldMatrix(layer,PM.time),targetTransform(PM,layer,target!)),p=inverse(matrix,world);if(!p){PM.hist.cancel();return;}
    const vertex=makeVertex(PM,p.x,p.y);target!.path.vertices.push(vertex);hit={...target,matrix,p,vertex,values:{x:p.x,y:p.y,inX:0,inY:0,outX:0,outY:0},side:'new'};
  }else PM.hist.begin('Edit path vertex');
  PM.touch();PM.invalidate();
  beginDrag(event,{cursor:'crosshair',move:(_dx:any,_dy:any,e:any)=>{
    const p=inverse(hit.matrix,pointer(e));if(!p)return;const dx=p.x-hit.p.x,dy=p.y-hit.p.y,v=hit.vertex.p;
    const write=(key:string,value:number)=>{if(v[key].kf.length)PM.setKeyOn(v[key],PM.time-layer.from,value,'linear',PM.proj.fps);else v[key].v=value;};
    if(hit.side==='point'&&!e.altKey){write('x',hit.values.x+dx);write('y',hit.values.y+dy);}else{const side=hit.side==='in'?'in':'out';write(side+'X',hit.values[side+'X']+dx);write(side+'Y',hit.values[side+'Y']+dy);if(!e.altKey){const other=side==='in'?'out':'in';write(other+'X',-hit.values[side+'X']-dx);write(other+'Y',-hit.values[side+'Y']-dy);}}
    PM.touch();PM.invalidate('render');
  },up:()=>{PM.hist.commit();PM.invalidate();PM.Inspector?.refresh?.();},cancel:()=>{PM.hist.cancel();PM.invalidate();}});
}
