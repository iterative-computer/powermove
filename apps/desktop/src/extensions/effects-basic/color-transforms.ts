import type {EffectDefinition,EffectParamDefinition} from 'powermove';
import {COLOR_PROFILES,COLOR_SPACE_GLSL} from './color-math';
const choice=(k:string,label:string,options:string[],def=0):EffectParamDefinition=>({k,label,def,min:0,max:options.length-1,advanced:false,options:options.map((label,v)=>({label,v}))});
const toggle=(k:string,label:string):EffectParamDefinition=>({k,label,type:'toggle',def:false,advanced:false});
function fragment(params:EffectParamDefinition[],body:string,helpers=''){return`#version 300 es
precision highp float;
in vec2 v_st;uniform sampler2D u_tex;
${params.map(p=>`uniform float u_${p.k};`).join('\n')}
out vec4 o;
${helpers}
void main(){vec4 original=texture(u_tex,v_st);if(original.a<=0.){o=original;return;}vec3 rgb=original.rgb/original.a;
${body}
}`;}
const profileParams=[choice('inputProfile','Input Profile',COLOR_PROFILES),choice('outputProfile','Output Profile',COLOR_PROFILES),toggle('linearInput','Linear Input'),toggle('linearOutput','Linear Output'),toggle('clip','Clip Output'),{k:'amount',label:'Amount',def:100,min:0,max:100,unit:'%',advanced:false}] as EffectParamDefinition[];
/** Fixed named D65 profiles are deterministic and portable, without external ICC. */
export const COLOR_PROFILE_CONVERTER:EffectDefinition={id:'colorProfileConverter',label:'Color Profile Converter',group:'Utility',passes:1,rawShader:true,viewportSafe:true,sourceProcessing:{},params:profileParams,
 frag:fragment(profileParams,`
 int inputProfile=int(clamp(round(u_inputProfile),0.,2.)),outputProfile=int(clamp(round(u_outputProfile),0.,2.));
 bool linearInput=u_linearInput>.5,linearOutput=u_linearOutput>.5;
 if(u_amount<=0.||inputProfile==outputProfile&&linearInput==linearOutput&&u_clip<.5){o=original;return;}
 vec3 result=convertProfile(rgb,inputProfile,outputProfile,linearInput,linearOutput);if(u_clip>.5)result=clamp(result,0.,1.);
 o=vec4(mix(rgb,result,clamp(u_amount/100.,0.,1.))*original.a,original.a);
 `,COLOR_SPACE_GLSL),
};
const companderParams=[choice('mode','Mode',['Compress','Expand']),{k:'gain',label:'Gain',def:4,min:.001,max:65504,step:.01,advanced:false},{k:'gamma',label:'Gamma',def:2.2,min:.1,max:10,step:.01,advanced:false}] as EffectParamDefinition[];
export const HDR_COMPANDER:EffectDefinition={id:'hdrCompander',label:'HDR Compander',group:'Utility',passes:1,rawShader:true,viewportSafe:true,sourceProcessing:{},params:companderParams,
 frag:fragment(companderParams,`
 float gain=max(u_gain,.001),gamma=max(u_gamma,.1);if(gain==1.&&gamma==1.){o=original;return;}
 vec3 result=u_mode>.5?sign(rgb)*pow(abs(rgb),vec3(gamma))*gain:sign(rgb)*pow(abs(rgb)/gain,vec3(1./gamma));
 o=vec4(result*original.a,original.a);
 `),
};
export const COLOR_TRANSFORMS=[COLOR_PROFILE_CONVERTER,HDR_COMPANDER];
