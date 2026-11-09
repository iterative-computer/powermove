import * as THREE from 'three';
import { splatVertexShader, splatFragmentShader, type SplatMesh } from '../scene3d/splats';

// Studio's Gaussian pass. It draws EWA ellipses directly through WebGL2; the
// three.js graph supplies only transforms and data, never a WebGLRenderer.
const depthVertex=`#version 300 es
in vec3 position;uniform mat4 modelViewMatrix;uniform mat4 projectionMatrix;
void main(){gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.0);}`;
const depthFragment=`#version 300 es
precision highp float;out vec4 color;void main(){color=vec4(0.0);}`;
type Geometry={positions:WebGLBuffer;indices:WebGLBuffer|null;positionVersion:number;indexVersion:number;count:number};
class SplatRaster {
  canvas=document.createElement('canvas');
  gl:WebGL2RenderingContext;
  splat:WebGLProgram;depth:WebGLProgram;
  quad:WebGLBuffer;indices:WebGLBuffer;order:WebGLBuffer;
  textures=new WeakMap<object,WebGLTexture>();ownedTextures=new Set<WebGLTexture>();
  geometry=new WeakMap<THREE.BufferGeometry,Geometry>();ownedBuffers=new Set<WebGLBuffer>();
  constructor(){
    const gl=this.canvas.getContext('webgl2',{alpha:true,premultipliedAlpha:true,antialias:false,preserveDrawingBuffer:true});
    if(!gl)throw new Error('Studio could not start the Gaussian raster pass');this.gl=gl;
    const program=(vs:string,fs:string)=>{const p=gl.createProgram()!;for(const [type,source] of [[gl.VERTEX_SHADER,vs],[gl.FRAGMENT_SHADER,fs]] as const){const shader=gl.createShader(type)!;gl.shaderSource(shader,source);gl.compileShader(shader);if(!gl.getShaderParameter(shader,gl.COMPILE_STATUS))throw new Error(gl.getShaderInfoLog(shader) || 'Gaussian shader failed');gl.attachShader(p,shader);gl.deleteShader(shader);}gl.linkProgram(p);if(!gl.getProgramParameter(p,gl.LINK_STATUS))throw new Error(gl.getProgramInfoLog(p)||'Gaussian shader failed');return p;};
    this.splat=program('#version 300 es\n'+splatVertexShader,'#version 300 es\n'+splatFragmentShader);this.depth=program(depthVertex,depthFragment);
    this.quad=gl.createBuffer()!;this.indices=gl.createBuffer()!;this.order=gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER,this.quad);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([-2,-2,0,2,-2,0,2,2,0,-2,2,0]),gl.STATIC_DRAW);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,this.indices);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,new Uint16Array([0,1,2,0,2,3]),gl.STATIC_DRAW);
  }
  private matrix(program:WebGLProgram,name:string,matrix:THREE.Matrix4){this.gl.uniformMatrix4fv(this.gl.getUniformLocation(program,name),false,matrix.elements);}
  private attribute(program:WebGLProgram,name:string,buffer:WebGLBuffer,size:number,divisor=0){const gl=this.gl,at=gl.getAttribLocation(program,name);gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.enableVertexAttribArray(at);gl.vertexAttribPointer(at,size,gl.FLOAT,false,0,0);gl.vertexAttribDivisor(at,divisor);}
  render(scene:THREE.Scene,camera:THREE.Camera,layer:string,ids:Set<string>,w:number,h:number,background:boolean):HTMLCanvasElement {
    const gl=this.gl;if(this.canvas.width!==w)this.canvas.width=w;if(this.canvas.height!==h)this.canvas.height=h;
    gl.viewport(0,0,w,h);const color=background && scene.background instanceof THREE.Color?scene.background.clone().convertLinearToSRGB():null;gl.clearColor(color?.r ?? 0,color?.g ?? 0,color?.b ?? 0,color?1:0);gl.depthMask(true);gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);gl.enable(gl.DEPTH_TEST);gl.depthFunc(gl.LEQUAL);gl.disable(gl.CULL_FACE);gl.disable(gl.BLEND);
    scene.updateMatrixWorld(true);camera.updateMatrixWorld(true);const mv=new THREE.Matrix4();
    // Opaque meshes hide splats behind them. This pass writes depth only;
    // Studio supplies all visible mesh shading, reflections and shadows.
    gl.useProgram(this.depth);gl.colorMask(false,false,false,false);this.matrix(this.depth,'projectionMatrix',camera.projectionMatrix);
    scene.traverseVisible((o:any)=>{
      const id=o.userData.sceneId,material=o.material;if(!o.isMesh || o.userData.splat || !ids.has(id) || o.userData.missing)return;
      let root=o;while(root.parent && root.parent.userData.sceneId===id)root=root.parent;
      if((root.userData.layerOpacity ?? 1)<.999 || root.userData.layerOccludes===false)return;
      const mats=Array.isArray(material)?material:[material];if(!mats.some((m:any)=>m?.visible!==false && m.opacity>=.999))return;
      const g=o.geometry as THREE.BufferGeometry,p=g.getAttribute('position') as THREE.BufferAttribute,index=g.getIndex();if(!p)return;
      let entry=this.geometry.get(g);if(!entry){entry={positions:gl.createBuffer()!,indices:index?gl.createBuffer()!:null,positionVersion:-1,indexVersion:-1,count:0};this.geometry.set(g,entry);this.ownedBuffers.add(entry.positions);if(entry.indices)this.ownedBuffers.add(entry.indices);}
      if(entry.positionVersion!==p.version){gl.bindBuffer(gl.ARRAY_BUFFER,entry.positions);gl.bufferData(gl.ARRAY_BUFFER,p.array as Float32Array,gl.STATIC_DRAW);entry.positionVersion=p.version;}
      this.attribute(this.depth,'position',entry.positions,3);this.matrix(this.depth,'modelViewMatrix',mv.multiplyMatrices(camera.matrixWorldInverse,o.matrixWorld));
      if(index){gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,entry.indices);if(entry.indexVersion!==index.version){gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,new Uint32Array(index.array),gl.STATIC_DRAW);entry.indexVersion=index.version;}gl.drawElements(gl.TRIANGLES,index.count,gl.UNSIGNED_INT,0);}
      else gl.drawArrays(gl.TRIANGLES,0,p.count);
    });
    gl.colorMask(true,true,true,true);gl.depthMask(false);gl.enable(gl.BLEND);gl.blendFunc(gl.ONE,gl.ONE_MINUS_SRC_ALPHA);gl.useProgram(this.splat);
    this.matrix(this.splat,'projectionMatrix',camera.projectionMatrix);gl.uniform2f(gl.getUniformLocation(this.splat,'viewport'),w,h);
    this.attribute(this.splat,'position',this.quad,3);this.attribute(this.splat,'splatIndex',this.order,1,1);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,this.indices);
    scene.traverseVisible((o:any)=>{
      if(!o.userData.splat || o.userData.sceneId!==layer || !o.data)return;
      const mesh=o as SplatMesh,view=mv.multiplyMatrices(camera.matrixWorldInverse,mesh.matrixWorld);
      this.matrix(this.splat,'modelViewMatrix',view);gl.bindBuffer(gl.ARRAY_BUFFER,this.order);gl.bufferData(gl.ARRAY_BUFFER,mesh.orderFor(view),gl.DYNAMIC_DRAW);
      ['tCenter','tCovA','tCovB','tColor'].forEach((name,unit)=>{
        const original=mesh.material.uniforms[name]!.value as THREE.DataTexture,image=original.image;
        gl.activeTexture(gl.TEXTURE0+unit);
        let texture=this.textures.get(original);if(!texture){texture=gl.createTexture()!;this.textures.set(original,texture);this.ownedTextures.add(texture);gl.bindTexture(gl.TEXTURE_2D,texture);const bytes=image.data instanceof Uint8Array;gl.texImage2D(gl.TEXTURE_2D,0,bytes?gl.RGBA8:gl.RGBA32F,image.width,image.height,0,gl.RGBA,bytes?gl.UNSIGNED_BYTE:gl.FLOAT,image.data as ArrayBufferView);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);}
        gl.activeTexture(gl.TEXTURE0+unit);gl.bindTexture(gl.TEXTURE_2D,texture);gl.uniform1i(gl.getUniformLocation(this.splat,name),unit);if(unit===0)gl.uniform1i(gl.getUniformLocation(this.splat,'width'),image.width);
      });
      gl.drawElementsInstanced(gl.TRIANGLES,6,gl.UNSIGNED_SHORT,0,mesh.data!.count);
    });
    gl.vertexAttribDivisor(gl.getAttribLocation(this.splat,'splatIndex'),0);gl.depthMask(true);gl.disable(gl.BLEND);return this.canvas;
  }
  dispose(){const gl=this.gl;for(const b of [this.quad,this.indices,this.order,...this.ownedBuffers])gl.deleteBuffer(b);for(const t of this.ownedTextures)gl.deleteTexture(t);gl.deleteProgram(this.splat);gl.deleteProgram(this.depth);gl.getExtension('WEBGL_lose_context')?.loseContext();}
}
const rasters=new WeakMap<object,SplatRaster>();
export function studioSplatLayer(runtime:any,camera:THREE.Camera,layer:string,ids:Set<string>,w:number,h:number,background=false):HTMLCanvasElement|null {
  let found=false;runtime.scene.traverseVisible((o:any)=>{if(o.userData.splat && o.userData.sceneId===layer)found=true;});if(!found)return null;
  let raster=rasters.get(runtime);if(!raster){raster=new SplatRaster();rasters.set(runtime,raster);}return raster.render(runtime.scene,camera,layer,ids,w,h,background);
}
export function disposeStudioSplats(runtime:object):void{rasters.get(runtime)?.dispose();rasters.delete(runtime);}
