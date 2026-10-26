import type { EffectDefinition, EffectParamDefinition } from 'powermove';

const number = (k: string, label: string, def: number, min: number, max: number, unit = '', step = 1): EffectParamDefinition =>
  ({ k, label, def, min, max, unit, step, advanced: false });
const toggle = (k: string, label: string, def = false): EffectParamDefinition => ({ k, label, def, type: 'toggle', advanced: false });
const directions: EffectParamDefinition = { ...number('dimensions','Blur Dimensions',0,0,2), options: [
  { v: 0, label: 'Horizontal and Vertical' }, { v: 1, label: 'Horizontal' }, { v: 2, label: 'Vertical' },
] };
const PRE = `#version 300 es
precision highp float;
in vec2 v_st;
in vec2 v_px;
uniform sampler2D u_tex;
uniform sampler2D u_orig;
uniform vec2 u_res, u_sourceRes, u_pxOrigin, u_pixelScale, u_layerRes, u_gridRes;
uniform mat3 u_layerUV;
uniform float u_sampleLod;
uniform int u_pass;
out vec4 o;
vec2 layerUV(vec2 pixel) { vec3 p=u_layerUV*vec3(pixel,1.); return p.xy/p.z; }
bool inside(vec2 uv) { return all(greaterThanEqual(uv,vec2(0.))) && all(lessThanEqual(uv,vec2(1.))); }
vec2 sourcePixel(vec2 uv,mat3 inverseLayer) { vec3 p=inverseLayer*vec3(uv,1.); return p.xy/p.z; }
vec4 samplePixel(vec2 pixel,float lod) {
  vec2 uv=(pixel-u_pxOrigin)/u_sourceRes;
  if(!inside(uv)) return vec4(0.);
  return textureLod(u_tex,vec2(uv.x,1.-uv.y),lod);
}
`;
const uniforms = (params: EffectParamDefinition[]) => params.map(param => `uniform float u_${param.k};`).join('\n');
const gaussian = `
vec4 gaussian(float sigma,bool repeatEdges,mat3 inverseLayer) {
  float stride=exp2(u_sampleLod); int taps=int(clamp(ceil(3.*sigma/stride),1.,32.));
  vec2 direction=u_pass==0 ? vec2(1.,0.) : vec2(0.,1.);
  vec4 sum=vec4(0.); float total=0.;
  for(int i=-32;i<=32;i++) {
    if(abs(i)>taps) continue;
    float distance=float(i)*stride;
    float weight=exp(-.5*distance*distance/max(sigma*sigma,1e-8));
    vec2 pixel=v_px+direction*distance;
    if(repeatEdges) pixel=sourcePixel(clamp(layerUV(pixel),vec2(0.),vec2(1.)),inverseLayer);
    sum+=samplePixel(pixel,u_sampleLod)*weight; total+=weight;
  }
  return sum/max(total,1e-8);
}
`;
const blurParams=[number('amount','Blurriness',0,0,400,'px',.1),directions,toggle('repeatEdgePixels','Repeat Edge Pixels')];
const blur: EffectDefinition={
  id:'blur',label:'Gaussian Blur',group:'Blur & Sharpen',passes:2,params:blurParams,
  rawShader:true,viewportSafe:true,viewportPadding:['amount'],viewportPaddingFactor:2,
  sampling:{radius:'amount',support:1.5,maxRadius:32},
  frag:PRE+uniforms(blurParams)+gaussian+`
void main() {
  bool inactive=(u_dimensions>.5&&u_dimensions<1.5&&u_pass==1)||(u_dimensions>1.5&&u_pass==0);
  float scale=u_pass==0 ? u_pixelScale.x : u_pixelScale.y;
  float sigma=max(0.,u_amount)*.5*scale;
  if(sigma<.01||inactive){o=texture(u_tex,v_st);return;}
  bool repeatEdges=u_repeatEdgePixels>.5;
  if(repeatEdges&&!inside(layerUV(v_px))){o=vec4(0.);return;}
  o=gaussian(sigma,repeatEdges,inverse(u_layerUV));
}`,
};

const mosaicParams=[number('horizontalBlocks','Horizontal Blocks',10,1,2048),number('verticalBlocks','Vertical Blocks',10,1,2048),toggle('sharpColors','Sharp Colors')];
const mosaic: EffectDefinition={
  id:'pixelate',label:'Mosaic',group:'Stylize',passes:2,params:mosaicParams,
  grid:{columns:'horizontalBlocks',rows:'verticalBlocks',pointSample:'sharpColors'},
  rawShader:true,viewportSafe:false,viewportPadding:[],
  frag:PRE+uniforms(mosaicParams)+`
void main() {
  if(u_pass==0) {
    vec2 cell=floor(vec2(v_st.x,1.-v_st.y)*u_gridRes);
    vec2 center=(cell+.5)/u_gridRes;
    mat3 inverseLayer=inverse(u_layerUV);
    if(u_sharpColors>.5){o=samplePixel(sourcePixel(center,inverseLayer),0.);return;}
    vec2 dx=sourcePixel(center+vec2(.5/u_gridRes.x,0.),inverseLayer)-sourcePixel(center-vec2(.5/u_gridRes.x,0.),inverseLayer);
    vec2 dy=sourcePixel(center+vec2(0.,.5/u_gridRes.y),inverseLayer)-sourcePixel(center-vec2(0.,.5/u_gridRes.y),inverseLayer);
    vec2 extent=vec2(length(dx),length(dy));
    float lod=clamp(floor(log2(max(1.,min(extent.x,extent.y)/8.))),0.,u_sampleLod);
    vec2 samples=clamp(ceil(extent/exp2(lod)),vec2(1.),vec2(32.));
    vec4 sum=vec4(0.);
    for(int y=0;y<32;y++) {
      if(float(y)>=samples.y) break;
      for(int x=0;x<32;x++) {
        if(float(x)>=samples.x) break;
        vec2 uv=(cell+(vec2(float(x),float(y))+.5)/samples)/u_gridRes;
        sum+=samplePixel(sourcePixel(uv,inverseLayer),lod);
      }
    }
    o=sum/(samples.x*samples.y);return;
  }
  vec2 uv=layerUV(v_px);
  if(!inside(uv)){o=vec4(0.);return;}
  ivec2 cell=ivec2(clamp(floor(uv*u_gridRes),vec2(0.),u_gridRes-1.));
  o=texelFetch(u_tex,ivec2(cell.x,int(u_gridRes.y)-1-cell.y),0);
}`,
};

const unsharpParams=[number('amount','Amount',50,0,500,'%',.1),number('radius','Radius',1,0,400,'px',.1),number('threshold','Threshold',0,0,255)];
const unsharp: EffectDefinition={
  id:'unsharpMask',label:'Unsharp Mask',group:'Blur & Sharpen',passes:2,params:unsharpParams,
  keepOrig:true,sampling:{radius:'radius',support:3,maxRadius:32},
  rawShader:true,viewportSafe:true,viewportPadding:['radius'],viewportPaddingFactor:4,
  frag:PRE+uniforms(unsharpParams)+gaussian+`
void main() {
  vec4 original=texture(u_orig,v_st);
  float scale=u_pass==0 ? u_pixelScale.x : u_pixelScale.y;
  float sigma=max(0.,u_radius)*scale;
  if(sigma<.01||u_amount<=0.){o=original;return;}
  vec4 blurred=gaussian(sigma,false,mat3(1.));
  if(u_pass==0){o=blurred;return;}
  if(original.a<=0.){o=vec4(0.);return;}
  vec3 rgb=original.rgb/original.a;
  vec3 soft=blurred.a>0. ? blurred.rgb/blurred.a : vec3(0.);
  vec3 difference=rgb-soft;
  vec3 mask=step(vec3(u_threshold/255.),abs(difference));
  o=vec4((rgb+difference*mask*u_amount/100.)*original.a,original.a);
}`,
};

const edgeParams=[toggle('invert','Invert'),number('blendWithOriginal','Blend With Original',0,0,100,'%')];
const edges: EffectDefinition={
  id:'findEdges',label:'Find Edges',group:'Stylize',passes:1,params:edgeParams,
  rawShader:true,viewportSafe:true,viewportPadding:[],viewportPaddingPixels:1,
  frag:PRE+uniforms(edgeParams)+`
vec3 straightAt(vec2 offset) {
  vec4 c=samplePixel(v_px+offset*u_pixelScale,0.);
  return c.a>0. ? c.rgb/c.a : vec3(0.);
}
void main() {
  vec4 c=texture(u_tex,v_st);if(c.a<=0.){o=vec4(0.);return;}
  vec3 a=straightAt(vec2(-1.,-1.)), b=straightAt(vec2(0.,-1.)), d=straightAt(vec2(1.,-1.));
  vec3 e=straightAt(vec2(-1.,0.)), f=straightAt(vec2(1.,0.));
  vec3 g=straightAt(vec2(-1.,1.)), h=straightAt(vec2(0.,1.)), i=straightAt(vec2(1.,1.));
  vec3 gx=-a+d-2.*e+2.*f-g+i, gy=-a-2.*b-d+g+2.*h+i;
  vec3 edge=clamp(sqrt(gx*gx+gy*gy)*.25,0.,1.);
  vec3 mapped=u_invert>.5 ? edge : 1.-edge;
  o=vec4(mix(mapped,c.rgb/c.a,u_blendWithOriginal/100.)*c.a,c.a);
}`,
};

export function reviseSpatialEffects(original: EffectDefinition[]): EffectDefinition[] {
  return original.map(definition=>{
    const revised=definition.id==='blur' ? blur : definition.id==='pixelate' ? mosaic : undefined;
    return revised ? {...revised,version:2,legacy:[{...definition,version:definition.version??1}]} : definition;
  });
}
export const SPATIAL_EFFECTS: EffectDefinition[]=[unsharp,edges];

export { PRE as SPATIAL_PRE };
