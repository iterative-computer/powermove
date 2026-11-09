import { isProperty } from './content-properties';
import { textAnimatorLabel } from './text-animation';
import { paintShape } from '../gl/drawing';
export type PathVertex = { id:string; p:Record<string,any> };
export type VectorPath = { id:string; name:string; parent:string|null; vertices:PathVertex[]; p:Record<string,any> };
export function makeVectorPath(PM:any,name='Path'):VectorPath {
  const values={x:0,y:0,rotation:0,scaleX:100,scaleY:100,closed:false,fill:'#E8E2CF',fillEnabled:true,fillOpacity:100,stroke:'#ffffff',strokeWidth:2,strokeOpacity:100,cap:'NONE',join:'miter',miterLimit:4,trimStart:0,trimEnd:100,trimOffset:0,copies:1,repeatX:30,repeatY:0,repeatRotation:0};
  return {id:PM.uid('path'),name,parent:null,vertices:[],p:Object.fromEntries(Object.entries(values).map(([k,v])=>[k,PM.P(v)]))};
}
export function makeVertex(PM:any,x:number,y:number):PathVertex {
  return {id:PM.uid('v'),p:Object.fromEntries(Object.entries({x,y,inX:0,inY:0,outX:0,outY:0}).map(([k,v])=>[k,PM.P(v)]))};
}
export function structuredProperties(layer:any):any[] {
  const out:any[]=[];
  const collect=(path:any,prefix:string)=> {
    for(const [k,prop] of Object.entries(path.p || {})) if(isProperty(prop))out.push({key:`${prefix}.${k}`,prop,label:k,group:path.name || 'Path'});
    for(const v of path.vertices || []) for(const [k,prop] of Object.entries(v.p || {})) if(isProperty(prop))out.push({key:`${prefix}.v.${v.id}.${k}`,prop,label:`Vertex ${path.vertices.indexOf(v)+1} ${k}`,group:path.name || 'Path'});
  };
  for(const path of layer.d?.paths || [])collect(path,`g.${path.id}`);
  for(const mask of layer.masks || [])if(mask.path)collect(mask.path,`mp.${mask.id}`);
  for(const animator of layer.d?.animators || []) for(const [key,prop] of Object.entries(animator.p || {}))if(isProperty(prop))out.push({key:`ta.${animator.id}.${key}`,prop,label:textAnimatorLabel(key),group:animator.name || 'Text animator'});
  for(const range of layer.d?.styles || [])for(const [key,prop] of Object.entries(range.p || {}))if(isProperty(prop))out.push({key:`ts.${range.id}.${key}`,prop,label:key,group:'Text range'});
  return out;
}
export function pathValues(PM:any,layer:any,path:VectorPath,time:number,prefix=`g.${path.id}`):any {
  const read=(p:any,key:string)=>isProperty(p)?PM.evP(layer,p,time,key):p;
  const values=Object.fromEntries(Object.entries(path.p).map(([k,p])=>[k,read(p,`${prefix}.${k}`)]));
  const vertices=path.vertices.map(v=>Object.fromEntries(Object.entries(v.p).map(([k,p])=>[k,Number(read(p,`${prefix}.v.${v.id}.${k}`)) || 0])));
  return {...values,vertices};
}
export function pathPoints(values:any):Array<[number,number]> {
  const vs=values.vertices, points:Array<[number,number]>=[];
  if(!vs.length)return points;
  points.push([vs[0].x,vs[0].y]);
  for(let i=0;i<vs.length-(values.closed?0:1);i++) {
    const a=vs[i],b=vs[(i+1)%vs.length];
    for(let j=1;j<=32;j++) { const t=j/32,u=1-t;
      points.push([u*u*u*a.x+3*u*u*t*(a.x+a.outX)+3*u*t*t*(b.x+b.inX)+t*t*t*b.x,u*u*u*a.y+3*u*u*t*(a.y+a.outY)+3*u*t*t*(b.y+b.inY)+t*t*t*b.y]);
    }
  }
  return points;
}
export function tracePath(context:CanvasRenderingContext2D,values:any):void {
  context.beginPath();
  const start=Math.max(0,Math.min(100,Number(values.trimStart)||0))/100;
  const end=Math.max(start,Math.min(100,Number(values.trimEnd)||0)/100),span=end-start;
  const vs=values.vertices;
  if(!vs.length||span<=0)return;
  if(span>=.999999){
    context.moveTo(vs[0].x,vs[0].y);
    for(let i=0;i<vs.length-(values.closed?0:1);i++){const a=vs[i],b=vs[(i+1)%vs.length];context.bezierCurveTo(a.x+a.outX,a.y+a.outY,b.x+b.inX,b.y+b.inY,b.x,b.y);}
    if(values.closed)context.closePath();return;
  }
  const points=pathPoints(values),lengths=[0];
  for(let i=1;i<points.length;i++)lengths.push(lengths[i-1]!+Math.hypot(points[i]![0]-points[i-1]![0],points[i]![1]-points[i-1]![1]));
  const total=lengths.at(-1)||0;if(!total)return;
  const offset=(((Number(values.trimOffset)||0)/360)%1+1)%1,lo=(start+offset)%1,hi=lo+span;
  for(const [from,to] of hi<=1?[[lo,hi]]:[[lo,1],[0,hi-1]]){
    let drawing=false;
    for(let i=1;i<points.length;i++){
      const a=points[i-1]!,b=points[i]!,l0=lengths[i-1]!,l1=lengths[i]!,low=Math.max(l0,from!*total),high=Math.min(l1,to!*total);
      if(high<=low||l1<=l0)continue;
      const point=(v:number)=>[a[0]+(b[0]-a[0])*(v-l0)/(l1-l0),a[1]+(b[1]-a[1])*(v-l0)/(l1-l0)];
      const p=point(low),q=point(high);if(!drawing)context.moveTo(p[0]!,p[1]!);context.lineTo(q[0]!,q[1]!);drawing=true;
    }
  }
}

export function groupMatrix(PM:any,layer:any,path:VectorPath,time:number,seen=new Set<string>()):number[] {
  if(seen.has(path.id))return [1,0,0,1,0,0];seen.add(path.id);
  // A transform does not depend on vertices, fills, or strokes. Imported
  // artwork can have hundreds of paths; avoid evaluating them a second time.
  const v=Object.fromEntries(['x','y','rotation','scaleX','scaleY'].map(key=>{
    const value=path.p[key];return [key,isProperty(value)?PM.evP(layer,value,time,`g.${path.id}.${key}`):value];
  })),r=v.rotation*Math.PI/180,c=Math.cos(r),s=Math.sin(r);
  const local=[c*v.scaleX/100,s*v.scaleX/100,-s*v.scaleY/100,c*v.scaleY/100,v.x,v.y];
  const parent=layer.d.paths?.find((p:VectorPath)=>p.id===path.parent);
  return parent?PM.mul(groupMatrix(PM,layer,parent,time,seen),local):local;
}
export function pathRasterGeometry(PM:any,layer:any,time:number,scale:number,records = (layer.d.paths || []).map((path:VectorPath)=>({v:pathValues(PM,layer,path,time),matrix:groupMatrix(PM,layer,path,time)}))):any {
  let x0=Infinity,y0=Infinity,x1=-Infinity,y1=-Infinity;
  let sx0=Infinity,sy0=Infinity,sx1=-Infinity,sy1=-Infinity;
  for(const rec of records)for(let i=0;i<Math.min(256,Math.max(1,Math.round(rec.v.copies)));i++){
    const r=i*rec.v.repeatRotation*Math.PI/180,c=Math.cos(r),s=Math.sin(r),m=PM.mul(rec.matrix,[c,s,-s,c,i*rec.v.repeatX,i*rec.v.repeatY]);
    const reach=Math.max(1,rec.v.join==='miter'?Math.min(100,Number(rec.v.miterLimit)||4):1,[rec.v.cap,rec.v.startCap,rec.v.endCap].some(cap=>/ARROW|TRIANGLE|DIAMOND|CIRCLE/.test(cap??''))?12:1);
    const stroke=rec.v.strokeWidth/2*Math.max(Math.hypot(m[0],m[1]),Math.hypot(m[2],m[3]));
    const visualPad=stroke*reach+1;
    // Canvas defaults to miter even for legacy paths without a join channel.
    // Keep that extra raster room separate from their selection geometry.
    const rasterPad=stroke*Math.max(reach,rec.v.join==null?4:1)+4;
    for(const p of pathPoints(rec.v)){
      const x=m[0]*p[0]+m[2]*p[1]+m[4],y=m[1]*p[0]+m[3]*p[1]+m[5];
      sx0=Math.min(sx0,x-visualPad);sy0=Math.min(sy0,y-visualPad);sx1=Math.max(sx1,x+visualPad);sy1=Math.max(sy1,y+visualPad);
      x0=Math.min(x0,x-rasterPad);y0=Math.min(y0,y-rasterPad);x1=Math.max(x1,x+rasterPad);y1=Math.max(y1,y+rasterPad);
    }
  }
  if(!Number.isFinite(x0)){x0=y0=sx0=sy0=0;x1=y1=sx1=sy1=1;}
  const w=Math.max(1,x1-x0),h=Math.max(1,y1-y0),density=Math.min(scale,8192/w,8192/h);
  return {records,x0,y0,x1,y1,w,h,density,width:Math.ceil(w*density),height:Math.ceil(h*density),selection:{x0:sx0,y0:sy0,x1:sx1,y1:sy1,w:sx1-sx0,h:sy1-sy0}};
}
export function rasterPaths(PM:any,layer:any,time:number,scale:number,geometry=pathRasterGeometry(PM,layer,time,scale)):any {
  const {records,x0,y0,x1,y1,w,h,density}=geometry;
  const cv=document.createElement('canvas');cv.width=Math.ceil(w*density);cv.height=Math.ceil(h*density);
  const ctx=cv.getContext('2d')!;ctx.scale(density,density);ctx.translate(-x0,-y0);
  paintVectorPaths(PM,ctx,records);
  return {cv,w,h,anchorX:-x0,anchorY:-y0,selection:geometry.selection};
}

/** Rasterize editable paths after their layer transform has been applied.
 *
 * The ordinary path raster is intentionally layer-local so it can be reused at
 * many transforms. At extreme preview zooms that local bitmap eventually hits
 * the texture-size ceiling and its pixels become visible. Full preview uses
 * this viewport-space variant instead: only the visible composition crop is
 * painted, at the exact backing resolution the viewer presents.
 */
export function rasterPathsToViewport(
  PM:any, layer:any, time:number, width:number, height:number,
  world:[number,number,number,number,number,number], previous?:any,
):any {
  const paths=(layer.d?.paths || []) as VectorPath[];
  const records=paths.map(path=>({v:pathValues(PM,layer,path,time),matrix:groupMatrix(PM,layer,path,time)}));
  const signature=JSON.stringify([width,height,world,records]);
  if(previous?.signature===signature && previous.cv)return previous;

  const cv=document.createElement('canvas');
  cv.width=Math.max(1,Math.round(width));cv.height=Math.max(1,Math.round(height));
  const ctx=cv.getContext('2d')!;
  ctx.transform(...world);paintVectorPaths(PM,ctx,records);
  return {cv,signature};
}

/** Keep contours from one SVG element in the same fill operation. Independent
 * elements retain their paint order, while strokes retain each local basis. */
export function paintVectorPaths(PM:any,ctx:CanvasRenderingContext2D,records:any[]):void {
  const groups=new Map<string,{fill:Path2D;v:any;strokes:Array<{path:Path2D;matrix:number[];v:any;ends:any[]}>}>();
  const alpha=(value:any)=>Number.isFinite(Number(value))?PM.clamp(Number(value),0,100)/100:1;
  records.forEach((rec,recordIndex)=>{
    const v=rec.v;
    for(let copy=0;copy<Math.min(256,Math.max(1,Math.round(v.copies)));copy++){
      const angle=copy*v.repeatRotation*Math.PI/180,c=Math.cos(angle),s=Math.sin(angle),matrix=PM.mul(rec.matrix,[c,s,-s,c,copy*v.repeatX,copy*v.repeatY]);
      const path=new Path2D(),ends:any[]=[];let current:any,start:any;
      const finish=()=>{if(start&&current){ends.push({x:start.x,y:start.y,angle:start.angle??0,cap:v.startCap??v.cap??'NONE'},{x:current.x,y:current.y,angle:current.angle??0,cap:v.endCap??v.cap??'NONE'});}start=null;};
      const direction=(...vectors:number[][])=>{const vector=vectors.find(([x,y])=>Math.hypot(x!,y!)>1e-8) || [1,0];return Math.atan2(vector[1]!,vector[0]!);};
      tracePath({
        beginPath(){},
        moveTo(x:number,y:number){finish();path.moveTo(x,y);start={x,y};current={x,y};},
        lineTo(x:number,y:number){const angle=direction([x-current.x,y-current.y]);if(start?.angle===undefined)start.angle=angle+Math.PI;current={x,y,angle};path.lineTo(x,y);},
        bezierCurveTo(x1:number,y1:number,x2:number,y2:number,x:number,y:number){
          if(start?.angle===undefined)start.angle=direction([x1-current.x,y1-current.y],[x2-current.x,y2-current.y],[x-current.x,y-current.y])+Math.PI;
          current={x,y,angle:direction([x-x2,y-y2],[x-x1,y-y1],[x-current.x,y-current.y])};path.bezierCurveTo(x1,y1,x2,y2,x,y);
        },
        closePath(){path.closePath();start=null;}
      } as any,v);finish();
      const key=JSON.stringify([v.fillGroup??`path-${recordIndex}`,copy,v.fill,v.fillOpacity,v.fillRule]);
      let group=groups.get(key);if(!group){group={fill:new Path2D(),v,strokes:[]};groups.set(key,group);}
      if((v.closed||v.fillOpen)&&v.fillEnabled&&v.trimEnd-v.trimStart>=99.999)group.fill.addPath(path,new DOMMatrix(matrix));
      if(v.strokeWidth>0)group.strokes.push({path,matrix,v,ends});
    }
  });
  for(const group of groups.values()){
    ctx.save();ctx.fillStyle=group.v.fill;ctx.globalAlpha*=alpha(group.v.fillOpacity);ctx.fill(group.fill,group.v.fillRule==='evenodd'?'evenodd':'nonzero');ctx.restore();
    for(const {path,matrix,v,ends} of group.strokes){
      ctx.save();ctx.transform(...matrix as [number,number,number,number,number,number]);ctx.globalAlpha*=alpha(v.strokeOpacity);
      const cap=String(v.cap??'NONE').toUpperCase(),start=String(v.startCap??cap).toUpperCase(),end=String(v.endCap??cap).toUpperCase();
      if(['NONE','BUTT','ROUND','SQUARE'].includes(start)&&start===end){
        ctx.strokeStyle=v.stroke;ctx.lineWidth=v.strokeWidth;ctx.lineCap=start==='ROUND'?'round':start==='SQUARE'?'square':'butt';ctx.lineJoin=v.join??'miter';ctx.miterLimit=v.miterLimit??4;ctx.stroke(path);
      }else{
        const points=pathPoints(v),reach=v.strokeWidth*Math.max(6,v.miterLimit??4)+4,xs=points.map(p=>p[0]),ys=points.map(p=>p[1]);
        const x=Math.min(...xs)-reach,y=Math.min(...ys)-reach,w=Math.max(...xs)-x+reach,h=Math.max(...ys)-y+reach;
        paintShape(ctx,path,{color:'transparent',stroke:v.strokeWidth,strokeColor:v.stroke,strokeStyle:{cap:'NONE',join:v.join??'miter',miterLimit:v.miterLimit??4}},{x,y,w,h},ends);
      }
      ctx.restore();
    }
  }
}

export function pathTargets(layer:any) {
  return [...(layer?.d?.paths||[]).map((path:any)=>({path,prefix:`g.${path.id}`})),...(layer?.masks||[]).filter((m:any)=>m.path).map((m:any)=>({path:m.path,prefix:`mp.${m.id}`}))];
}
