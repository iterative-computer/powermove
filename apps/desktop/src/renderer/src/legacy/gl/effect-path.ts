import {pathValues,pathPoints,groupMatrix} from '../core/vector-paths';
import {referencedPath,EffectInputError} from '../../core/effect-inputs';
type Point=[number,number];
type Curve={points:Point[];hull:Point[]};
const clamp=(n:number,a:number,b:number)=>Math.max(a,Math.min(b,n));
const transform=(m:number[],p:Point):Point=>{const x=m[0]!*p[0]+m[2]!*p[1]+m[4]!,y=m[1]!*p[0]+m[3]!*p[1]+m[5]!;if(!Number.isFinite(x+y))throw new EffectInputError('Non-finite stroke geometry');return[x,y];};
export function trimPoints(points:Point[],start:number,end:number):Point[]{
 const lengths=[0];for(let i=1;i<points.length;i++)lengths.push(lengths[i-1]!+Math.hypot(points[i]![0]-points[i-1]![0],points[i]![1]-points[i-1]![1]));
 const total=lengths.at(-1)??0,lo=clamp(start,0,1)*total,hi=clamp(end,0,1)*total,out:Point[]=[];if(hi<=lo)return out;
 for(let i=1;i<points.length;i++){const a=points[i-1]!,b=points[i]!,l=lengths[i-1]!,d=lengths[i]!-l,from=Math.max(lo,l),to=Math.min(hi,l+d);if(d<=0||to<=from)continue;const at=(x:number):Point=>[a[0]+(b[0]-a[0])*(x-l)/d,a[1]+(b[1]-a[1])*(x-l)/d];if(!out.length)out.push(at(from));out.push(at(to));}return out;
}
export function brushGeometry(PM:any,provider:any,ref:any,T:number,allMasks:boolean):Curve[]{
 const selected=referencedPath(provider,ref.pathId);if(!selected)throw new EffectInputError('Missing stroke path');
 const items=allMasks&&selected.mask?(provider.masks??[]).map((mask:any)=>({mask,path:mask.path})):[selected],out:Curve[]=[];let count=0;
 for(const item of items){const mask=item.mask,v=item.path?pathValues(PM,provider,item.path,T):null;
  if(v?.vertices.length>2048)throw new EffectInputError('Stroke geometry exceeds the vertex budget');
  const ev=(key:string)=>Number(PM.evP(provider,mask.p[key],T,`m.${mask.id}.${key}`))||0;
  const affine=(x:number,y:number,r:number,sx=1,sy=1)=>{const a=r*Math.PI/180;return[Math.cos(a)*sx,Math.sin(a)*sx,-Math.sin(a)*sy,Math.cos(a)*sy,x,y];};
  let matrix=mask?affine(ev('x'),ev('y'),ev('rotation')):groupMatrix(PM,provider,item.path,T);
  if(mask&&v)matrix=PM.mul(matrix,affine(v.x,v.y,v.rotation,v.scaleX/100,v.scaleY/100));
  let points:Point[],hull:Point[],wrapped:Point[]|null=null;
  if(v){points=pathPoints(v);hull=v.vertices.flatMap((p:any)=>[[p.x,p.y],[p.x+p.inX,p.y+p.inY],[p.x+p.outX,p.y+p.outY]] as Point[]);
   const start=clamp(Number(v.trimStart)||0,0,100)/100,end=clamp(Number(v.trimEnd)||0,0,100)/100,offset=((Number(v.trimOffset)||0)/360%1+1)%1,lo=(start+offset)%1,span=end-start;
   if(span<.999999){if(span<=0)points=[];else if(lo+span<=1)points=trimPoints(points,lo,lo+span);else{wrapped=trimPoints(points,0,lo+span-1);points=trimPoints(points,lo,1);}}
  }else if(PM.evP(provider,mask.shape,T,`m.${mask.id}.shape`)==='ellipse'){const w=Math.abs(ev('w'))/2,h=Math.abs(ev('h'))/2;points=Array.from({length:129},(_,i)=>[Math.cos(i*Math.PI/64)*w,Math.sin(i*Math.PI/64)*h] as Point);hull=[[-w,-h],[w,-h],[w,h],[-w,h]];}
  else{const w=ev('w')/2,h=ev('h')/2;points=[[-w,-h],[w,-h],[w,h],[-w,h],[-w,-h]];hull=points;}
  const copies=v?Math.min(256,Math.max(1,Math.round(v.copies??1))):1;
  for(let i=0;i<copies;i++){count+=points.length;if(count>65536)throw new EffectInputError('Stroke geometry exceeds the segment budget');const m=v?PM.mul(matrix,affine(i*(v.repeatX??0),i*(v.repeatY??0),i*(v.repeatRotation??0))):matrix;out.push({points:points.map(p=>transform(m,p)),hull:hull.map(p=>transform(m,p))});if(wrapped)out.push({points:wrapped.map(p=>transform(m,p)),hull:hull.map(p=>transform(m,p))});}
 }return out;
}
export function brushBounds(curves:Curve[],width:number){
 if(!Number.isFinite(width))throw new EffectInputError('Non-finite stroke width');if(!(width>0))return null;const points=curves.flatMap(c=>c.hull);if(!points.length)return null;
 let x0=Infinity,y0=Infinity,x1=-Infinity,y1=-Infinity;for(const [x,y]of points){if(!Number.isFinite(x+y))throw new EffectInputError('Non-finite stroke geometry');x0=Math.min(x0,x);y0=Math.min(y0,y);x1=Math.max(x1,x);y1=Math.max(y1,y);}const r=width/2+1;return{x0:Math.floor(x0-r),y0:Math.floor(y0-r),w:Math.ceil(x1+r)-Math.floor(x0-r),h:Math.ceil(y1+r)-Math.floor(y0-r)};
}
/** Maximum coverage avoids opacity buildup where brush dabs overlap. Work and
 * allocation are bounded independently of project geometry or zoom. */
export function rasterBrush(canvas:HTMLCanvasElement,curves:Curve[],matrix:number[],width:number,hardness:number,spacing:number,start:number,end:number,sequential:boolean){
 if(![width,hardness,spacing,start,end,...matrix].every(Number.isFinite))throw new EffectInputError('Non-finite stroke brush');
 const W=canvas.width,H=canvas.height,c=canvas.getContext('2d')!,radius=Math.max(0,width/2);
 if(!radius||end<=start)return;const lengths=curves.map(c=>c.points.reduce((sum,p,i,a)=>sum+(i?Math.hypot(p[0]-a[i-1]![0],p[1]-a[i-1]![1]):0),0)),total=lengths.reduce((a,b)=>a+b,0);let offset=0,work=0;
 const paths=curves.map((curve,i)=>{const len=lengths[i]!,lo=sequential?(start/100*total-offset)/(len||1):start/100,hi=sequential?(end/100*total-offset)/(len||1):end/100;offset+=len;return trimPoints(curve.points,lo,hi);});
 if(hardness>=100&&spacing<=0){c.setTransform(...matrix as [number,number,number,number,number,number]);c.strokeStyle='#fff';c.lineWidth=radius*2;c.lineCap='round';c.lineJoin='round';for(const points of paths){if(!points.length)continue;c.beginPath();c.moveTo(...points[0]!);for(const p of points.slice(1))c.lineTo(...p);c.stroke();}return;}
 const determinant=matrix[0]!*matrix[3]!-matrix[1]!*matrix[2]!;if(Math.abs(determinant)<1e-12)return;
 const inv=[matrix[3]!/determinant,-matrix[1]!/determinant,-matrix[2]!/determinant,matrix[0]!/determinant,0,0];inv[4]=-(inv[0]!*matrix[4]!+inv[2]!*matrix[5]!);inv[5]=-(inv[1]!*matrix[4]!+inv[3]!*matrix[5]!);
 const antialias=.5*Math.max(Math.hypot(inv[0]!,inv[1]!),Math.hypot(inv[2]!,inv[3]!)),padX=radius*Math.hypot(matrix[0]!,matrix[2]!)+1,padY=radius*Math.hypot(matrix[1]!,matrix[3]!)+1;
 const image=c.createImageData(W,H),data=image.data,inner=radius*clamp(hardness/100,0,1);
 const draw=(a:Point,b:Point)=>{const A=transform(matrix,a),B=transform(matrix,b),x0=clamp(Math.floor(Math.min(A[0],B[0])-padX),0,W),x1=clamp(Math.ceil(Math.max(A[0],B[0])+padX),0,W),y0=clamp(Math.floor(Math.min(A[1],B[1])-padY),0,H),y1=clamp(Math.ceil(Math.max(A[1],B[1])+padY),0,H);work+=(x1-x0)*(y1-y0);if(work>64*1024*1024)throw new EffectInputError('Stroke brush exceeds the raster work budget');const dx=b[0]-a[0],dy=b[1]-a[1],len=dx*dx+dy*dy;
  for(let y=y0;y<y1;y++)for(let x=x0;x<x1;x++){const px=inv[0]!*(x+.5)+inv[2]!*(y+.5)+inv[4]!,py=inv[1]!*(x+.5)+inv[3]!*(y+.5)+inv[5]!,t=len?clamp(((px-a[0])*dx+(py-a[1])*dy)/len,0,1):0,d=Math.hypot(px-a[0]-t*dx,py-a[1]-t*dy),alpha=Math.round(clamp((radius+antialias-d)/Math.max(2*antialias,radius-inner+antialias),0,1)*255),i=(y*W+x)*4;if(alpha>data[i+3]!){data[i]=data[i+1]=data[i+2]=255;data[i+3]=alpha;}}
 };
 for(const points of paths){if(spacing<=0){for(let i=1;i<points.length;i++)draw(points[i-1]!,points[i]!);continue;}const step=Math.max(.25,width*spacing/100);let distance=0,next=0,dabs=0;for(let i=1;i<points.length;i++){const a=points[i-1]!,b=points[i]!,len=Math.hypot(b[0]-a[0],b[1]-a[1]);if(!len)continue;while(next<=distance+len){if(++dabs>65536)throw new EffectInputError('Stroke brush exceeds the dab budget');const t=(next-distance)/len,p:Point=[a[0]+t*(b[0]-a[0]),a[1]+t*(b[1]-a[1])];draw(p,p);next+=step;}distance+=len;}}
 c.putImageData(image,0,0);
}
