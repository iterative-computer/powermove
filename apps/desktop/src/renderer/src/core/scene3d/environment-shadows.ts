import * as THREE from 'three';

export interface EnvironmentSample { direction: THREE.Vector3; energy: THREE.Vector3; angularRadius:number }
const samples=new WeakMap<THREE.Texture,{version:number;flipY:boolean;value:EnvironmentSample[]}>();

/** Reduce linear panorama radiance to four solid-angle-weighted light regions.
 * Cache by texture revision; rotating a light never scans the panorama again. */
export function environmentSamples(texture:THREE.Texture):EnvironmentSample[] {
  const cached=samples.get(texture);
  if(cached?.version===texture.version && cached.flipY===texture.flipY)return cached.value;
  const image=texture.image as {data?:ArrayLike<number>;width:number;height:number}|undefined;
  if(!image?.data || !image.width || !image.height)return [];
  const channels=image.data.length/(image.width*image.height);
  if(channels!==3 && channels!==4)return [];
  const width=Math.min(64,image.width),height=Math.min(32,image.height);
  const points:Array<{direction:THREE.Vector3;energy:THREE.Vector3;weight:number}>=[];
  const read=(value:number)=>texture.type===THREE.HalfFloatType?THREE.DataUtils.fromHalfFloat(value)
    :texture.type===THREE.UnsignedByteType?value/255:value;
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){
    // Integrate each cell, retaining small HDR emitters instead of point-sampling.
    const energy=new THREE.Vector3();
    const x0=Math.floor(x*image.width/width),x1=Math.floor((x+1)*image.width/width);
    const y0=Math.floor(y*image.height/height),y1=Math.floor((y+1)*image.height/height);
    for(let sy=y0;sy<y1;sy++){
      const area=Math.sin(Math.PI*(sy+.5)/image.height);
      for(let sx=x0;sx<x1;sx++)for(let c=0;c<3;c++){
        let v=read(image.data[(sy*image.width+sx)*channels+c]!);
        if(texture.colorSpace===THREE.SRGBColorSpace)v=v<=.04045?v/12.92:((v+.055)/1.055)**2.4;
        if(Number.isFinite(v))energy.setComponent(c,energy.getComponent(c)+Math.max(0,v)*area);
      }
    }
    const weight=energy.dot(new THREE.Vector3(.2126,.7152,.0722));
    if(weight<=1e-10)continue;
    const longitude=((x+.5)/width-.5)*2*Math.PI;
    const latitude=((texture.flipY?1-(y+.5)/height:(y+.5)/height)-.5)*Math.PI;
    points.push({direction:new THREE.Vector3(Math.cos(latitude)*Math.cos(longitude),Math.sin(latitude),Math.cos(latitude)*Math.sin(longitude)),energy,weight});
  }
  const centers:THREE.Vector3[]=[];
  for(let i=0;i<Math.min(4,points.length);i++){
    let best=points[0]!,score=-1;
    for(const p of points){
      const distance=centers.length?Math.min(...centers.map(c=>1-c.dot(p.direction))):1;
      const next=p.weight*distance;
      if(next>score){best=p;score=next;}
    }
    if(score<=1e-12)break;
    centers.push(best.direction.clone());
  }
  let value:EnvironmentSample[]=[];
  for(let iteration=0;iteration<8 && centers.length;iteration++){
    const groups=centers.map(()=>({direction:new THREE.Vector3(),energy:new THREE.Vector3()}));
    for(const p of points){
      let index=0,best=-Infinity;
      centers.forEach((c,i)=>{const dot=c.dot(p.direction);if(dot>best){best=dot;index=i;}});
      groups[index]!.direction.addScaledVector(p.direction,p.weight);groups[index]!.energy.add(p.energy);
    }
    value=groups.map((g,i)=>({direction:g.direction.lengthSq()>1e-20?g.direction.normalize():centers[i]!.clone(),energy:g.energy,angularRadius:0}));
    value.forEach((g,i)=>centers[i]!.copy(g.direction));
  }
  const spread=value.map(()=>({sum:0,weight:0}));
  for(const p of points){
    let index=0,best=-Infinity;
    value.forEach((s,i)=>{const dot=s.direction.dot(p.direction);if(dot>best){best=dot;index=i;}});
    const cell=spread[index];if(!cell)continue;
    cell.sum+=Math.acos(THREE.MathUtils.clamp(best,-1,1))**2*p.weight;cell.weight+=p.weight;
  }
  value.forEach((s,i)=>s.angularRadius=Math.min(.65,Math.sqrt(2*spread[i]!.sum/Math.max(1e-10,spread[i]!.weight))));
  const total=value.reduce((sum,s)=>sum+s.energy.length(),0);
  for(const s of value)s.energy.multiplyScalar(1/Math.max(total,1e-10));
  samples.set(texture,{version:texture.version,flipY:texture.flipY,value});
  return value;
}

const pars=/* glsl */`
#if defined( USE_SHADOWMAP ) && NUM_DIR_LIGHT_SHADOWS > 0
uniform vec4 pmEnvironmentShadowWeights[ NUM_DIR_LIGHT_SHADOWS ];
// Blocker distance and emitter size give contact-hardening soft shadows.
// Fixed Poisson taps avoid temporal noise during playback and export.
vec2 pmEnvironmentDisk(int index, float count) {
  float angle=float(index)*2.39996323;
  return vec2(cos(angle),sin(angle))*sqrt((float(index)+0.5)/count);
}
float pmEnvironmentShadow(sampler2D map, DirectionalLightShadow shadow, vec4 coord, float width) {
  vec3 p=coord.xyz/coord.w;p.z+=shadow.shadowBias;
  if(p.x<0.0||p.x>1.0||p.y<0.0||p.y>1.0||p.z>1.0)return 1.0;
  vec2 texel=1.0/shadow.shadowMapSize;
  // Compare against the receiver's tangent plane at every offset. Using the
  // centre depth over a sloping surface creates stippling and false blockers.
  vec3 dx=dFdx(p),dy=dFdy(p);
  float determinant=dx.x*dy.y-dx.y*dy.x;
  vec2 gradient=abs(determinant)>0.000000000001
    ? vec2(dy.y*dx.z-dx.y*dy.z,dx.x*dy.z-dy.x*dx.z)/determinant : vec2(0.0);
  float pcfBias=shadow.shadowBias-dot(abs(gradient),texel)*1.5;
  // A small emitter uses the exact same subtexel PCF filter as a Sun light.
  if(width<=texel.x)return getShadow(map,shadow.shadowMapSize,shadow.shadowIntensity,pcfBias,shadow.shadowRadius,coord);
  float blocker=0.0,count=0.0;
  float search=min(0.04,width*0.25);
  for(int tap=0;tap<12;tap++){
    vec2 offset=pmEnvironmentDisk(tap,12.0)*search;
    float depth=unpackRGBAToDepth(texture2D(map,p.xy+offset));
    float distance=p.z+dot(gradient,offset)-depth;
    if(distance>0.0){blocker+=distance;count+=1.0;}
  }
  if(count==0.0)return 1.0;
  float radius=blocker/count*width;
  if(radius<=1.5*texel.x)return getShadow(map,shadow.shadowMapSize,shadow.shadowIntensity,pcfBias,shadow.shadowRadius,coord);
  float visibility=0.0;
  for(int tap=0;tap<16;tap++){
    vec2 offset=pmEnvironmentDisk(tap,16.0)*radius;
    vec4 sampleCoord=coord;sampleCoord.xy+=offset*coord.w;
    sampleCoord.z+=dot(gradient,offset)*coord.w;
    visibility+=getShadow(map,shadow.shadowMapSize,1.0,pcfBias,shadow.shadowRadius,sampleCoord);
  }
  return mix(1.0,visibility/16.0,shadow.shadowIntensity);
}
#endif
vec3 pmEnvironmentVisibility(vec3 direction, float exponent) {
  vec3 total = vec3(0.0), visible = vec3(0.0);
  #if defined( USE_SHADOWMAP ) && NUM_DIR_LIGHT_SHADOWS > 0
  vec3 weight;
  DirectionalLightShadow shadow;
  float visibility;
  #pragma unroll_loop_start
  for ( int i = 0; i < NUM_DIR_LIGHT_SHADOWS; i ++ ) {
    weight = pmEnvironmentShadowWeights[ i ].rgb * pow(max(dot(direction, directionalLights[ i ].direction), 0.0), exponent);
    total += weight;
    shadow = directionalLightShadows[ i ];
    visibility = receiveShadow && dot(weight,vec3(1.0))>0.000001 ? pmEnvironmentShadow(directionalShadowMap[ i ], shadow, vDirectionalShadowCoord[ i ], pmEnvironmentShadowWeights[ i ].a) : 1.0;
    visible += weight * visibility;
  }
  #pragma unroll_loop_end
  #endif
  return clamp((visible + vec3(0.00001)) / (total + vec3(0.00001)), 0.0, 1.0);
}
`;

/** Shadow-only directional probes attenuate existing IBL. Their intensity is
 * zero: they never add a second copy of the environment's illumination. */
export class EnvironmentShadows {
  readonly lights:THREE.DirectionalLight[]=[];
  readonly weights={value:[] as THREE.Vector4[]};
  private installed=new WeakSet<THREE.Material>();
  private pass:string|null=null;
  constructor(private scene:THREE.Scene){}

  sync(texture:THREE.Texture|null,rotation:THREE.Euler,enabled:boolean):void {
    const regions=enabled && texture?environmentSamples(texture):[];
    this.scene.updateMatrixWorld(true);
    const box=new THREE.Box3();
    if(regions.length)this.scene.traverseVisible(object=>{
      const mesh=object as THREE.Mesh;if(mesh.isMesh)box.expandByObject(mesh);
    });
    const count=box.isEmpty()?0:regions.length;
    while(this.lights.length>count){const light=this.lights.pop()!;light.removeFromParent();light.target.removeFromParent();light.shadow.dispose();}
    while(this.lights.length<count){
      const light=new THREE.DirectionalLight(0xffffff,0);light.castShadow=true;
      light.userData.environmentShadow=true;light.shadow.mapSize.set(1024,1024);
      light.shadow.autoUpdate=false;light.shadow.bias=-.0002;
      this.lights.push(light);this.scene.add(light,light.target);
    }
    if(count){
      const center=box.getCenter(new THREE.Vector3()),radius=Math.max(.1,box.getSize(new THREE.Vector3()).length()*.55);
      // Match Three's left-handed environment lookup, including compound XYZ rotation.
      const transform=new THREE.Matrix4().makeRotationFromEuler(new THREE.Euler(-rotation.x,-rotation.y,-rotation.z,rotation.order)).invert();
      this.lights.forEach((light,i)=>{
        light.userData.environmentEnergy=regions[i]!.energy;
        const direction=regions[i]!.direction.clone().transformDirection(transform);
        light.target.position.copy(center);light.position.copy(center).addScaledVector(direction,radius*2+.1);
        const camera=light.shadow.camera;
        Object.assign(camera,{left:-radius,right:radius,top:radius,bottom:-radius,near:.01,far:radius*4+.2});
        camera.updateProjectionMatrix();light.shadow.normalBias=radius*.0005;
        light.userData.environmentSoftness=Math.tan(regions[i]!.angularRadius)*(camera.far-camera.near)/(2*radius);
        light.shadow.needsUpdate=true;
      });
    }
    this.pass=null;
    // WebGLLights puts shadow-casting directional lights first, retaining scene
    // traversal order. Non-environment lights receive zero attenuation weights.
    const weights:THREE.Vector4[]=[];
    this.scene.traverseVisible(object=>{
      const light=object as THREE.DirectionalLight;
      if(light.isDirectionalLight && light.castShadow){
        const energy=light.userData.environmentEnergy as THREE.Vector3|undefined;
        weights.push(energy?new THREE.Vector4(energy.x,energy.y,energy.z,light.userData.environmentSoftness):new THREE.Vector4(0,0,0,0));
      }
      const mesh=object as THREE.Mesh;
      if(mesh.isMesh)for(const material of Array.isArray(mesh.material)?mesh.material:[mesh.material])this.install(material);
    });
    this.weights.value=weights;
  }

  preparePass(key:string):void {
    if(this.pass===key)return;
    this.pass=key;for(const light of this.lights)light.shadow.needsUpdate=true;
  }

  private install(material:THREE.Material):void {
    if(this.installed.has(material) || !(material as THREE.MeshStandardMaterial).isMeshStandardMaterial)return;
    this.installed.add(material);
    const previous=material.onBeforeCompile,key=material.customProgramCacheKey(),weights=this.weights;
    material.onBeforeCompile=function(shader,renderer){
      previous.call(this,shader,renderer);shader.uniforms.pmEnvironmentShadowWeights=weights;
      shader.fragmentShader=shader.fragmentShader.replace('#include <shadowmap_pars_fragment>',`#include <shadowmap_pars_fragment>\n${pars}`);
      const chunk=THREE.ShaderChunk.lights_fragment_maps
        .replace('getIBLIrradiance( geometryNormal )','(getIBLIrradiance( geometryNormal ) * pmEnvironmentVisibility(geometryNormal, 1.0))')
        .replace('getIBLRadiance( geometryViewDir, geometryNormal, material.roughness )','(getIBLRadiance( geometryViewDir, geometryNormal, material.roughness ) * pmEnvironmentVisibility(normalize(mix(reflect(-geometryViewDir, geometryNormal), geometryNormal, material.roughness * material.roughness)), max(1.0, 2.0 / max(0.01, pow(material.roughness, 4.0)) - 2.0)))');
      shader.fragmentShader=shader.fragmentShader.replace('#include <lights_fragment_maps>',chunk);
    };
    material.customProgramCacheKey=()=>`${key}:pm-environment-shadows-v2`;material.needsUpdate=true;
  }

  dispose():void {
    for(const light of this.lights){light.removeFromParent();light.target.removeFromParent();light.shadow.dispose();}
    this.lights.length=0;this.weights.value=[];
  }
}
