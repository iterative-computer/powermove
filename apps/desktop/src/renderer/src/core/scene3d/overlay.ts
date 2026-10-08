import * as THREE from 'three';
import type {SceneRuntime} from './runtime';
import type {ViewAxis} from './viewport';
import {AXIS_COLORS,type Axis,type Rect} from './modal';

/*
 * Guides drawn over the composition in the editor only: in a free view the
 * composition frame (the plane 2D layers sit on) and a floor grid under it;
 * light and camera wires; and the axis line of a constrained move. Guides
 * never draw into the composition, so exports and captures are unaffected.
 */

export const WIRE_COLOR='#c8c8c8';
export type Glyph={id:string;kind:'point'|'sun'|'spot'|'area'|'camera';position:THREE.Vector3;target:THREE.Vector3|null;up?:THREE.Vector3;fov?:number;aspect?:number;angle?:number;size?:[number,number];state:'none'|'selected'};
export type Guide={origin:THREE.Vector3;direction:THREE.Vector3;axis:Axis};
export interface OverlayFrame {
  camera:THREE.PerspectiveCamera|THREE.OrthographicCamera;
  rect:Rect;
  runtime:SceneRuntime|null;
  /** Free view: draw the composition frame and floor grid. */
  userView:boolean;
  axis:ViewAxis|null;
  /** Pivot distance and field of view, for grid density. */
  distance:number;
  fov:number;
  overlays:boolean;
  /** Selection ink, matching 2D selection boxes. */
  ink:string;
  /** The composition frame's corners and the floor height, in scene units. */
  frame:THREE.Vector3[];
  floor:number;
  glyphs:Glyph[];
  guides:Guide[];
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
  private lineScene=new THREE.Scene();
  private materials=new Map<string,THREE.LineBasicMaterial>();
  private disposed=false;

  constructor(private stage:HTMLElement){
    this.renderer=new THREE.WebGLRenderer({alpha:true,antialias:true});
    this.renderer.setPixelRatio(Math.min(2,window.devicePixelRatio||1));
    this.renderer.autoClear=false;
    this.canvas=this.renderer.domElement;
    this.canvas.setAttribute('aria-hidden','true');this.canvas.dataset.sceneOverlay='';
    this.canvas.style.cssText='position:absolute;left:0;top:0;width:100%;height:100%;pointer-events:none;z-index:4';
    stage.append(this.canvas);
    this.grid=new THREE.Mesh(new THREE.PlaneGeometry(1,1),new THREE.ShaderMaterial({
      vertexShader:GRID_VERTEX,fragmentShader:GRID_FRAGMENT,transparent:true,depthWrite:false,side:THREE.DoubleSide,
      uniforms:{uStep:{value:1},uCenter:{value:new THREE.Vector3()},uFade:{value:50},uPlane:{value:0},uFar:{value:0},
        uAxisA:{value:new THREE.Color(AXIS_COLORS.x)},uAxisB:{value:new THREE.Color(AXIS_COLORS.z)}}
    }));
    this.grid.frustumCulled=false;this.gridScene.add(this.grid);
  }

  private line(color:string,depth:boolean,opacity=1):THREE.LineBasicMaterial {
    const key=`${color}:${depth}:${opacity}`;let material=this.materials.get(key);
    if(!material){material=new THREE.LineBasicMaterial({color,depthTest:depth,depthWrite:false,transparent:true,opacity});this.materials.set(key,material);}
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
    camera.setViewOffset(rect.width,rect.height,-rect.x,-rect.y,width,height);camera.updateProjectionMatrix();camera.updateMatrixWorld(true);
    renderer.setScissorTest(true);renderer.setScissor(rect.x,height-rect.y-rect.height,rect.width,rect.height);
    const runtime=frame.runtime;
    this.clearLines();
    if(frame.overlays&&runtime){
      if(frame.userView){
        // Depth of the models, so the floor passes behind them.
        const scene=runtime.scene,background=scene.background;
        scene.background=null;scene.overrideMaterial=this.depthMaterial;
        try{renderer.render(scene,camera);}finally{scene.overrideMaterial=null;scene.background=background;}
        this.drawGrid(frame,camera);
        const [a,b,c,d]=frame.frame;
        if(a&&b&&c&&d)this.segments([a,b,b,c,c,d,d,a],this.line(frame.ink,false,.55));
      }
      for(const glyph of frame.glyphs)this.drawGlyph(glyph,frame,camera);
    }
    for(const guide of frame.guides){
      const reach=Math.max(1e3,frame.distance*100);
      this.segments([guide.origin.clone().addScaledVector(guide.direction,-reach),guide.origin.clone().addScaledVector(guide.direction,reach)],this.line(AXIS_COLORS[guide.axis],false,.8));
    }
    if(this.lineScene.children.length)renderer.render(this.lineScene,camera);
    renderer.setScissorTest(false);
  }

  private drawGrid(frame:OverlayFrame,camera:THREE.Camera):void {
    const named=frame.axis&&camera instanceof THREE.OrthographicCamera?frame.axis:null;
    const kind=named==='front'||named==='back'?'front':named==='right'||named==='left'?'side':'floor',plane=PLANES[kind]!;
    const half=frame.distance*Math.tan(THREE.MathUtils.degToRad(frame.fov)/2);
    const step=Math.pow(10,Math.floor(Math.log10(Math.max(1e-4,half/2))));
    const forward=new THREE.Vector3(0,0,-1).transformDirection(camera.matrixWorld);
    const target=new THREE.Vector3().setFromMatrixPosition(camera.matrixWorld).addScaledVector(forward,frame.distance);
    const center=target.clone();
    if(kind==='floor')center.y=frame.floor;else center[kind==='front'?'z':'x']=0;
    const fade=named?half*Math.max(4,frame.rect.width/Math.max(1,frame.rect.height)*3):Math.max(frame.distance*6,step*40);
    const uniforms=this.grid.material.uniforms;
    uniforms.uStep!.value=step;uniforms.uCenter!.value.copy(center);uniforms.uFade!.value=fade;uniforms.uPlane!.value=plane.plane;uniforms.uFar!.value=named?1:0;
    uniforms.uAxisA!.value.set(AXIS_COLORS[plane.a]);uniforms.uAxisB!.value.set(AXIS_COLORS[plane.b]);
    plane.rotate(this.grid);this.grid.position.copy(center);this.grid.scale.setScalar(fade*2.2);this.grid.updateMatrixWorld(true);
    this.renderer.render(this.gridScene,camera);
  }

  /** World units per screen pixel at `point`, so wires keep a constant on-screen size. */
  private perPixel(point:THREE.Vector3,frame:OverlayFrame):number {
    const camera=frame.camera;
    if(camera instanceof THREE.OrthographicCamera)return (camera.top-camera.bottom)/camera.zoom/Math.max(1,frame.rect.height);
    const depth=Math.max(1e-3,point.clone().applyMatrix4(camera.matrixWorldInverse).z*-1);
    return 2*depth*Math.tan(THREE.MathUtils.degToRad(camera.fov)/2)/Math.max(1,frame.rect.height);
  }
  private drawGlyph(glyph:Glyph,frame:OverlayFrame,camera:THREE.Camera):void {
    const color=glyph.state==='none'?WIRE_COLOR:frame.ink;
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
      // An area light shows its real rectangle and the direction it shines.
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
    this.clearLines();this.grid.geometry.dispose();this.grid.material.dispose();this.depthMaterial.dispose();
    for(const material of this.materials.values())material.dispose();
    this.renderer.dispose();this.renderer.forceContextLoss();this.canvas.remove();
  }
}
