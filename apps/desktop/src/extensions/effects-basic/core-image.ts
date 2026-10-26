import type { EffectDefinition, EffectParamDefinition } from 'powermove';

const parameter = (k: string, label: string, def: number, min: number, max: number, group?: string, unit = ''): EffectParamDefinition =>
  ({ k, label, def, min, max, group, unit, step: .01, advanced: false });
const choice = (k: string, label: string, labels: string[], def = 0): EffectParamDefinition =>
  ({ k, label, def, min: 0, max: labels.length - 1, step: 1, advanced: false, options: labels.map((label, v) => ({ v, label })) });
const toggle = (k: string, label: string, def: boolean): EffectParamDefinition => ({ k, label, def, type: 'toggle', advanced: false });

export const HSL = `
vec3 rgbToHsl(vec3 c) {
  float hi=max(c.r,max(c.g,c.b)), lo=min(c.r,min(c.g,c.b));
  float d=hi-lo, l=(hi+lo)*.5, h=0., s=0.;
  if(d>1e-7) {
    s=d/max(1.-abs(2.*l-1.),1e-7);
    h=hi==c.r ? mod((c.g-c.b)/d,6.) : hi==c.g ? (c.b-c.r)/d+2. : (c.r-c.g)/d+4.;
    h=fract(h/6.+1.);
  }
  return vec3(h,l,s);
}
vec3 hslToRgb(vec3 hls) {
  vec3 hue=clamp(abs(mod(hls.x*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.);
  return hls.y+(hue-.5)*(1.-abs(2.*hls.y-1.))*hls.z;
}`;

/** Alpha-creating operators are bounded to the source layer, including transforms. */
export function imageFragment(params: EffectParamDefinition[], body: string, helpers = '', createsAlpha = false): string {
  return `#version 300 es
precision highp float;
in vec2 v_st;
in vec2 v_px;
uniform sampler2D u_tex;
uniform mat3 u_layerUV;
${params.map(p => `uniform ${p.type === 'color' ? 'vec3' : 'float'} u_${p.k};`).join('\n')}
out vec4 o;
float luma(vec3 c) { return dot(c,vec3(.2126,.7152,.0722)); }
float signedPower(float x,float exponent) { return sign(x)*pow(abs(x),exponent); }
bool insideLayer() {
  vec3 q=u_layerUV*vec3(v_px,1.); vec2 uv=q.xy/q.z;
  return all(greaterThanEqual(uv,vec2(0.))) && all(lessThanEqual(uv,vec2(1.)));
}
${helpers}
void main() {
  vec4 c=texture(u_tex,v_st);
  ${createsAlpha ? 'if(!insideLayer()){o=vec4(0.);return;}' : 'if(c.a<=0.){o=vec4(0.);return;}'}
  vec3 rgb=c.a>0. ? c.rgb/c.a : vec3(0.);
  ${body}
}`;
}
const effect = (id: string, label: string, group: string, params: EffectParamDefinition[], body: string, helpers = '', createsAlpha = false): EffectDefinition =>
  ({ id, label, group, params, passes: 1, rawShader: true, viewportSafe: true, viewportPadding: [], frag: imageFragment(params, body, helpers, createsAlpha) });

const invertParams = [
  choice('channel', 'Channel', ['RGB','Red','Green','Blue','HLS','Hue','Lightness','Saturation','YIQ','Luminance','In Phase Chrominance','Quadrature Chrominance','Alpha']),
  parameter('blendWithOriginal','Blend With Original',0,0,100,undefined,'%'),
];
const invert = effect('invert','Invert','Color',invertParams,`
  int channel=int(clamp(u_channel,0.,12.)+.5); vec3 result=rgb; float alpha=c.a;
  if(channel==0) result=1.-rgb;
  else if(channel<=3) result[channel-1]=1.-rgb[channel-1];
  else if(channel<=7) {
    vec3 hls=rgbToHsl(clamp(rgb,0.,1.));
    if(channel==4 || channel==5) hls.x=fract(1.-hls.x);
    if(channel==4 || channel==6) hls.y=1.-hls.y;
    if(channel==4 || channel==7) hls.z=1.-hls.z;
    result=hslToRgb(hls);
  } else if(channel<=11) {
    vec3 yiq=vec3(dot(rgb,vec3(.299,.587,.114)),dot(rgb,vec3(.596,-.274,-.322)),dot(rgb,vec3(.211,-.523,.312)));
    if(channel==8 || channel==9) yiq.x=1.-yiq.x;
    if(channel==8 || channel==10) yiq.y=-yiq.y;
    if(channel==8 || channel==11) yiq.z=-yiq.z;
    result=vec3(yiq.x+.956*yiq.y+.621*yiq.z,yiq.x-.272*yiq.y-.647*yiq.z,yiq.x-1.106*yiq.y+1.703*yiq.z);
  } else alpha=1.-c.a;
  o=mix(vec4(result*alpha,alpha),c,u_blendWithOriginal/100.);
`,HSL,true);

// Normalized values are independent of the project's storage bit depth.
const levelsParams: EffectParamDefinition[] = [];
for (const [prefix, group] of [['','RGB'],['red','Red'],['green','Green'],['blue','Blue'],['alpha','Alpha']]) {
  const key = (name: string) => prefix ? prefix + name[0]!.toUpperCase() + name.slice(1) : name;
  levelsParams.push(parameter(key('inBlack'),'Input Black',0,-16,16,group), parameter(key('inWhite'),'Input White',1,-16,16,group),
    parameter(key('gamma'),'Gamma',1,.01,10,group), parameter(key('outBlack'),'Output Black',0,-16,16,group), parameter(key('outWhite'),'Output White',1,-16,16,group));
}
levelsParams.push(toggle('clipBlack','Clip To Output Black',true),toggle('clipWhite','Clip To Output White',true));
const levelCall=(x: string,prefix: string)=>`level(${x},u_${prefix ? prefix+'InBlack' : 'inBlack'},u_${prefix ? prefix+'InWhite' : 'inWhite'},u_${prefix ? prefix+'Gamma' : 'gamma'},u_${prefix ? prefix+'OutBlack' : 'outBlack'},u_${prefix ? prefix+'OutWhite' : 'outWhite'})`;
const levels = effect('levels','Levels','Color',levelsParams,`
  vec3 mapped=vec3(${levelCall('rgb.r','')},${levelCall('rgb.g','')},${levelCall('rgb.b','')});
  mapped=vec3(${levelCall('mapped.r','red')},${levelCall('mapped.g','green')},${levelCall('mapped.b','blue')});
  float alpha=clamp(${levelCall('c.a','alpha')},0.,1.);
  o=vec4(mapped*alpha,alpha);
`, `float level(float x,float black,float white,float gamma,float outBlack,float outWhite) {
  float span=white-black;
  float t=abs(span)<1e-7 ? step(black,x) : (x-black)/span;
  if(u_clipBlack>.5) t=max(0.,t);
  if(u_clipWhite>.5) t=min(1.,t);
  return mix(outBlack,outWhite,signedPower(t,1./max(gamma,.01)));
}`,true);

const thresholdParams = [parameter('level','Level',128,0,255)];
const threshold = effect('threshold','Threshold','Stylize',thresholdParams, 'float white=step(u_level/255.,luma(rgb)); o=vec4(vec3(white)*c.a,c.a);');

const channelNames=['Alpha','Red','Green','Blue','Luminance','Hue','Lightness','Saturation','Full On','Full Off'];
const shiftParams=[choice('takeAlphaFrom','Take Alpha From',channelNames,0),choice('takeRedFrom','Take Red From',channelNames,1),choice('takeGreenFrom','Take Green From',channelNames,2),choice('takeBlueFrom','Take Blue From',channelNames,3)];
const shift = effect('shiftChannels','Shift Channels','Color',shiftParams,`
  vec3 hls=rgbToHsl(clamp(rgb,0.,1.));
  float alpha=clamp(channelValue(int(clamp(u_takeAlphaFrom,0.,9.)+.5),rgb,c.a,hls),0.,1.);
  vec3 mapped=vec3(channelValue(int(clamp(u_takeRedFrom,0.,9.)+.5),rgb,c.a,hls),channelValue(int(clamp(u_takeGreenFrom,0.,9.)+.5),rgb,c.a,hls),channelValue(int(clamp(u_takeBlueFrom,0.,9.)+.5),rgb,c.a,hls));
  o=vec4(mapped*alpha,alpha);
`, HSL+`
float channelValue(int index,vec3 rgb,float alpha,vec3 hls) {
  if(index==0) return alpha;
  if(index<=3) return rgb[index-1];
  if(index==4) return luma(rgb);
  if(index==5) return hls.x;
  if(index==6) return hls.y;
  if(index==7) return hls.z;
  return index==8 ? 1. : 0.;
}`,true);

const exposureParams: EffectParamDefinition[] = [];
for (const [prefix, group] of [['','Master'],['red','Red'],['green','Green'],['blue','Blue']]) {
  const key=(name: string)=>prefix ? prefix+name[0]!.toUpperCase()+name.slice(1) : name;
  exposureParams.push(parameter(key('exposure'),'Exposure',0,-20,20,group,'stops'),parameter(key('offset'),'Offset',0,-10,10,group),parameter(key('gamma'),'Gamma Correction',1,.01,10,group));
}
exposureParams.push(toggle('bypassLinearLight','Bypass Linear Light Conversion',false));
const expose=(v:string,prefix:string)=>`expose(${v},u_${prefix ? prefix+'Exposure' : 'exposure'},u_${prefix ? prefix+'Offset' : 'offset'},u_${prefix ? prefix+'Gamma' : 'gamma'})`;
const exposure = effect('exposure','Exposure','Color',exposureParams,`
  vec3 linearRGB=u_bypassLinearLight>.5 ? rgb : vec3(toLinear(rgb.r),toLinear(rgb.g),toLinear(rgb.b));
  linearRGB=vec3(${expose('linearRGB.r','')},${expose('linearRGB.g','')},${expose('linearRGB.b','')});
  linearRGB=vec3(${expose('linearRGB.r','red')},${expose('linearRGB.g','green')},${expose('linearRGB.b','blue')});
  vec3 mapped=u_bypassLinearLight>.5 ? linearRGB : vec3(fromLinear(linearRGB.r),fromLinear(linearRGB.g),fromLinear(linearRGB.b));
  o=vec4(mapped*c.a,c.a);
`, `float toLinear(float x) { float v=abs(x); return sign(x)*(v<=.04045 ? v/12.92 : pow((v+.055)/1.055,2.4)); }
float fromLinear(float x) { float v=abs(x); return sign(x)*(v<=.0031308 ? v*12.92 : 1.055*pow(v,1./2.4)-.055); }
float expose(float x,float stops,float offset,float gamma) { return signedPower(x*exp2(stops)+offset,1./max(gamma,.01)); }`);

export function reviseCoreEffects(original: EffectDefinition[]): EffectDefinition[] {
  return original.map(definition => {
    const revised=definition.id==='invert' ? invert : definition.id==='levels' ? levels : undefined;
    return revised ? { ...revised, version: 2, legacy: [{ ...definition, version: 1 }] } : definition;
  });
}
export const CORE_IMAGE_EFFECTS: EffectDefinition[] = [threshold,shift,exposure,{...levels,id:'levelsIndividual',label:'Levels (Individual Controls)'}];
