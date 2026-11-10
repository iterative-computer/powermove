import * as THREE from 'three';
import {splitPaintColor} from '../../../../shared/paint-color';
import ClipperLib from 'clipper-lib';
import type { ExtrusionValues } from './extrusion-data';

export type Point=[number,number];
export interface Outline {outer:Point[];holes:Point[][];color?:string;opacity?:number}
export interface OutlineBounds {x0:number;y0:number;x1:number;y1:number}
const SCALE=1024;
const intPath=(points:Point[])=>points.map(([X,Y])=>{
  if(!Number.isFinite(X+Y) || Math.max(Math.abs(X),Math.abs(Y))>1e9)throw new Error('Outline contains invalid coordinates');
  return {X:Math.round(X*SCALE),Y:Math.round(Y*SCALE)};
});
const points=(path:ClipperLib.Path):Point[]=>path.map(p=>[p.X/SCALE,p.Y/SCALE]);
/** Clipper resolves self intersections with the same nonzero rule as Canvas,
 * retaining compound holes rather than filling them with a guessed fan. */
export function resolveContours(contours:Point[][],fillRule:'nonzero'|'evenodd'='nonzero'):Outline[]{
  const clip=new ClipperLib.Clipper(),tree=new ClipperLib.PolyTree();
  clip.StrictlySimple=true;
  clip.AddPaths(contours.filter(c=>c.length>=3).map(intPath),ClipperLib.PolyType.ptSubject,true);
  const rule=fillRule==='evenodd'?ClipperLib.PolyFillType.pftEvenOdd:ClipperLib.PolyFillType.pftNonZero;
  clip.Execute(ClipperLib.ClipType.ctUnion,tree,rule,rule);
  return ClipperLib.JS.PolyTreeToExPolygons(tree).map(poly=>({outer:points(poly.outer),holes:poly.holes.map(points)}));
}
/** Subtract stroked regions so coplanar fill/stroke caps never z-fight. */
export function subtractOutlines(fill:Outline[],cut:Outline[]):Outline[]{
  if(!cut.length)return fill;
  const clip=new ClipperLib.Clipper(),tree=new ClipperLib.PolyTree();
  const contours=(outlines:Outline[])=>outlines.flatMap(o=>[o.outer,...o.holes]).map(intPath);
  clip.AddPaths(contours(fill),ClipperLib.PolyType.ptSubject,true);clip.AddPaths(contours(cut),ClipperLib.PolyType.ptClip,true);
  clip.Execute(ClipperLib.ClipType.ctDifference,tree,ClipperLib.PolyFillType.pftNonZero,ClipperLib.PolyFillType.pftNonZero);
  return ClipperLib.JS.PolyTreeToExPolygons(tree).map(poly=>({outer:points(poly.outer),holes:poly.holes.map(points)}));
}
export function strokeContours(contour:Point[],width:number,closed:boolean):Outline[]{
  if(width<=0 || contour.length<2)return [];
  const offset=new ClipperLib.ClipperOffset(2,.15*SCALE),result:ClipperLib.Paths=[];
  offset.AddPath(intPath(contour),ClipperLib.JoinType.jtRound,closed?ClipperLib.EndType.etClosedLine:ClipperLib.EndType.etOpenRound);
  offset.Execute(result,width/2*SCALE);
  return resolveContours(result.map(points));
}
export function outlineBounds(outlines:Outline[]):OutlineBounds {
  let x0=Infinity,y0=Infinity,x1=-Infinity,y1=-Infinity;
  for(const outline of outlines)for(const [x,y] of outline.outer){x0=Math.min(x0,x);y0=Math.min(y0,y);x1=Math.max(x1,x);y1=Math.max(y1,y);}
  return Number.isFinite(x0)?{x0,y0,x1,y1}:{x0:0,y0:0,x1:1,y1:1};
}
const area=(ring:Point[])=>THREE.ShapeUtils.area(ring.map(p=>new THREE.Vector2(...p)));
function oriented(ring:Point[],ccw:boolean):Point[]{return (area(ring)>0)===ccw?ring:[...ring].reverse();}
function clean(ring:Point[]):Point[]{
  return ring.filter((p,i)=>{const prev=ring[(i+ring.length-1)%ring.length]!;return Math.hypot(p[0]-prev[0],p[1]-prev[1])>1e-5;});
}
/** Cap UVs use layer-local bounds. One perimeter seam per contour, and a shared
 * depth coordinate, keep side and bevel texture coordinates continuous. */
export function buildExtrusionGeometry(outlines:Outline[],options:ExtrusionValues,bounds=outlineBounds(outlines)):THREE.BufferGeometry {
  let vertexCount=0;
  const positions:number[][]=[[],[],[],[]],normals:number[][]=[[],[],[],[]],uvs:number[][]=[[],[],[],[]],colors:number[][]=[[],[],[],[]];
  const defaults:number[][]=[[],[],[],[]];
  const depth=options.depth,b=options.bevelStyle==='none'?0:Math.min(options.bevelDepth,depth/2);
  const steps=options.bevelStyle==='angular'?1:options.quality==='draft'?3:options.quality==='high'?12:6;
  const w=Math.max(1e-5,bounds.x1-bounds.x0),h=Math.max(1e-5,bounds.y1-bounds.y0);
  const vertex=(surface:number,p:number[],normal:number[],uv:number[],color:THREE.Color,defaultColor=true,opacity=1)=>{
    if(![...p,...normal,...uv,color.r,color.g,color.b,opacity].every(Number.isFinite))throw new Error('Extrusion contains non-finite vertex data');
    if(++vertexCount>500000)throw new Error('Extrusion exceeds the 500,000 vertex geometry budget; reduce quality or text length');
    defaults[surface]!.push(defaultColor?1:0);positions[surface]!.push(...p);normals[surface]!.push(...normal);uvs[surface]!.push(...uv);colors[surface]!.push(color.r,color.g,color.b,opacity);
  };
  for(const outline of outlines){
    // Input artwork is y-down. Mesh coordinates are y-up with the original
    // front at z=0 and extrusion extending away from the viewer.
    const convert=(r:Point[])=>clean(r.map(([x,y])=>[x,-y] as Point));
    const outer=oriented(convert(outline.outer),true),holes=outline.holes.map(h=>oriented(convert(h),false));
    if(outer.length<3)continue;
    const paint=splitPaintColor(outline.color || '#FFFFFF'),opacity=(outline.opacity ?? 1)*paint.opacity;
    const rings=[outer,...holes.filter(h=>h.length>=3)],flat=rings.flat(),color=new THREE.Color(paint.color);
    const triangles=THREE.ShapeUtils.triangulateShape(outer.map(p=>new THREE.Vector2(...p)),rings.slice(1).map(r=>r.map(p=>new THREE.Vector2(...p))));
    for(const tri of triangles){
      for(const index of tri){const [x,y]=flat[index]!;vertex(0,[x,y,0],[0,0,1],[(x-bounds.x0)/w,(-y-bounds.y0)/h],color,!outline.color,opacity);}
      if(depth>0)for(const index of [...tri].reverse()){const [x,y]=flat[index]!;vertex(3,[x,y,-depth],[0,0,-1],[(x-bounds.x0)/w,(-y-bounds.y0)/h],color,!outline.color,opacity);}
    }
    if(!depth)continue;
    for(let ri=0;ri<rings.length;ri++){
      const ring=rings[ri]!,count=ring.length,distances=[0];
      for(let i=0;i<count;i++){const a=ring[i]!,next=ring[(i+1)%count]!;distances.push(distances[i]!+Math.hypot(next[0]-a[0],next[1]-a[1]));}
      const perimeter=distances[count]!,widthLimit=Math.abs(area(ring))/Math.max(1,perimeter)*.4;
      const bevel=Math.min(b*(ri?options.holeBevelDepth/100:1),widthLimit);
      // Outward miter vectors; limits prevent spikes at acute star/glyph joins.
      const offsets=ring.map((p,i)=>{
        const prev=ring[(i+count-1)%count]!,next=ring[(i+1)%count]!;
        const d1=new THREE.Vector2(p[0]-prev[0],p[1]-prev[1]).normalize(),d2=new THREE.Vector2(next[0]-p[0],next[1]-p[1]).normalize();
        const n1=new THREE.Vector2(d1.y,-d1.x),n2=new THREE.Vector2(d2.y,-d2.x),m=n1.clone().add(n2).normalize();
        const factor=Math.min(2,1/Math.max(.25,m.dot(n1)));
        return {x:m.x*factor,y:m.y*factor};
      });
      const profile:(number[])[]=[];
      if(bevel){
        for(let j=0;j<=steps;j++){
          const t=j/steps,a=t*Math.PI/2;
          const r=options.bevelStyle==='convex'?Math.sin(a):options.bevelStyle==='concave'?1-Math.cos(a):t;
          const z=options.bevelStyle==='convex'?1-Math.cos(a):options.bevelStyle==='concave'?Math.sin(a):t;
          profile.push([r*bevel,-z*bevel,1]);
        }
      }else profile.push([0,0,2]);
      profile.push([bevel,-depth+bevel,2]);
      if(bevel)for(let j=steps-1;j>=0;j--){const row=profile[j]!;profile.push([row[0]!,-depth-row[1]!,1]);}
      // Arc-length V has no reset at a material boundary.
      const lengths=[0];for(let j=1;j<profile.length;j++)lengths.push(lengths[j-1]!+Math.hypot(profile[j]![0]!-profile[j-1]![0]!,profile[j]![1]!-profile[j-1]![1]!));
      for(let j=0;j<profile.length-1;j++){
        const a=profile[j]!,next=profile[j+1]!,surface=next[2]!;
        if(Math.hypot(next[0]!-a[0]!,next[1]!-a[1]!)<1e-8)continue;
        for(let i=0;i<count;i++){
          const k=(i+1)%count;
          const point=(index:number,row:number)=>{const p=ring[index]!,o=offsets[index]!,r=profile[row]!;return [p[0]+o.x*r[0]!,p[1]+o.y*r[0]!,r[1]!];};
          const faceNormal=(index:number,row:number)=>{
            const prevRow=Math.max(0,row-1),nextRow=Math.min(profile.length-1,row+1);
            const dr=profile[nextRow]![0]!-profile[prevRow]![0]!,dz=profile[nextRow]![1]!-profile[prevRow]![1]!;
            // Keep angular profiles faceted across rings. Smooth round profiles.
            const slope=options.bevelStyle==='angular'?(next[0]!-a[0]!)/Math.max(1e-8,a[1]!-next[1]!):dr/Math.max(1e-8,-dz);
            const p=ring[i]!,q=ring[k]!,edge=new THREE.Vector2(q[0]-p[0],q[1]-p[1]).normalize();
            const prev=ring[(index+count-1)%count]!,cur=ring[index]!,after=ring[(index+1)%count]!;
            const e1=new THREE.Vector2(cur[0]-prev[0],cur[1]-prev[1]).normalize(),e2=new THREE.Vector2(after[0]-cur[0],after[1]-cur[1]).normalize();
            const n=e1.dot(e2)>.7?new THREE.Vector2(e1.y+e2.y,-e1.x-e2.x).normalize():new THREE.Vector2(edge.y,-edge.x);
            return new THREE.Vector3(n.x,n.y,slope).normalize().toArray();
          };
          // CCW contour: outward faces follow upper-left, lower-left, lower-right.
          for(const [index,row,u] of [[i,j,distances[i]!],[i,j+1,distances[i]!],[k,j+1,distances[i+1]!],[i,j,distances[i]!],[k,j+1,distances[i+1]!],[k,j,distances[i+1]!]])
            vertex(surface,point(index!,row!),faceNormal(index!,row!),[u!/Math.max(1,perimeter),lengths[row!]!/Math.max(1,lengths.at(-1)!)],color,!outline.color,opacity);
        }
      }
    }
  }
  const geometry=new THREE.BufferGeometry(),p:number[]=[],n:number[]=[],uv:number[]=[],c:number[]=[];
  for(let surface=0;surface<4;surface++){
    geometry.addGroup(p.length/3,positions[surface]!.length/3,surface);
    for(const value of positions[surface]!)p.push(value);for(const value of normals[surface]!)n.push(value);for(const value of uvs[surface]!)uv.push(value);for(const value of colors[surface]!)c.push(value);
  }
  if(p.length/3>500000)throw new Error('Extrusion exceeds the 500,000 vertex geometry budget; reduce quality or text length');
  geometry.setAttribute('position',new THREE.Float32BufferAttribute(p,3));geometry.setAttribute('normal',new THREE.Float32BufferAttribute(n,3));
  geometry.setAttribute('uv',new THREE.Float32BufferAttribute(uv,2));geometry.setAttribute('color',new THREE.Float32BufferAttribute(c,4));
  geometry.userData.hasVertexAlpha=c.some((value,index)=>index%4===3&&value<1);
  geometry.userData.sourceHasVertexAlpha=geometry.userData.hasVertexAlpha;
  geometry.userData.defaultColorVertices=defaults.flat().flatMap((value,index)=>value?[index]:[]);
  geometry.userData.defaultColorOpacities=geometry.userData.defaultColorVertices.map((index:number)=>c[index*4+3]);
  geometry.computeBoundingBox();geometry.computeBoundingSphere();return geometry;
}
