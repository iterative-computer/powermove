import type { AffineMatrix, Channel, Layer, PowermoveAPI } from 'powermove';
import { tracePath, pathTargets } from 'powermove';

type PathVertex = { id: string; p: Record<string, Channel> };
type VectorPath = { id: string; name: string; parent: string | null; vertices: PathVertex[]; p: Record<string, Channel> };
export interface PathEditingState { activePath: string | null; requestOverlay?(): void }

function makeVectorPath(api: PowermoveAPI, name = 'Path'): VectorPath {
  const values = { x:0,y:0,rotation:0,scaleX:100,scaleY:100,closed:false,fill:'#E8E2CF',fillEnabled:true,fillOpacity:100,stroke:'#ffffff',strokeWidth:2,strokeOpacity:100,trimStart:0,trimEnd:100,trimOffset:0,copies:1,repeatX:30,repeatY:0,repeatRotation:0 };
  return { id:api.util.uid('path'),name,parent:null,vertices:[],p:Object.fromEntries(Object.entries(values).map(([key,value])=>[key,api.model.P(value)])) };
}

function makeVertex(api: PowermoveAPI, x: number, y: number): PathVertex {
  return { id:api.util.uid('v'),p:Object.fromEntries(Object.entries({x,y,inX:0,inY:0,outX:0,outY:0}).map(([key,value])=>[key,api.model.P(value)])) };
}

function pathValues(api: PowermoveAPI, layer: Layer, path: VectorPath, time: number, prefix=`g.${path.id}`): any {
  const values=Object.fromEntries(Object.entries(path.p).map(([key,prop])=>[key,api.anim.evP(layer,prop,time,`${prefix}.${key}`)]));
  const vertices=path.vertices.map(vertex=>Object.fromEntries(Object.entries(vertex.p).map(([key,prop])=>[key,Number(api.anim.evP(layer,prop,time,`${prefix}.v.${vertex.id}.${key}`))||0])));
  return {...values,vertices};
}

function groupMatrix(api: PowermoveAPI, layer: Layer, path: VectorPath, time: number, seen=new Set<string>()): AffineMatrix {
  if(seen.has(path.id))return [1,0,0,1,0,0];seen.add(path.id);
  const value=pathValues(api,layer,path,time),r=value.rotation*Math.PI/180,c=Math.cos(r),s=Math.sin(r);
  const local=[c*value.scaleX/100,s*value.scaleX/100,-s*value.scaleY/100,c*value.scaleY/100,value.x,value.y] as const;
  const parent=((layer.d as any).paths as VectorPath[]|undefined)?.find(candidate=>candidate.id===path.parent);
  return parent?api.anim.mul(groupMatrix(api,layer,parent,time,seen),local):[...local];
}

function targetTransform(api: PowermoveAPI,layer:any,target:any,time:number){
  if(target.prefix.startsWith('g.'))return groupMatrix(api,layer,target.path,time);
  const mask=layer.masks.find((candidate:any)=>candidate.path===target.path),value=pathValues(api,layer,target.path,time,target.prefix);
  const read=(key:string)=>Number(api.anim.evP(layer,mask.p[key],time,`m.${mask.id}.${key}`))||0;
  const r=read('rotation')*Math.PI/180,c=Math.cos(r),s=Math.sin(r),pr=value.rotation*Math.PI/180,pc=Math.cos(pr),ps=Math.sin(pr);
  return api.anim.mul([c,s,-s,c,read('x'),read('y')],[pc*value.scaleX/100,ps*value.scaleX/100,-ps*value.scaleY/100,pc*value.scaleY/100,value.x,value.y]);
}

function inverse(m:[number,number,number,number,number,number],p:any){const d=m[0]*m[3]-m[1]*m[2];return Math.abs(d)<1e-9?null:{x:(m[3]*(p.x-m[4])-m[2]*(p.y-m[5]))/d,y:(-m[1]*(p.x-m[4])+m[0]*(p.y-m[5]))/d};}

export function drawEditablePaths(api:PowermoveAPI,c:CanvasRenderingContext2D,scale:number) {
  if(api.services.get<{tool:string}>('tool')?.tool!=='pen')return;
  const time=api.transport.time();
  for(const layer of api.selection.layers().map(id=>api.model.layer(id)).filter((value):value is Layer=>!!value))for(const target of pathTargets(layer)){
    const m=api.anim.mul(api.anim.worldMatrix(layer,time),targetTransform(api,layer,target,time));
    const value=pathValues(api,layer,target.path,time,target.prefix);
    const point=(x:number,y:number):[number,number]=>[m[0]*x+m[2]*y+m[4],m[1]*x+m[3]*y+m[5]];
    c.save();c.strokeStyle='#70b7ff';c.fillStyle='#152c46';c.lineWidth=1/scale;
    c.save();c.transform(...m);tracePath(c,value);c.restore();c.stroke();
    for(const vertex of value.vertices){const p=point(vertex.x,vertex.y);for(const side of ['in','out']){const h=point(vertex.x+vertex[side+'X'],vertex.y+vertex[side+'Y']);c.beginPath();c.moveTo(p[0],p[1]);c.lineTo(h[0],h[1]);c.stroke();c.beginPath();c.arc(h[0],h[1],3/scale,0,Math.PI*2);c.fill();c.stroke();}c.fillRect(p[0]-4/scale,p[1]-4/scale,8/scale,8/scale);c.strokeRect(p[0]-4/scale,p[1]-4/scale,8/scale,8/scale);}
    c.restore();
  }
}

export function startPathEdit(api:PowermoveAPI,state:PathEditingState,event:any,pointer:(e:any)=>any,beginDrag:any,scale:number) {
  let layer=api.selection.first();if(layer?.lock)return;
  if(!layer || !['shape','text','solid','image','video','precomp','shader'].includes(layer.type)){
    const result=api.edit.apply({type:'add_layer',layerType:'shape',name:'Path',content:{paths:[]},properties:{'position.x':0,'position.y':0},select:true},{label:'New path layer',origin:'canvas'});
    layer=result.ok?(result.data as {results?:Array<{data?:{layer?:Layer}}> }|undefined)?.results?.[0]?.data?.layer??null:null;if(!layer)return;
  }
  const time=api.transport.time(),world=pointer(event),targets=pathTargets(layer);let hit:any=null;
  for(const target of targets){const matrix=api.anim.mul(api.anim.worldMatrix(layer,time),targetTransform(api,layer,target,time)),p=inverse(matrix,world);if(!p)continue;
    const value=pathValues(api,layer,target.path,time,target.prefix),radius=8/scale/Math.max(.01,Math.hypot(matrix[0],matrix[1]));
    value.vertices.forEach((vertex:any,index:number)=>{for(const side of ['in','out','point']){const x=vertex.x+(side==='point'?0:vertex[side+'X']),y=vertex.y+(side==='point'?0:vertex[side+'Y']);if(Math.hypot(p.x-x,p.y-y)<radius)hit={...target,matrix,p,vertex:target.path.vertices[index],values:vertex,side,index};}});
  }
  if(hit && (event.detail>1 || (hit.index===0 && hit.side==='point' && hit.path.vertices.length>=3 && state.activePath===hit.path.id && !event.altKey && !event.metaKey && !event.ctrlKey))){api.edit.mutate('Close path',()=>{hit.path.p.closed.v=true;state.activePath=null;},{origin:'canvas'});api.transport.invalidate();state.requestOverlay?.();return;}
  if(hit && (event.metaKey||event.ctrlKey)){api.edit.mutate('Delete path vertex',()=>hit.path.vertices.splice(hit.index,1),{origin:'canvas'});api.transport.invalidate();state.requestOverlay?.();return;}
  if(!hit){
    let target=targets.find((candidate:any)=>candidate.path.id===state.activePath && !api.anim.evP(layer!,candidate.path.p.closed,time,candidate.prefix+'.closed'));
    api.history.begin('Draw path vertex');
    if(!target){const path=makeVectorPath(api,layer.type==='shape'?'Path':'Mask path');if(layer.type==='shape'){const content=layer.d as any;const paths=(content.paths??=[]) as VectorPath[];paths.push(path);target={path,prefix:`g.${path.id}`};}else{const mask=api.model.mkMask('rect');mask.path=path as any;mask.p.feather!.v=0;(layer.masks||=[]).push(mask);target={path,prefix:`mp.${mask.id}`};}state.activePath=path.id;}
    const matrix=api.anim.mul(api.anim.worldMatrix(layer,time),targetTransform(api,layer,target,time)),p=inverse(matrix,world);if(!p){api.history.cancel();return;}
    const vertex=makeVertex(api,p.x,p.y);target.path.vertices.push(vertex);hit={...target,matrix,p,vertex,values:{x:p.x,y:p.y,inX:0,inY:0,outX:0,outY:0},side:'new'};
  }else api.history.begin('Edit path vertex');
  api.anim.touch();api.transport.invalidate();state.requestOverlay?.();
  beginDrag(event,{cursor:'crosshair',move:(_dx:any,_dy:any,e:any)=>{
    const p=inverse(hit.matrix,pointer(e));if(!p)return;const dx=p.x-hit.p.x,dy=p.y-hit.p.y,value=hit.vertex.p;
    const write=(key:string,next:number)=>{if(value[key].kf.length)api.anim.setKeyOn(value[key],time-layer!.from,next,'linear',api.project.get().fps);else value[key].v=next;};
    if(hit.side==='point'&&!e.altKey){write('x',hit.values.x+dx);write('y',hit.values.y+dy);}else{const side=hit.side==='in'?'in':'out';write(side+'X',hit.values[side+'X']+dx);write(side+'Y',hit.values[side+'Y']+dy);if(!e.altKey){const other=side==='in'?'out':'in';write(other+'X',-hit.values[side+'X']-dx);write(other+'Y',-hit.values[side+'Y']-dy);}}
    api.anim.touch();api.transport.invalidate('render');state.requestOverlay?.();
  },up:()=>{api.history.commit();api.transport.invalidate();state.requestOverlay?.();api.services.get<{refresh():void}>('inspector')?.refresh();},cancel:()=>{api.history.cancel();api.transport.invalidate();state.requestOverlay?.();}});
}
