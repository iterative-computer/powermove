import * as THREE from 'three';

/** 4096² RGBA8 is at most 64 MiB per capture before mipmaps. */
export const MATERIAL_TEXTURE_MAX_EDGE=4096;

export interface MaterialTextureSize {width:number;height:number}

/** Keep the source's aspect and native resolution, increasing vector/composite
 * density when the camera needs more texels. The capture budget and device limit may reduce it. */
export function materialTextureSize(width:number,height:number,minimum?:MaterialTextureSize,limit=8192):MaterialTextureSize {
  const w=Number.isFinite(width) && width>0?width:2,h=Number.isFinite(height) && height>0?height:2;
  const demand=(value:number|undefined)=>Number.isFinite(value) && (value ?? 0)>0?value!:0;
  const scale=Math.min(Math.max(2,Math.min(MATERIAL_TEXTURE_MAX_EDGE,limit))/Math.max(w,h),Math.max(1,demand(minimum?.width)/w,demand(minimum?.height)/h));
  return {width:Math.max(2,Math.round(w*scale)),height:Math.max(2,Math.round(h*scale))};
}

type UVPatch={points:THREE.Vector3[];u:THREE.Vector3;v:THREE.Vector3};
type GeometrySample = {position:unknown;uv:unknown;index:unknown;stamp:string;box:THREE.Box3;patches:UVPatch[]};
const geometrySamples=new WeakMap<THREE.BufferGeometry,Map<string,GeometrySample>>();
function geometrySample(geometry:THREE.BufferGeometry,channel:number,materialIndex?:number):GeometrySample|null {
  const position=geometry.getAttribute('position'),uv=geometry.getAttribute(channel?`uv${channel}`:'uv');
  if(!position || !uv)return null;
  const index=geometry.index,key=`${channel}:${materialIndex ?? 'all'}`;
  let entries=geometrySamples.get(geometry);if(!entries){entries=new Map();geometrySamples.set(geometry,entries);}
  const groups=materialIndex==null || !geometry.groups.length?[{start:0,count:index?.count ?? position.count}]
    :geometry.groups.filter(group=>group.materialIndex===materialIndex);
  const version=(attribute:any)=>attribute?.version ?? attribute?.data?.version;
  const stamp=JSON.stringify([version(position),version(uv),version(index),position.count,uv.count,index?.count,groups.map(group=>[group.start,group.count])]);
  const cached=entries.get(key);
  if(cached && cached.position===position && cached.uv===uv && cached.index===index && cached.stamp===stamp)return cached;
  const box=new THREE.Box3(),point=new THREE.Vector3();
  for(const group of groups)for(let offset=group.start,end=Math.min(group.start+group.count,index?.count ?? position.count);offset<end;offset++){
    const at=index?index.getX(offset):offset;
    box.expandByPoint(point.fromBufferAttribute(position,at));
  }
  if(box.isEmpty())return null;
  const patches:UVPatch[]=[],triangles=groups.reduce((count,group)=>count+Math.floor(group.count/3),0),count=Math.min(16,triangles);
  for(let patch=0;patch<count;patch++){
    let triangle=count===1?0:Math.floor(patch*(triangles-1)/(count-1)),offset=0;
    for(const group of groups){const size=Math.floor(group.count/3);if(triangle<size){offset=group.start+triangle*3;break;}triangle-=size;}
    const indices=[offset,offset+1,offset+2].map(at=>index?index.getX(at):at);
    const points=indices.map(at=>new THREE.Vector3().fromBufferAttribute(position,at));
    const [a,b,c]=indices as [number,number,number],du1=uv.getX(b)-uv.getX(a),dv1=uv.getY(b)-uv.getY(a),du2=uv.getX(c)-uv.getX(a),dv2=uv.getY(c)-uv.getY(a);
    const determinant=du1*dv2-du2*dv1;if(Math.abs(determinant)<1e-12)continue;
    const e1=points[1]!.clone().sub(points[0]!),e2=points[2]!.clone().sub(points[0]!);
    patches.push({points,u:e1.clone().multiplyScalar(dv2).addScaledVector(e2,-dv1).divideScalar(determinant),
      v:e2.clone().multiplyScalar(du1).addScaledVector(e1,-du2).divideScalar(determinant)});
  }
  const sample={position,uv,index,stamp,box,patches};entries.set(key,sample);return sample;
}

/** Estimate texels from the material's projected geometry and UV coverage.
 * Geometry/UV scans are cached; camera motion evaluates bounded UV patches.
 * A small UV island needs a larger full texture than a surface using 0..1. */
export function projectedMaterialSize(object:THREE.Object3D,texture:THREE.Texture,width:number,height:number,camera:THREE.Camera):MaterialTextureSize|null {
  let neededWidth=0,neededHeight=0;
  const box=new THREE.Box3(),projection=new THREE.Matrix4().multiplyMatrices(camera.projectionMatrix,camera.matrixWorldInverse);
  const frustum=new THREE.Frustum().setFromProjectionMatrix(projection),matrix=new THREE.Matrix4();
  const clip=new THREE.Vector4(),du=new THREE.Vector4(),dv=new THREE.Vector4();
  if(texture.matrixAutoUpdate)texture.updateMatrix();
  const uv=texture.matrix.elements,a=uv[0]!,b=uv[3]!,c=uv[1]!,d=uv[4]!,determinant=a*d-b*c;
  if(Math.abs(determinant)<1e-12)return null;
  object.traverseVisible((child:any)=>{
    if(!child.isMesh)return;
    const materials=Array.isArray(child.material)?child.material:[child.material];
    materials.forEach((material:any,index:number)=>{
      if(!['map','emissiveMap'].some(slot=>material[slot]===texture))return;
      const sample=geometrySample(child.geometry,texture.channel,Array.isArray(child.material)?index:undefined);if(!sample)return;
      box.copy(sample.box).applyMatrix4(child.matrixWorld);
      if(!frustum.intersectsBox(box))return;
      matrix.multiplyMatrices(projection,child.matrixWorld);
      for(const patch of sample.patches){
        du.set(patch.u.x,patch.u.y,patch.u.z,0).applyMatrix4(matrix);dv.set(patch.v.x,patch.v.y,patch.v.z,0).applyMatrix4(matrix);
        for(const point of patch.points){
          clip.set(point.x,point.y,point.z,1).applyMatrix4(matrix);if(clip.w<=0 || clip.z>clip.w)continue;
          const divisor=clip.w*clip.w;
          const ux=(du.x*clip.w-clip.x*du.w)/divisor*width/2,uy=(du.y*clip.w-clip.y*du.w)/divisor*height/2;
          const vx=(dv.x*clip.w-clip.x*dv.w)/divisor*width/2,vy=(dv.y*clip.w-clip.y*dv.w)/divisor*height/2;
          neededWidth=Math.max(neededWidth,Math.hypot(ux*d-vx*c,uy*d-vy*c)/Math.abs(determinant));
          neededHeight=Math.max(neededHeight,Math.hypot(vx*a-ux*b,vy*a-uy*b)/Math.abs(determinant));
        }
      }
    });
  });
  if(!neededWidth || !neededHeight)return null;
  // Upgrade in buckets and retain that density during camera motion instead
  // of re-capturing a slightly different resolution on every drag step.
  const bucket=(pixels:number)=>2**Math.ceil(Math.log2(Math.max(2,pixels)));
  return {width:bucket(neededWidth),height:bucket(neededHeight)};
}

export function materialTextureLimit(PM:any):number {
  const GL=PM.GL;
  if(GL?.gl && !GL.maxTextureSize)GL.maxTextureSize=GL.gl.getParameter?.(GL.gl.MAX_TEXTURE_SIZE);
  return Number(GL?.maxTextureSize)>0?Number(GL.maxTextureSize):8192;
}
