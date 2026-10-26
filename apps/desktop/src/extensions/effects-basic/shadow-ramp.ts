import type { EffectDefinition, EffectParamDefinition } from 'powermove';
import { SPATIAL_PRE } from './spatial';
const number = (k: string, label: string, def: number, min: number, max: number, unit = ''): Extract<EffectParamDefinition, { def: number }> =>
  ({ k, label, def, min, max, unit, step: .1, advanced: false });
const color = (k: string, label: string, def: string): EffectParamDefinition => ({ k, label, def, type: 'color', advanced: false });
const uniforms = (params: EffectParamDefinition[]) => params.map(p => `uniform ${p.type === 'color' ? 'vec3' : 'float'} u_${p.k};`).join('\n');
const shadowParams: EffectParamDefinition[] = [
  color('color','Shadow Color','#000000'),number('opacity','Opacity',50,0,100,'%'),number('angle','Direction',135,-360,360,'°'),
  number('distance','Distance',5,0,400,'px'),number('softness','Softness',0,0,400,'px'),
  {k:'shadowOnly',label:'Shadow Only',def:false,type:'toggle',advanced:false},
];
const shadow: EffectDefinition = {
  id:'shadow',label:'Drop Shadow',group:'Light & Shadow',params:shadowParams,passes:3,keepOrig:true,rawShader:true,
  viewportSafe:true,viewportPadding:['distance','softness'],viewportPaddingFactor:2,
  sampling:{radius:'softness',support:1.5,maxRadius:32,space:'layer',firstPass:1},
  frag:SPATIAL_PRE+uniforms(shadowParams)+`
float softenedAlpha() {
  float sigma=max(0.,u_softness)*.5;
  if(sigma<.01)return texture(u_tex,v_st).a;
  mat3 inverseLayer=inverse(u_layerUV);
  vec2 uv=layerUV(v_px),axis=u_pass==1 ? vec2(1.,0.) : vec2(0.,1.);
  vec2 projected=sourcePixel(uv+axis/u_layerRes,inverseLayer)-v_px;
  float stride=max(exp2(u_sampleLod)/max(length(projected),1e-6),3.*sigma/32.);
  int taps=int(clamp(ceil(3.*sigma/stride),1.,32.));
  float sum=0.,total=0.;
  for(int i=-32;i<=32;i++) {
    if(abs(i)>taps)continue;
    float distance=float(i)*stride,weight=exp(-.5*distance*distance/(sigma*sigma));
    vec2 pixel=sourcePixel(uv+axis*distance/u_layerRes,inverseLayer);
    sum+=samplePixel(pixel,u_sampleLod).a*weight;total+=weight;
  }
  return sum/max(total,1e-8);
}
void main(){
  if(u_pass==0){
    float angle=radians(u_angle);
    vec2 offset=vec2(sin(angle),-cos(angle))*u_distance/u_layerRes;
    float alpha=samplePixel(sourcePixel(layerUV(v_px)-offset,inverse(u_layerUV)),0.).a;
    o=vec4(0.,0.,0.,alpha);return;
  }
  float alpha=softenedAlpha();
  if(u_pass==1){o=vec4(0.,0.,0.,alpha);return;}
  alpha=clamp(alpha*u_opacity/100.,0.,1.);
  vec4 shaded=vec4(u_color*alpha,alpha);
  vec4 original=texture(u_orig,v_st);
  o=u_shadowOnly>.5 ? shaded : original+shaded*(1.-original.a);
}`,
};
const point = (k: string, label: string, dimension?: 'width'|'height',factor=1): EffectParamDefinition => ({
  ...number(k,label,0,-1e6,1e6,'px'),...(dimension ? {defaultFrom:{dimension,factor}} : {}),
});
const rampParams: EffectParamDefinition[] = [
  point('startX','Start Of Ramp X','width',.5),point('startY','Start Of Ramp Y'),color('startColor','Start Color','#000000'),
  point('endX','End Of Ramp X','width',.5),point('endY','End Of Ramp Y','height'),color('endColor','End Color','#FFFFFF'),
  {...number('rampShape','Ramp Shape',0,0,1),step:1,options:[{v:0,label:'Linear Ramp'},{v:1,label:'Radial Ramp'}]},
  number('rampScatter','Ramp Scatter',0,0,100),number('blendWithOriginal','Blend With Original',0,0,100,'%'),
];
const ramp: EffectDefinition = {
  id:'gradient',label:'Gradient Ramp',group:'Generate',params:rampParams,passes:1,rawShader:true,viewportSafe:true,viewportPadding:[],
  ui:[{kind:'point',x:'startX',y:'startY',label:'Start Of Ramp'},{kind:'point',x:'endX',y:'endY',label:'End Of Ramp'}],
  actions:[{kind:'swap',label:'Swap Colors',keys:['startColor','endColor']}],
  frag:SPATIAL_PRE+uniforms(rampParams)+`
float scatter(vec2 p){return fract(sin(dot(floor(p),vec2(127.1,311.7)))*43758.5453)-.5;}
void main(){
  vec4 c=texture(u_tex,v_st);if(c.a<=0.||u_blendWithOriginal>=100.){o=c;return;}
  vec2 pixel=layerUV(v_px)*u_layerRes;
  vec2 start=vec2(u_startX,u_startY),delta=vec2(u_endX,u_endY)-start;
  float lengthSquared=dot(delta,delta);
  float t=lengthSquared<1e-10 ? 0. : u_rampShape>.5 ? length(pixel-start)/sqrt(lengthSquared) : dot(pixel-start,delta)/lengthSquared;
  t=clamp(t+scatter(pixel)*u_rampScatter/255.,0.,1.);
  vec3 mapped=mix(u_startColor,u_endColor,t);
  o=vec4(mix(mapped,c.rgb/c.a,u_blendWithOriginal/100.)*c.a,c.a);
}`,
};
export function reviseShadowRamp(original: EffectDefinition[]): EffectDefinition[] {
  return original.map(definition=>{
    const revised=definition.id==='shadow' ? shadow : definition.id==='gradient' ? ramp : undefined;
    return revised ? {...revised,version:2,legacy:[{...definition,version:definition.version??1}]} : definition;
  });
}
