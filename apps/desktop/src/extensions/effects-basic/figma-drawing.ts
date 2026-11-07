import { BLEND_GLSL } from "./blend-glsl";
import type { EffectDefinition, EffectParamDefinition } from "powermove";
import { SPATIAL_PRE } from "./spatial";
const number = (
  k: string,
  def: number,
  min = -100000,
  max = 100000,
): EffectParamDefinition => ({
  k,
  label: k,
  def,
  min,
  max,
  step: 0.1,
  advanced: true,
});
const color = (k: string, def: string): EffectParamDefinition => ({
  k,
  label: k,
  def,
  type: "color",
  advanced: true,
});
const parameters = [
  color("color", "#000000"),
  number("opacity", 50, 0, 100),
  number("offsetX", 0),
  number("offsetY", 4),
  number("radius", 4, 0),
  number("spread", 0),
  number("blendMode", 0, 0, 18),
  {
    k: "inner",
    label: "inner",
    def: false,
    type: "toggle",
    advanced: true,
  } as EffectParamDefinition,
];
/** Separate from historical Drop Shadow versions. Each shadow takes its alpha
 * from the layer's unprocessed source, never from the previous shadow. */
const shadow: EffectDefinition = {
  id: "figmaShadow",
  label: "Shape Shadow",
  group: "Light & Shadow",
  version: 1,
  params: parameters,
  passes: 5,
  keepOrig: true,
  backdrop: true,
  rawShader: true,
  viewportSafe: true,
  viewportPadding: ["radius", "spread", "offsetX", "offsetY"],
  viewportPaddingFactor: 3,
  frag:
    SPATIAL_PRE +
    BLEND_GLSL +
    `
 uniform sampler2D u_sourceShape,u_backdrop;
 uniform float u_blendMode;
 uniform vec3 u_color;
 uniform float u_opacity,u_offsetX,u_offsetY,u_radius,u_spread,u_inner;
 float alphaAt(vec2 pixel){vec2 uv=(pixel-u_pxOrigin)/u_sourceRes;float a=inside(uv)?texture(u_sourceShape,vec2(uv.x,1.-uv.y)).a:0.;return u_inner>.5?1.-a:a;}
 void main(){
   mat3 inv=inverse(u_layerUV);vec2 uv=layerUV(v_px),axis=u_pass==0||u_pass==2?vec2(1.,0.):vec2(0.,1.);
   if(u_pass<2){
     if(u_pass==1)uv-=vec2(u_offsetX,u_offsetY)/u_layerRes;
     float a=u_spread>=0.?0.:1.,r=abs(u_spread);
     for(int i=-32;i<=32;i++){float delta=clamp(float(i),-r,r);if(r>32.)delta=float(i)*r/32.;vec2 px=sourcePixel(uv+axis*delta/u_layerRes,inv);float sampleAlpha=u_pass==0?alphaAt(px):samplePixel(px,0.).a;a=u_spread>=0.?max(a,sampleAlpha):min(a,sampleAlpha);}
     o=vec4(0.,0.,0.,a);return;
   }
   if(u_pass<4){float sigma=max(0.,u_radius)*.5,sum=0.,total=0.;if(sigma<.01){o=texture(u_tex,v_st);return;}
     float stride=max(1.,3.*sigma/32.);for(int i=-32;i<=32;i++){float delta=float(i)*stride;if(abs(delta)>3.*sigma)continue;float weight=exp(-.5*delta*delta/(sigma*sigma));sum+=samplePixel(sourcePixel(uv+axis*delta/u_layerRes,inv),0.).a*weight;total+=weight;}o=vec4(0.,0.,0.,sum/max(total,1e-8));return;
   }
   vec4 original=texture(u_orig,v_st);float coverage=texture(u_sourceShape,v_st).a;
   float a=texture(u_tex,v_st).a*u_opacity/100.;
   if(u_inner>.5){a*=coverage;vec3 color=blend(original.a>0.?original.rgb/original.a:vec3(0.),u_color,int(u_blendMode));o=vec4(color*a,a)+original*(1.-a);}
   else{vec4 backdrop=texture(u_backdrop,v_st);vec3 color=blend(backdrop.a>0.?backdrop.rgb/backdrop.a:vec3(0.),u_color,int(u_blendMode));o=original+vec4(color*a,a)*(1.-original.a);}
 }
`,
};
const backdropLegacy: EffectDefinition = {
  id: "figmaBackdropBlur",
  label: "Background Blur",
  group: "Blur & Sharpen",
  version: 1,
  params: [
    number("radius", 6, 0),
    number("progressive", 0, 0, 1),
    number("startRadius", 0, 0),
    number("startX", 0),
    number("startY", 0),
    number("endX", 1),
    number("endY", 0),
  ],
  passes: 2,
  keepOrig: true,
  backdrop: true,
  rawShader: true,
  viewportSafe: false,
  frag:
    SPATIAL_PRE +
    `
 uniform sampler2D u_backdrop;
 uniform float u_radius,u_progressive,u_startRadius,u_startX,u_startY,u_endX,u_endY;
 vec4 sampleBackdrop(vec2 pixel){vec2 uv=(pixel-u_pxOrigin)/u_sourceRes;return inside(uv)?texture(u_backdrop,vec2(uv.x,1.-uv.y)):vec4(0.);}
 void main(){vec2 delta=vec2(u_endX-u_startX,u_endY-u_startY);float t=clamp(dot(layerUV(v_px)-vec2(u_startX,u_startY),delta)/max(dot(delta,delta),1e-8),0.,1.);float sigma=max(.001,(u_progressive>.5?mix(u_startRadius,u_radius,t):u_radius)*.5),sum=0.;vec4 color=vec4(0.);vec2 axis=u_pass==0?vec2(1.,0.):vec2(0.,1.);mat3 inv=inverse(u_layerUV);vec2 uv=layerUV(v_px);float stride=max(1.,3.*sigma/32.);
 for(int i=-32;i<=32;i++){float d=float(i)*stride;if(abs(d)>sigma*3.)continue;float weight=exp(-.5*d*d/(sigma*sigma));vec2 px=sourcePixel(uv+axis*d/u_layerRes,inv);color+=(u_pass==0?sampleBackdrop(px):samplePixel(px,0.))*weight;sum+=weight;}
 color/=max(sum,1e-8);if(u_pass==0){o=color;return;}vec4 original=texture(u_orig,v_st);float mask=texture(u_sourceShape,v_st).a;o=color*mask;
 }`.replace(
      "uniform sampler2D u_backdrop;",
      "uniform sampler2D u_backdrop; uniform sampler2D u_sourceShape;",
    ),
};
// Native Figma exports clamp the background sample at the blur owner's box
// and use a finite radius (two sigma). Retain revision 1 for saved artwork.
const backdrop: EffectDefinition = {
  ...backdropLegacy,
  version: 2,
  legacy: [backdropLegacy],
  frag: backdropLegacy.frag
    .replace("3.*sigma/32.", "2.*sigma/32.")
    .replace("abs(d)>sigma*3.", "abs(d)>sigma*2.")
    .replace("uv+axis*d/u_layerRes,inv", "clamp(uv+axis*d/u_layerRes,.5/u_layerRes,1.-.5/u_layerRes),inv"),
};
const noise: EffectDefinition = {
  id: "figmaNoise",
  label: "Shape Noise",
  group: "Stylize",
  version: 1,
  params: [
    number("size", 1, 0.01),
    number("density", 1, 0, 1),
    color("color", "#000000"),
    color("secondaryColor", "#FFFFFF"),
    number("opacity", 100, 0, 100),
    number("noiseType", 0, 0, 2),
    number("blendMode", 0, 0, 18),
  ],
  passes: 1,
  rawShader: true,
  viewportSafe: true,
  viewportPadding: [],
  frag:
    SPATIAL_PRE +
    BLEND_GLSL +
    `
 uniform float u_blendMode;
 uniform float u_size,u_density,u_opacity,u_noiseType;uniform vec3 u_color,u_secondaryColor;
 float hash(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}
 void main(){vec4 c=texture(u_tex,v_st);vec2 p=floor(layerUV(v_px)*u_layerRes/max(.01,u_size));float h=hash(p),a=step(h,u_density)*u_opacity/100.*c.a;
 vec3 color=u_noiseType>1.5?vec3(hash(p+4.),hash(p+9.),hash(p+17.)):u_noiseType>.5?mix(u_color,u_secondaryColor,step(.5,hash(p+3.))):u_color;color=blend(c.a>0.?c.rgb/c.a:vec3(0.),color,int(u_blendMode));o=vec4(color*a,a)+c*(1.-a);}
`,
};
const layerBlur: EffectDefinition = {
  ...backdropLegacy,
  id: "figmaLayerBlur",
  viewportSafe: true,
  viewportPadding: ["radius", "startRadius"],
  viewportPaddingFactor: 2,
  label: "Progressive Blur",
  backdrop: false,
  frag: backdropLegacy.frag
    .replace(
      "u_pass==0?sampleBackdrop(px):samplePixel(px,0.)",
      "samplePixel(px,0.)",
    )
    .replace("o=color*mask;", "o=color;"),
};
export const FIGMA_DRAWING_EFFECTS = [shadow, backdrop, layerBlur, noise];
