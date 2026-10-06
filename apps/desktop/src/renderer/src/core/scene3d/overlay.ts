import * as THREE from 'three';
import type {SceneRuntime} from './runtime';
import type {ViewAxis} from './viewport';
import {AXIS_COLORS,type Axis,type Rect} from './modal';

/*
 * Blender's viewport overlays drawn over the composition: the floor grid and
 * its axes (user views only), selection outlines, light and camera wires, and
 * transform constraint guides. Overlays never draw into the composition, so
 * exports and captures are unaffected.
 */

export const SELECTED_COLOR='#f15800',ACTIVE_COLOR='#ffaa40',WIRE_COLOR='#c8c8c8';
export type Glyph={id:string;kind:'point'|'sun'|'spot'|'area'|'camera';position:THREE.Vector3;target:THREE.Vector3|null;up?:THREE.Vector3;fov?:number;aspect?:number;angle?:number;size?:[number,number];state:'none'|'selected'|'active'};
export type Guide={origin:THREE.Vector3;direction:THREE.Vector3;axis:Axis};
export interface OverlayFrame {
  camera:THREE.PerspectiveCamera|THREE.OrthographicCamera;
  rect:Rect;
  runtime:SceneRuntime|null;
  /** User view: draw the floor grid against the scene's depth. */
  userView:boolean;
  axis:ViewAxis|null;
  /** Pivot distance and field of view, for grid density. */
  distance:number;
  fov:number;
  overlays:boolean;
  selected:Set<string>;
  active:Set<string>;
  glyphs:Glyph[];
  guides:Guide[];
  /** Edit Mode: the edited model's points and edges (world space). */
  edit?:{points:THREE.Vector3[];selected:boolean[];edges:[number,number][]}|null;
  /** X-ray: the floor and wires show through models. */
  xray?:boolean;
  /** Drawn last and unclipped: the transform gizmo. */
  gizmo:THREE.Object3D|null;
}

const GRID_VERTEX=`
varying vec3 vWorld;
uniform float uFar;
void main(){
  vec4 world=modelMatrix*vec4(position,1.0);vWorld=world.xyz;
  gl_Position=projectionMatrix*viewMatrix*world;
  if(uFar>0.5)gl_Position.z=gl_Position.w*0.999999;
}`;
const GRID_FRAGMENT=`
varying vec3 vWorld;
uniform float uStep;uniform vec3 uCenter;uniform float uFade;uniform int uPlane;uniform vec3 uAxisA;uniform vec3 uAxisB;
float grid(vec2 coord,float step){vec2 c=coord/step;vec2 d=max(fwidth(c),vec2(1e-6));vec2 g=abs(fract(c-0.5)-0.5)/d;return 1.0-min(min(g.x,g.y),1.0);}
float axisLine(float v){float d=max(fwidth(v),1e-6);return 1.0-min(abs(v)/d/1.25,1.0);}
void main(){
  vec2 coord=uPlane==0?vWorld.xz:uPlane==1?vWorld.xy:vWorld.zy;
  float minor=grid(coord,uStep),major=grid(coord,uStep*10.0);
  vec4 color=vec4(vec3(0.62),max(minor*0.22,major*0.4));
  float a=axisLine(coord.y),b=axisLine(coord.x);
  color=mix(color,vec4(uAxisA,0.85),a);
  color=mix(color,vec4(uAxisB,0.85),b*(1.0-a));
  color.a*=1.0-smoothstep(uFade*0.3,uFade,distance(vWorld,uCenter));
  if(color.a<0.004)discard;
  gl_FragColor=color;
}`;
const OUTLINE_FRAGMENT=`
varying vec2 vUv;
uniform sampler2D uMask;uniform vec2 uTexel;uniform vec3 uSelected;uniform vec3 uActive;uniform float uRadius;
void main(){
  vec4 here=texture2D(uMask,vUv);
  if(max(here.r,here.g)>0.5)discard;
  float nearSelected=0.0,nearActive=0.0;
  for(int x=-2;x<=2;x++)for(int y=-2;y<=2;y++){
    vec2 offset=vec2(float(x),float(y));if(length(offset)>uRadius+0.01)continue;
    vec4 sampled=texture2D(uMask,vUv+offset*uTexel);nearSelected=max(nearSelected,sampled.r);nearActive=max(nearActive,sampled.g);
  }
  if(nearActive>0.5)gl_FragColor=vec4(uActive,1.0);
  else if(nearSelected>0.5)gl_FragColor=vec4(uSelected,1.0);
  else discard;
}`;

const PLANES:Record<string,{plane:number;a:Axis;b:Axis;rotate:(mesh:THREE.Mesh)=>void}>={
  floor:{plane:0,a:'x',b:'z',rotate:mesh=>mesh.rotation.set(-Math.PI/2,0,0)},
  front:{plane:1,a:'x',b:'y',rotate:mesh=>mesh.rotation.set(0,0,0)},
  side:{plane:2,a:'z',b:'y',rotate:mesh=>mesh.rotation.set(0,Math.PI/2,0)}
};

export class ViewportOverlay {
  readonly canvas:HTMLCanvasElement;
  private renderer:THREE.WebGLRenderer;
  private gridScene=new THREE.Scene();
  private grid:THREE.Mesh<THREE.PlaneGeometry,THREE.ShaderMaterial>;
  private depthMaterial=new THREE.MeshBasicMaterial({colorWrite:false});
  private maskMaterial=new THREE.MeshBasicMaterial({color:0xff0000,side:THREE.DoubleSide});
  private mask:THREE.WebGLRenderTarget|null=null;
  private quadScene=new THREE.Scene();
  private quadCamera=new THREE.OrthographicCamera(-1,1,1,-1,0,1);
  private outline:THREE.ShaderMaterial;
  private lineScene=new THREE.Scene();
  private gizmoScene=new THREE.Scene();
  private materials=new Map<string,THREE.LineBasicMaterial>();
  private disposed=false;

  constructor(private stage:HTMLElement){
    this.renderer=new THREE.WebGLRenderer({alpha:true,antialias:true});
    this.renderer.setPixelRatio(Math.min(2,window.devicePixelRatio||1));
    this.renderer.autoClear=false;
    this.canvas=this.renderer.domElement;
    this.canvas.setAttribute('aria-hidden','true');this.canvas.dataset.sceneGizmo='';this.canvas.dataset.sceneOverlay='';
    this.canvas.style.cssText='position:absolute;left:0;top:0;width:100%;height:100%;pointer-events:none;z-index:4';
    stage.append(this.canvas);
    this.grid=new THREE.Mesh(new THREE.PlaneGeometry(1,1),new THREE.ShaderMaterial({
      vertexShader:GRID_VERTEX,fragmentShader:GRID_FRAGMENT,transparent:true,depthWrite:false,side:THREE.DoubleSide,
      uniforms:{uStep:{value:1},uCenter:{value:new THREE.Vector3()},uFade:{value:50},uPlane:{value:0},uFar:{value:0},
        uAxisA:{value:new THREE.Color(AXIS_COLORS.x)},uAxisB:{value:new THREE.Color(AXIS_COLORS.z)}}
    }));
    this.grid.frustumCulled=false;this.gridScene.add(this.grid);
    this.outline=new THREE.ShaderMaterial({
      vertexShader:'varying vec2 vUv;void main(){vUv=uv;gl_Position=vec4(position.xy,0.0,1.0);}',fragmentShader:OUTLINE_FRAGMENT,
      transparent:true,depthTest:false,depthWrite:false,
      uniforms:{uMask:{value:null},uTexel:{value:new THREE.Vector2()},uSelected:{value:new THREE.Color(SELECTED_COLOR)},uActive:{value:new THREE.Color(ACTIVE_COLOR)},uRadius:{value:1.5}}
    });
    const quad=new THREE.Mesh(new THREE.PlaneGeometry(2,2),this.outline);quad.frustumCulled=false;this.quadScene.add(quad);
  }
  /** Shared with the transform gizmo so handles draw in the same pass and camera. */
  get scene():THREE.Scene {return this.gizmoScene;}

  private line(color:string,depth:boolean):THREE.LineBasicMaterial {
    const key=`${color}:${depth}`;let material=this.materials.get(key);
    if(!material){material=new THREE.LineBasicMaterial({color,depthTest:depth,depthWrite:false,transparent:true});this.materials.set(key,material);}
    return material;
  }
  private clearLines(){for(const child of [...this.lineScene.children]){(child as THREE.LineSegments).geometry.dispose();child.removeFromParent();}}
  private segments(points:THREE.Vector3[],material:THREE.Material){
    const geometry=new THREE.BufferGeometry().setFromPoints(points),line=new THREE.LineSegments(geometry,material);
    line.frustumCulled=false;this.lineScene.add(line);
  }

  draw(frame:OverlayFrame):void {
    if(this.disposed)return;
    const width=Math.max(2,this.stage.clientWidth),height=Math.max(2,this.stage.clientHeight),renderer=this.renderer;
    renderer.setSize(width,height,false);
    renderer.setScissorTest(false);renderer.setClearColor(0,0);renderer.clear(true,true,true);
    const {rect}=frame,camera=frame.camera.clone() as THREE.PerspectiveCamera|THREE.OrthographicCamera;
    // Extend the frustum over the whole stage so handles stay visible past the composition edge.
    camera.setViewOffset(rect.width,rect.height,-rect.x,-rect.y,width,height);camera.updateProjectionMatrix();camera.updateMatrixWorld(true);
    const scissor=()=>{renderer.setScissorTest(true);renderer.setScissor(rect.x,height-rect.y-rect.height,rect.width,rect.height);};
    const runtime=frame.runtime;
    if(frame.overlays&&runtime){
      scissor();
      const scene=runtime.scene,background=scene.background;
      if(frame.userView&&!frame.xray){
        // Depth of the models, so the floor and wires pass behind them.
        scene.background=null;scene.overrideMaterial=this.depthMaterial;
        try{renderer.render(scene,camera);}finally{scene.overrideMaterial=null;scene.background=background;}
        this.drawGrid(frame,camera);
      }else if(frame.userView)this.drawGrid(frame,camera);
      this.drawOutlines(frame,camera,width,height,scissor);
      this.clearLines();
      for(const glyph of frame.glyphs)this.drawGlyph(glyph,frame,camera);
      if(frame.edit)this.drawEdit(frame.edit);
      scissor();renderer.render(this.lineScene,camera);
    }
    if(frame.guides.length){
      this.clearLines();
      for(const guide of frame.guides){
        const reach=Math.max(1e3,frame.distance*100);
        this.segments([guide.origin.clone().addScaledVector(guide.direction,-reach),guide.origin.clone().addScaledVector(guide.direction,reach)],this.line(AXIS_COLORS[guide.axis],false));
      }
      scissor();renderer.clearDepth();renderer.render(this.lineScene,camera);
    }
    renderer.setScissorTest(false);
    if(frame.gizmo){renderer.clearDepth();renderer.render(this.gizmoScene,camera);}
  }

  private drawGrid(frame:OverlayFrame,camera:THREE.Camera):void {
    const named=frame.axis&&camera instanceof THREE.OrthographicCamera?frame.axis:null;
    const kind=named==='front'||named==='back'?'front':named==='right'||named==='left'?'side':'floor',plane=PLANES[kind]!;
    const half=frame.distance*Math.tan(THREE.MathUtils.degToRad(frame.fov)/2);
    const step=Math.pow(10,Math.floor(Math.log10(Math.max(1e-4,half/2))));
    const forward=new THREE.Vector3(0,0,-1).transformDirection(camera.matrixWorld);
    const target=new THREE.Vector3().setFromMatrixPosition(camera.matrixWorld).addScaledVector(forward,frame.distance);
    const center=target.clone();center[kind==='floor'?'y':kind==='front'?'z':'x']=0;
    const fade=named?half*Math.max(4,frame.rect.width/Math.max(1,frame.rect.height)*3):Math.max(frame.distance*6,step*40);
    const uniforms=this.grid.material.uniforms;
    uniforms.uStep!.value=step;uniforms.uCenter!.value.copy(center);uniforms.uFade!.value=fade;uniforms.uPlane!.value=plane.plane;uniforms.uFar!.value=named?1:0;
    uniforms.uAxisA!.value.set(AXIS_COLORS[plane.a]);uniforms.uAxisB!.value.set(AXIS_COLORS[plane.b]);
    plane.rotate(this.grid);this.grid.position.copy(center);this.grid.scale.setScalar(fade*2.2);this.grid.updateMatrixWorld(true);
    this.renderer.render(this.gridScene,camera);
  }

  private drawOutlines(frame:OverlayFrame,camera:THREE.Camera,width:number,height:number,scissor:()=>void):void {
    const runtime=frame.runtime!;if(!frame.selected.size)return;
    const ratio=this.renderer.getPixelRatio(),w=Math.max(2,Math.round(width*ratio)),h=Math.max(2,Math.round(height*ratio));
    if(!this.mask||this.mask.width!==w||this.mask.height!==h){this.mask?.dispose();this.mask=new THREE.WebGLRenderTarget(w,h,{depthBuffer:false});}
    const marked:THREE.Object3D[]=[];
    runtime.scene.traverse((child:any)=>{
      if(!child.isMesh)return;const id=child.userData.sceneId;
      if(frame.active.has(id)){child.layers.enable(2);marked.push(child);}
      else if(frame.selected.has(id)){child.layers.enable(1);marked.push(child);}
    });
    if(!marked.length)return;
    const scene=runtime.scene,background=scene.background,previous=this.renderer.getRenderTarget();
    try{
      scene.background=null;scene.overrideMaterial=this.maskMaterial;
      this.renderer.setRenderTarget(this.mask);this.renderer.setScissorTest(false);this.renderer.setClearColor(0,0);this.renderer.clear(true,false,false);
      camera.layers.set(1);this.maskMaterial.color.setRGB(1,0,0);this.renderer.render(scene,camera);
      camera.layers.set(2);this.maskMaterial.color.setRGB(0,1,0);this.renderer.render(scene,camera);
    }finally{
      camera.layers.set(0);scene.overrideMaterial=null;scene.background=background;this.renderer.setRenderTarget(previous);
      for(const child of marked){child.layers.disable(1);child.layers.disable(2);}
    }
    this.outline.uniforms.uMask!.value=this.mask.texture;this.outline.uniforms.uTexel!.value.set(1/w,1/h);
    this.outline.uniforms.uRadius!.value=ratio>=2?2:1.5;
    scissor();this.renderer.render(this.quadScene,this.quadCamera);
  }

  /** Edit Mode: edges and points over everything, selected ones in orange like Blender. */
  private drawEdit(edit:NonNullable<OverlayFrame['edit']>):void {
    const plain:THREE.Vector3[]=[],chosen:THREE.Vector3[]=[];
    for(const [a,b] of edit.edges)(edit.selected[a]&&edit.selected[b]?chosen:plain).push(edit.points[a]!,edit.points[b]!);
    if(plain.length)this.segments(plain,this.line('#d9d9d9',false));
    if(chosen.length)this.segments(chosen,this.line(SELECTED_COLOR,false));
    const geometry=new THREE.BufferGeometry().setFromPoints(edit.points),colors:number[]=[],plainColor=new THREE.Color('#111111'),pick=new THREE.Color(SELECTED_COLOR);
    for(const selected of edit.selected)colors.push(...(selected?pick:plainColor).toArray());
    geometry.setAttribute('color',new THREE.Float32BufferAttribute(colors,3));
    this.pointMaterial??=new THREE.PointsMaterial({size:6,sizeAttenuation:false,vertexColors:true,depthTest:false,transparent:true});
    const points=new THREE.Points(geometry,this.pointMaterial);points.frustumCulled=false;this.lineScene.add(points);
  }
  private pointMaterial:THREE.PointsMaterial|null=null;
  /** World units per screen pixel at `point`, so wires keep a constant on-screen size. */
  private perPixel(point:THREE.Vector3,frame:OverlayFrame):number {
    const camera=frame.camera;
    if(camera instanceof THREE.OrthographicCamera)return (camera.top-camera.bottom)/camera.zoom/Math.max(1,frame.rect.height);
    const depth=Math.max(1e-3,point.clone().applyMatrix4(camera.matrixWorldInverse).z*-1);
    return 2*depth*Math.tan(THREE.MathUtils.degToRad(camera.fov)/2)/Math.max(1,frame.rect.height);
  }
  private drawGlyph(glyph:Glyph,frame:OverlayFrame,camera:THREE.Camera):void {
    const color=glyph.state==='active'?ACTIVE_COLOR:glyph.state==='selected'?SELECTED_COLOR:WIRE_COLOR;
    const material=this.line(color,frame.userView),p=glyph.position,unit=this.perPixel(p,frame);
    const q=new THREE.Quaternion();frame.camera.matrixWorld.decompose(new THREE.Vector3(),q,new THREE.Vector3());
    const right=new THREE.Vector3(1,0,0).applyQuaternion(q),up=new THREE.Vector3(0,1,0).applyQuaternion(q);
    const points:THREE.Vector3[]=[];
    const circle=(center:THREE.Vector3,radius:number,a=right,b=up,steps=24)=>{
      for(let i=0;i<steps;i++){
        const t0=i/steps*Math.PI*2,t1=(i+1)/steps*Math.PI*2;
        points.push(center.clone().addScaledVector(a,Math.cos(t0)*radius).addScaledVector(b,Math.sin(t0)*radius),
          center.clone().addScaledVector(a,Math.cos(t1)*radius).addScaledVector(b,Math.sin(t1)*radius));
      }
    };
    if(glyph.kind==='camera'){
      const target=glyph.target&&glyph.target.distanceToSquared(p)>1e-12?glyph.target:p.clone().add(new THREE.Vector3(0,0,-1));
      const look=new THREE.Matrix4().lookAt(p,target,glyph.up||new THREE.Vector3(0,1,0));
      const forward=new THREE.Vector3(0,0,-1).applyMatrix4(look).normalize(),cr=new THREE.Vector3(1,0,0).applyMatrix4(look).normalize(),cu=new THREE.Vector3(0,1,0).applyMatrix4(look).normalize();
      const depth=unit*70,halfH=depth*Math.tan(THREE.MathUtils.degToRad(glyph.fov??50)/2),halfW=halfH*(glyph.aspect??16/9);
      const center=p.clone().addScaledVector(forward,depth);
      const corners=[[-1,-1],[1,-1],[1,1],[-1,1]].map(([x,y])=>center.clone().addScaledVector(cr,x!*halfW).addScaledVector(cu,y!*halfH));
      for(let i=0;i<4;i++)points.push(corners[i]!,corners[(i+1)%4]!,p,corners[i]!);
      const top=center.clone().addScaledVector(cu,halfH*1.12),tri=halfW*.35;
      points.push(top.clone().addScaledVector(cr,-tri),top.clone().addScaledVector(cr,tri),top.clone().addScaledVector(cr,tri),top.clone().addScaledVector(cu,tri*.9),
        top.clone().addScaledVector(cu,tri*.9),top.clone().addScaledVector(cr,-tri));
      this.segments(points,material);return;
    }
    if(glyph.kind==='area'){
      // Blender draws the lamp's real rectangle and the direction it shines.
      const target=glyph.target&&glyph.target.distanceToSquared(p)>1e-12?glyph.target:p.clone().add(new THREE.Vector3(0,-1,0));
      const look=new THREE.Matrix4().lookAt(p,target,Math.abs(target.clone().sub(p).normalize().y)>.99?new THREE.Vector3(0,0,1):new THREE.Vector3(0,1,0));
      const ax=new THREE.Vector3(1,0,0).applyMatrix4(look).normalize(),ay=new THREE.Vector3(0,1,0).applyMatrix4(look).normalize(),forward=new THREE.Vector3(0,0,-1).applyMatrix4(look).normalize();
      const [w,h]=glyph.size??[1,1],corners=[[-1,-1],[1,-1],[1,1],[-1,1]].map(([x,y])=>p.clone().addScaledVector(ax,x!*w/2).addScaledVector(ay,y!*h/2));
      for(let i=0;i<4;i++)points.push(corners[i]!,corners[(i+1)%4]!);
      points.push(p.clone(),p.clone().addScaledVector(forward,Math.max(unit*60,Math.min(w,h))));
      this.segments(points,material);return;
    }
    const radius=unit*9;
    circle(p,radius);
    if(glyph.kind==='point')circle(p,radius*.35,right,up,12);
    const aim=glyph.target&&glyph.target.distanceToSquared(p)>1e-12?glyph.target.clone().sub(p).normalize():new THREE.Vector3(0,-1,0);
    if(glyph.kind==='sun'){
      for(let i=0;i<8;i++){const t=i/8*Math.PI*2,d=right.clone().multiplyScalar(Math.cos(t)).addScaledVector(up,Math.sin(t));points.push(p.clone().addScaledVector(d,radius*1.45),p.clone().addScaledVector(d,radius*2));}
      points.push(p.clone(),p.clone().addScaledVector(aim,unit*80));
    }
    if(glyph.kind==='spot'){
      const length=Math.min(glyph.target?glyph.target.distanceTo(p):unit*80,unit*90),angle=THREE.MathUtils.degToRad(Math.min(80,glyph.angle??30));
      const side=Math.abs(aim.y)>.99?new THREE.Vector3(1,0,0):new THREE.Vector3(0,1,0),a=new THREE.Vector3().crossVectors(aim,side).normalize(),b=new THREE.Vector3().crossVectors(aim,a).normalize();
      const end=p.clone().addScaledVector(aim,length),ring=Math.tan(angle)*length;
      for(let i=0;i<4;i++){const t=i/4*Math.PI*2;points.push(p.clone(),end.clone().addScaledVector(a,Math.cos(t)*ring).addScaledVector(b,Math.sin(t)*ring));}
      circle(end,ring,a,b,32);
    }
    this.segments(points,material);
  }

  dispose():void {
    if(this.disposed)return;this.disposed=true;
    this.clearLines();this.grid.geometry.dispose();this.grid.material.dispose();this.depthMaterial.dispose();this.maskMaterial.dispose();
    this.outline.dispose();this.mask?.dispose();this.pointMaterial?.dispose();for(const material of this.materials.values())material.dispose();
    this.renderer.dispose();this.renderer.forceContextLoss();this.canvas.remove();
  }
}
