import {matrixGLSL,inverse,RGB_TO_YIQ,RGB_TO_YUV,RGB_TO_YCBCR,RGB_TO_XYZ,XYZ_TO_RGB} from './color-math';
import type {EffectDefinition,EffectParamDefinition} from 'powermove';
import {HSL} from './core-image';
const choice=(k:string,label:string,labels:string[],def=0):EffectParamDefinition=>({k,label,def,min:0,max:labels.length-1,step:1,advanced:false,options:labels.map((label,v)=>({label,v}))});
const toggle=(k:string,label:string,def=false):EffectParamDefinition=>({k,label,def,type:'toggle',advanced:false});
const percent=(k:string,label:string,def:number):EffectParamDefinition=>({k,label,def,min:0,max:100,unit:'%',advanced:false});
const map=(label:string):EffectParamDefinition=>({k:'map',label,type:'layer',def:'',advanced:false});
export const COMPOSITING_MODES=['Normal','Multiply','Screen','Overlay','Darken','Lighten','Difference','Add','Subtract'];
const CHANNELS=['RGBA','Gray','Red','Green','Blue','Alpha'];
/** Shared independent straight-channel math. Alpha remains explicit throughout. */
const HELPERS=HSL+`
vec3 straight(vec4 c){return c.a>0.?c.rgb/c.a:vec3(0.);}
vec4 extractChannel(vec4 c,float channel,float invert){
 vec3 rgb=straight(c);float alpha=c.a;if(invert>.5){rgb=1.-rgb;alpha=1.-alpha;}
 int ch=int(clamp(round(channel),0.,5.));
 if(ch==1)rgb=vec3(dot(rgb,vec3(.2126,.7152,.0722)));
 else if(ch>=2&&ch<=4)rgb=vec3(rgb[ch-2]);else if(ch==5)rgb=vec3(alpha);
 return vec4(rgb,clamp(alpha,0.,1.));
}
vec3 blendColor(vec3 b,vec3 s,int mode){
 if(mode==1)return b*s;if(mode==2)return 1.-(1.-b)*(1.-s);
 if(mode==3)return mix(2.*b*s,1.-2.*(1.-b)*(1.-s),step(vec3(.5),b));
 if(mode==4)return min(b,s);if(mode==5)return max(b,s);if(mode==6)return abs(b-s);
 if(mode==7)return b+s;if(mode==8)return b-s;return s;
}
vec4 overBlend(vec4 base,vec4 source,int mode){
 float ab=base.a,as=source.a;vec3 rgb=(1.-as)*base.rgb*ab+(1.-ab)*source.rgb*as+blendColor(base.rgb,source.rgb,mode)*ab*as;
 return vec4(rgb,ab+as*(1.-ab));
}
`;
/** Raw bodies share the same validated named reference/sampler contract as
 * generated effects; helper functions live outside main, without extra passes. */
function fragment(params:EffectParamDefinition[],body:string){return`#version 300 es
precision highp float;
in vec2 v_st;in vec2 v_px;
uniform sampler2D u_tex,u_map;
uniform mat3 u_layerUV,u_ownerFromUV,u_mapFromUV;
uniform vec2 u_res,u_layerRes,u_mapRes;
uniform float u_has_map,u_ownerValid;
${params.filter(p=>p.type!=='layer').map(p=>`uniform ${p.type==='color'?'vec3':'float'} u_${p.k};`).join('\n')}
out vec4 o;
bool inUnit(vec2 q){return all(greaterThanEqual(q,vec2(0.)))&&all(lessThanEqual(q,vec2(1.)));}
vec4 sampleMap(vec2 q){if(!inUnit(q))return vec4(0.);vec3 px=u_mapFromUV*vec3(q,1.);vec2 st=px.xy/px.z/u_res;st.y=1.-st.y;return inUnit(st)?texture(u_map,st):vec4(0.);}
${HELPERS}
void main(){vec4 original=texture(u_tex,v_st);if(u_ownerValid<.5){o=original;return;}vec3 point=u_layerUV*vec3(v_px,1.);vec2 owner=point.xy/point.z;
${body}
}`;}
function effect(id:string,label:string,params:EffectParamDefinition[],body:string):EffectDefinition{return{id,label,group:'Channel',passes:1,rawShader:true,viewportSafe:false,sourceProcessing:{},params,frag:fragment(params,body)};}
export const BLEND=effect('blend','Blend',[
 map('Blend With Layer'),choice('mode','Mode',['Color Only','Tint Only','Darken Only','Lighten Only','Crossfade'],4),percent('blendWithOriginal','Blend With Original',0),choice('placement','If Layer Sizes Differ',['Center','Stretch To Fit'],1),
],`
 float amount=1.-clamp(u_blendWithOriginal/100.,0.,1.);if(u_has_map<.5||amount<=0.){o=original;return;}
 vec2 q=u_placement>.5?owner:(owner-.5)*u_layerRes/u_mapRes+.5;
 vec4 secondary=inUnit(owner)?sampleMap(q):vec4(0.);int mode=int(clamp(round(u_mode),0.,4.));
 // A premultiplied crossfade fades both images, including transparent regions.
 if(mode==4){o=mix(original,secondary,amount);return;}
 vec3 a=straight(original),b=straight(secondary),color=a;
 if(mode==2)color=min(a,b);else if(mode==3)color=max(a,b);
 else{float peak=max(1.,max(a.r,max(a.g,a.b)));vec3 ah=rgbToHsl(clamp(a/peak,0.,1.)),bh=rgbToHsl(clamp(b,0.,1.));
  float saturation=mode==1?ah.z*bh.z:bh.z;color=hslToRgb(vec3(bh.x,ah.y,saturation))*peak;}
 o=vec4(mix(a,color,amount*secondary.a)*original.a,original.a);
`);
export const CALCULATIONS=effect('calculations','Calculations',[
 choice('inputChannel','Input Channel',CHANNELS),toggle('invertInput','Invert Input'),map('Second Layer'),choice('secondChannel','Second Layer Channel',CHANNELS),percent('secondOpacity','Second Layer Opacity',100),toggle('invertSecond','Invert Second Layer'),choice('mode','Blending Mode',COMPOSITING_MODES),toggle('stretch','Stretch Second Layer To Fit',true),toggle('preserveTransparency','Preserve Transparency',true),
],`
 if(u_has_map<.5&&u_inputChannel<.5&&u_invertInput<.5){o=original;return;}
 // Alpha creation is bounded to the native owner; already-expanded content is retained.
 if(!inUnit(owner)){o=original;return;}
 vec2 q=u_stretch>.5?owner:(owner-.5)*u_layerRes/u_mapRes+.5;bool present=u_has_map>.5&&inUnit(q);
 vec4 base=extractChannel(original,u_inputChannel,u_invertInput),source=extractChannel(sampleMap(q),u_secondChannel,u_invertSecond);
 source.a*=present?clamp(u_secondOpacity/100.,0.,1.):0.;
 vec4 result=overBlend(base,source,int(clamp(round(u_mode),0.,8.)));
 if(u_preserveTransparency>.5){vec3 rgb=straight(result);result=vec4(rgb*original.a,original.a);}
 o=result;
`);
const CHANNEL_COMBINER_V1=effect('channelCombiner','Channel Combiner',[
 map('Source Layer'),choice('from','From',['Red','Green','Blue','Alpha','Luminance','Hue','Lightness','Saturation','Saturation Multiplied','Min RGB','Max RGB'],4),choice('to','To',['RGB','Red Only','Green Only','Blue Only','Alpha Only','Hue Only','Lightness Only','Saturation Only']),toggle('invert','Invert'),toggle('solidAlpha','Solid Alpha'),
],`
 if(!inUnit(owner)){o=original;return;}
 vec4 source=u_has_map>.5?sampleMap(owner):original;vec3 rgb=straight(source),hls=rgbToHsl(clamp(rgb,0.,1.));
 float values[11]=float[11](rgb.r,rgb.g,rgb.b,source.a,dot(rgb,vec3(.2126,.7152,.0722)),hls.x,hls.y,hls.z,hls.z*min(hls.y,1.-hls.y),min(rgb.r,min(rgb.g,rgb.b)),max(rgb.r,max(rgb.g,rgb.b)));
 float value=values[int(clamp(round(u_from),0.,10.))];
 vec3 result=vec3(value);float alpha=original.a;int target=int(clamp(round(u_to),0.,7.));
 if(target>=1&&target<=3){result=vec3(0.);result[target-1]=value;}
 else if(target==4){result=vec3(1.);alpha=clamp(value,0.,1.);}
 else if(target==5)result=hslToRgb(vec3(fract(value),.5,1.));
 else if(target==7)result=hslToRgb(vec3(0.,.5,clamp(value,0.,1.)));
 if(u_invert>.5){if(target==4)alpha=1.-alpha;else result=1.-result;}
 if(u_solidAlpha>.5)alpha=1.;o=vec4(result*alpha,alpha);
`);
const CONVERSIONS=['RGB to HLS','HLS to RGB','RGB to HSV','HSV to RGB','RGB to YIQ','YIQ to RGB','RGB to YUV','YUV to RGB','Linear RGB to XYZ','XYZ to Linear RGB','RGB to YCbCr (709)','YCbCr (709) to RGB'];
const CONVERSION_GLSL=`
vec3 rgbToHslStable(vec3 c){float hi=max(c.r,max(c.g,c.b)),lo=min(c.r,min(c.g,c.b)),d=hi-lo,l=(hi+lo)*.5,h=0.,s=0.;if(d>0.){s=clamp(d/max(2.*min(l,1.-l),1e-30),0.,1.);h=hi==c.r?mod((c.g-c.b)/d,6.):hi==c.g?(c.b-c.r)/d+2.:(c.r-c.g)/d+4.;h=fract(h/6.+1.);}return vec3(h,l,s);}
vec3 hslToRgbStable(vec3 c){vec3 hue=clamp(abs(mod(c.x*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.);return c.y+(hue-.5)*(2.*min(c.y,1.-c.y))*c.z;}
vec3 rgbToHsv(vec3 rgb){vec3 c=max(rgb,vec3(0.));float hi=max(c.r,max(c.g,c.b)),lo=min(c.r,min(c.g,c.b)),d=hi-lo,h=0.;if(d>0.){h=hi==c.r?mod((c.g-c.b)/d,6.):hi==c.g?(c.b-c.r)/d+2.:(c.r-c.g)/d+4.;h=fract(h/6.+1.);}return vec3(h,hi>0.?d/hi:0.,hi);}
vec3 hsvToRgb(vec3 c){vec3 hue=clamp(abs(mod(c.x*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.);return max(c.z,0.)*mix(vec3(1.),hue,clamp(c.y,0.,1.));}
vec3 convertChannels(vec3 rgb,int mode){
 if(mode==11)return rgbToHslStable(clamp(rgb,0.,1.));if(mode==12)return hslToRgbStable(vec3(fract(rgb.x),clamp(rgb.yz,0.,1.)));
 if(mode==13)return rgbToHsv(rgb);if(mode==14)return hsvToRgb(rgb);
 if(mode==15)return ${matrixGLSL(RGB_TO_YIQ)}*rgb;if(mode==16)return ${matrixGLSL(inverse(RGB_TO_YIQ))}*rgb;
 if(mode==17)return ${matrixGLSL(RGB_TO_YUV)}*rgb;if(mode==18)return ${matrixGLSL(inverse(RGB_TO_YUV))}*rgb;
 if(mode==19)return ${matrixGLSL(RGB_TO_XYZ[0]!)}*rgb;if(mode==20)return ${matrixGLSL(XYZ_TO_RGB[0]!)}*rgb;
 if(mode==21)return ${matrixGLSL(RGB_TO_YCBCR)}*rgb+vec3(0.,.5,.5);return ${matrixGLSL(inverse(RGB_TO_YCBCR))}*(rgb-vec3(0.,.5,.5));
}
`;
export const CHANNEL_COMBINER:EffectDefinition={...CHANNEL_COMBINER_V1,version:2,legacy:[{...CHANNEL_COMBINER_V1,version:1}],
 params:CHANNEL_COMBINER_V1.params.map(p=>p.k==='from'?{...p,max:22,options:[...p.options!,...CONVERSIONS.map((label,i)=>({label,v:11+i}))]}:p),
 frag:CHANNEL_COMBINER_V1.frag.replace('void main(){',CONVERSION_GLSL+'\nvoid main(){').replace(
 'vec3 rgb=straight(source),hls=rgbToHsl(clamp(rgb,0.,1.));',
 `vec3 rgb=straight(source);int mode=int(clamp(round(u_from),0.,22.));
 if(mode>=11){vec3 converted=convertChannels(rgb,mode);if(u_invert>.5)converted=1.-converted;float alpha=u_solidAlpha>.5?1.:original.a;o=vec4(converted*alpha,alpha);return;}
 vec3 hls=rgbToHsl(clamp(rgb,0.,1.));`),
};
export const LAYER_COMPOSITING=[BLEND,CALCULATIONS,CHANNEL_COMBINER];
