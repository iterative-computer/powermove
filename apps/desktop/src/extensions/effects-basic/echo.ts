import type {EffectDefinition} from 'powermove';
/** Independent implementation from Adobe's public Time effects description.
 * The renderer supplies one bounded, exact owner-prefix sample per pass. */
export const ECHO:EffectDefinition={id:'echo',label:'Echo',group:'Time',rawShader:true,passes:1,viewportSafe:false,sourceProcessing:{},temporal:{step:'echoTime',count:'echoes',maxSamples:32},params:[
 {k:'echoTime',label:'Echo Time',def:-1/30,min:-3600,max:3600,step:.001,unit:'s',advanced:false},
 {k:'echoes',label:'Echoes',def:1,min:0,max:32,step:1,advanced:false},
 {k:'intensity',label:'Starting Intensity',def:1,min:0,max:1,step:.01,advanced:false},
 {k:'decay',label:'Decay',def:.5,min:0,max:1,step:.01,advanced:false},
 {k:'operator',label:'Echo Operator',def:0,min:0,max:6,step:1,advanced:false,options:['Add','Maximum','Minimum','Screen','Composite In Back','Composite In Front','Blend'].map((label,v)=>({label,v}))},
],frag:`#version 300 es
precision highp float;
in vec2 v_st;in vec2 v_px;
uniform sampler2D u_tex,u_temporal;
uniform mat3 u_temporalOwnerUV,u_temporalFromUV;
uniform vec2 u_res,u_temporalRes;
uniform vec4 u_temporalExtent,u_temporalPriorExtent,u_temporalBounds,u_temporalPriorBounds;
uniform float u_temporalIndex,u_temporalCount,u_temporalValid,u_temporalSource,u_temporalDirect;
uniform float u_echoTime,u_echoes,u_intensity,u_decay,u_operator;
out vec4 o;
vec4 historical(){
 if(u_temporalDirect>.5)return texture(u_temporal,v_st);
 if(u_temporalValid<.5)return vec4(0.);
 vec2 st;
 if(u_temporalSource>.5){vec2 local=u_temporalExtent.xy+vec2(v_st.x,1.-v_st.y)*u_temporalExtent.zw;st=(local-u_temporalPriorExtent.xy)/u_temporalPriorExtent.zw;}
 else{vec3 local=u_temporalOwnerUV*vec3(v_px,1.);vec2 point=u_temporalBounds.xy+(local.xy/local.z)*u_temporalBounds.zw;vec2 prior=(point-u_temporalPriorBounds.xy)/u_temporalPriorBounds.zw;vec3 px=u_temporalFromUV*vec3(prior,1.);st=px.xy/px.z/u_temporalRes;}
 st.y=1.-st.y;return any(lessThan(st,vec2(0.)))||any(greaterThan(st,vec2(1.)))?vec4(0.):texture(u_temporal,st);
}
void main(){
 vec4 sampleColor=historical()*clamp(u_intensity,0.,1.)*pow(clamp(u_decay,0.,1.),u_temporalIndex);
 // Explicit first sample avoids relying on pow(0,0) or a cleared accumulator
 // as the identity for Minimum/Maximum over signed floating-point colors.
 if(u_temporalIndex<.5)sampleColor=historical()*clamp(u_intensity,0.,1.);
 int mode=int(clamp(round(u_operator),0.,6.));
 if(mode==6)sampleColor/=u_temporalCount;
 if(u_temporalIndex<.5){o=sampleColor;return;}
 vec4 base=texture(u_tex,v_st),result;
 if(mode==1)result=max(base,sampleColor);else if(mode==2)result=min(base,sampleColor);
 else if(mode==3)result=base+sampleColor-base*sampleColor;
 else if(mode==4)result=sampleColor+base*(1.-sampleColor.a);
 else if(mode==5)result=base+sampleColor*(1.-base.a);
 else result=base+sampleColor;
 o=result.a<=0.?vec4(0.):vec4(result.rgb,clamp(result.a,0.,1.));
}
`};
