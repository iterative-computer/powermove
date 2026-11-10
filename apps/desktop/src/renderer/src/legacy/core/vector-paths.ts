import { isProperty } from './content-properties';
import { textAnimatorLabel } from './text-animation';
export type PathVertex = { id:string; p:Record<string,any> };
export type VectorPath = { id:string; name:string; parent:string|null; vertices:PathVertex[]; p:Record<string,any> };
export function makeVectorPath(PM:any,name='Path'):VectorPath {
  const values={x:0,y:0,rotation:0,scaleX:100,scaleY:100,closed:false,fill:'#E8E2CF',fillEnabled:true,fillOpacity:100,stroke:'#ffffff',strokeWidth:2,strokeOpacity:100,trimStart:0,trimEnd:100,trimOffset:0,copies:1,repeatX:30,repeatY:0,repeatRotation:0};
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
/** Visual stroke geometry, separate from the bitmap's antialiasing margin.
 * Measure actual segment normals and joins; a miter limit is a cutoff, not
 * a multiplier to apply to every point in the path. */
export function pathRasterGeometry(PM:any,layer:any,time:number,scale:number,records = (layer.d.paths || []).map((path:VectorPath)=>({v:pathValues(PM,layer,path,time),matrix:groupMatrix(PM,layer,path,time)}))):any {
  let x0=Infinity,y0=Infinity,x1=-Infinity,y1=-Infinity;
  for(const rec of records)for(let i=0;i<Math.min(256,Math.max(1,Math.round(rec.v.copies)));i++){
    const v=rec.v,r=i*v.repeatRotation*Math.PI/180,c=Math.cos(r),s=Math.sin(r),m=PM.mul(rec.matrix,[c,s,-s,c,i*v.repeatX,i*v.repeatY]);
    const add=(x:number,y:number)=>{const xx=m[0]*x+m[2]*y+m[4],yy=m[1]*x+m[3]*y+m[5];x0=Math.min(x0,xx);y0=Math.min(y0,yy);x1=Math.max(x1,xx);y1=Math.max(y1,yy);};
    const paths:Array<Array<[number,number]>>=[];let points:Array<[number,number]>=[];
    const curve=(a:[number,number],b:[number,number],c:[number,number],d:[number,number])=>{
      for(let n=1;n<=64;n++){const t=n/64,u=1-t;points.push([u*u*u*a[0]+3*u*u*t*b[0]+3*u*t*t*c[0]+t*t*t*d[0],u*u*u*a[1]+3*u*u*t*b[1]+3*u*t*t*c[1]+t*t*t*d[1]]);}
    };
    tracePath({beginPath(){},moveTo(x:number,y:number){points=[[x,y]];paths.push(points);},lineTo(x:number,y:number){points.push([x,y]);},
      bezierCurveTo(x1:number,y1:number,x2:number,y2:number,x:number,y:number){curve(points.at(-1)!,[x1,y1],[x2,y2],[x,y]);},
      closePath(){if(points.length)points.push(points[0]!);}} as any,v);
    const radius=Math.max(0,Number(v.strokeWidth)||0)/2;
    const round=(p:[number,number])=>{const rx=radius*Math.hypot(m[0],m[2]),ry=radius*Math.hypot(m[1],m[3]);const x=m[0]*p[0]+m[2]*p[1]+m[4],y=m[1]*p[0]+m[3]*p[1]+m[5];x0=Math.min(x0,x-rx);x1=Math.max(x1,x+rx);y0=Math.min(y0,y-ry);y1=Math.max(y1,y+ry);};
    for(const raw of paths){
      const path=raw.filter((p,i)=>!i||Math.hypot(p[0]-raw[i-1]![0],p[1]-raw[i-1]![1])>1e-9);
      if(!path.length)continue;
      if(!radius || v.closed && v.fillEnabled)for(const p of path)add(...p);
      if(!radius)continue;
      const normals:Array<[number,number]>=[];
      for(let j=1;j<path.length;j++){
        const a=path[j-1]!,b=path[j]!,dx=b[0]-a[0],dy=b[1]-a[1],len=Math.hypot(dx,dy),normal:[number,number]=[-dy/len,dx/len];normals.push(normal);
        for(const point of [a,b])for(const side of [-1,1])add(point[0]+normal[0]*radius*side,point[1]+normal[1]*radius*side);
      }
      const closed=!!v.closed&&v.trimEnd-v.trimStart>=99.999;
      for(let j=closed?0:1;j<path.length-1;j++){
        const point=path[j]!,a=normals[(j+normals.length-1)%normals.length],b=normals[j];if(!a||!b)continue;
        const join=v.join??'miter';
        if(join==='round'){round(point);continue;}
        if(join==='miter'){
          const denominator=1+a[0]*b[0]+a[1]*b[1];
          if(denominator>1e-8){const x=(a[0]+b[0])/denominator,y=(a[1]+b[1])/denominator;
            if(Math.hypot(x,y)<=Math.max(1,Number(v.miterLimit)||10))for(const side of [-1,1])add(point[0]+x*radius*side,point[1]+y*radius*side);
          }
        }
      }
      if(!closed && normals.length)for(const [point,normal,direction,cap] of [[path[0],normals[0],-1,v.startCap??v.cap],[path.at(-1),normals.at(-1),1,v.endCap??v.cap]] as any[]){
        if(String(cap).toUpperCase()==='ROUND')round(point);
        else if(String(cap).toUpperCase()==='SQUARE')for(const side of [-1,1])add(point[0]+radius*(normal[0]*side+normal[1]*direction),point[1]+radius*(normal[1]*side-normal[0]*direction));
      }
    }
  }
  if(!Number.isFinite(x0)){x0=y0=0;x1=y1=1;}
  const visual={x0,y0,x1,y1,w:x1-x0,h:y1-y0};
  // Bitmap padding belongs only to raster storage, never pivots or selection.
  x0-=4;y0-=4;x1+=4;y1+=4;
  const w=Math.max(1,x1-x0),h=Math.max(1,y1-y0),density=Math.min(scale,8192/w,8192/h);
  return {records,x0,y0,x1,y1,w,h,density,width:Math.ceil(w*density),height:Math.ceil(h*density),visual,selection:visual};
}
export function rasterPaths(PM:any,layer:any,time:number,scale:number,geometry=pathRasterGeometry(PM,layer,time,scale)):any {
  const {records,x0,y0,x1,y1,w,h,density}=geometry;
  const cv=document.createElement('canvas');cv.width=Math.ceil(w*density);cv.height=Math.ceil(h*density);
  const ctx=cv.getContext('2d')!;ctx.scale(density,density);ctx.translate(-x0,-y0);
  for(const rec of records)for(let i=0;i<Math.min(256,Math.max(1,Math.round(rec.v.copies)));i++){
    ctx.save();ctx.transform(...rec.matrix as [number,number,number,number,number,number]);ctx.translate(i*rec.v.repeatX,i*rec.v.repeatY);ctx.rotate(i*rec.v.repeatRotation*Math.PI/180);
    tracePath(ctx,rec.v);ctx.fillStyle=rec.v.fill;ctx.strokeStyle=rec.v.stroke;ctx.lineWidth=Math.max(0,rec.v.strokeWidth);ctx.lineJoin=rec.v.join??'miter';ctx.miterLimit=rec.v.miterLimit??10;ctx.lineCap=String(rec.v.cap).toUpperCase()==='ROUND'?'round':String(rec.v.cap).toUpperCase()==='SQUARE'?'square':'butt';
    if(rec.v.closed && rec.v.fillEnabled && rec.v.trimEnd-rec.v.trimStart>=99.999){ctx.globalAlpha=Number.isFinite(Number(rec.v.fillOpacity))?PM.clamp(Number(rec.v.fillOpacity),0,100)/100:1;ctx.fill();}
    if(rec.v.strokeWidth>0){ctx.globalAlpha=Number.isFinite(Number(rec.v.strokeOpacity))?PM.clamp(Number(rec.v.strokeOpacity),0,100)/100:1;ctx.stroke();}ctx.restore();
  }
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
  for(const rec of records)for(let i=0;i<Math.min(256,Math.max(1,Math.round(rec.v.copies)));i++){
    ctx.save();
    ctx.transform(...world);
    ctx.transform(...rec.matrix as [number,number,number,number,number,number]);
    ctx.translate(i*rec.v.repeatX,i*rec.v.repeatY);
    ctx.rotate(i*rec.v.repeatRotation*Math.PI/180);
    tracePath(ctx,rec.v);
    ctx.fillStyle=rec.v.fill;ctx.strokeStyle=rec.v.stroke;ctx.lineWidth=Math.max(0,rec.v.strokeWidth);ctx.lineJoin=rec.v.join??'miter';ctx.miterLimit=rec.v.miterLimit??10;ctx.lineCap=String(rec.v.cap).toUpperCase()==='ROUND'?'round':String(rec.v.cap).toUpperCase()==='SQUARE'?'square':'butt';
    if(rec.v.closed && rec.v.fillEnabled && rec.v.trimEnd-rec.v.trimStart>=99.999){
      ctx.globalAlpha=Number.isFinite(Number(rec.v.fillOpacity))?PM.clamp(Number(rec.v.fillOpacity),0,100)/100:1;
      ctx.fill();
    }
    if(rec.v.strokeWidth>0){
      ctx.globalAlpha=Number.isFinite(Number(rec.v.strokeOpacity))?PM.clamp(Number(rec.v.strokeOpacity),0,100)/100:1;
      ctx.stroke();
    }
    ctx.restore();
  }
  return {cv,signature};
}

export function pathTargets(layer:any) {
  return [...(layer?.d?.paths||[]).map((path:any)=>({path,prefix:`g.${path.id}`})),...(layer?.masks||[]).filter((m:any)=>m.path).map((m:any)=>({path:m.path,prefix:`mp.${m.id}`}))];
}
