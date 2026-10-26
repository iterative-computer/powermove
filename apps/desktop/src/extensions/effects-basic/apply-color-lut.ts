import type {EffectDefinition} from 'powermove';
/** Straight RGB table lookup; working-space conversion is intentionally separate. */
export const APPLY_COLOR_LUT:EffectDefinition={id:'applyColorLut',label:'Apply Color LUT',group:'Utility',viewportSafe:true,sourceProcessing:{},
 params:[{k:'lut',label:'LUT',type:'asset',assetKind:'lut',def:'',advanced:false},{k:'amount',label:'Amount',def:100,min:0,max:100,unit:'%',advanced:false}],
 frag:`vec4 c=texture(u_tex,v_st);if(u_has_lut<.5||u_amount<=0.||c.a<=0.){o=c;return;}
 vec3 x=c.rgb/c.a,q=clamp((x-u_lutDomainMin)/(u_lutDomainMax-u_lutDomainMin),0.,1.)*(u_lutSize-1.);
 ivec3 a=ivec3(floor(q)),b=min(a+1,ivec3(u_lutSize-1.));vec3 t=fract(q),result;
 if(u_lutRes.y<1.5){result=vec3(mix(texelFetch(u_lut,ivec2(a.r,0),0).r,texelFetch(u_lut,ivec2(b.r,0),0).r,t.r),mix(texelFetch(u_lut,ivec2(a.g,0),0).g,texelFetch(u_lut,ivec2(b.g,0),0).g,t.g),mix(texelFetch(u_lut,ivec2(a.b,0),0).b,texelFetch(u_lut,ivec2(b.b,0),0).b,t.b));}
 else{int n=int(u_lutSize);vec3 c000=texelFetch(u_lut,ivec2(a.r+a.g*n,a.b),0).rgb,c100=texelFetch(u_lut,ivec2(b.r+a.g*n,a.b),0).rgb,c010=texelFetch(u_lut,ivec2(a.r+b.g*n,a.b),0).rgb,c110=texelFetch(u_lut,ivec2(b.r+b.g*n,a.b),0).rgb,c001=texelFetch(u_lut,ivec2(a.r+a.g*n,b.b),0).rgb,c101=texelFetch(u_lut,ivec2(b.r+a.g*n,b.b),0).rgb,c011=texelFetch(u_lut,ivec2(a.r+b.g*n,b.b),0).rgb,c111=texelFetch(u_lut,ivec2(b.r+b.g*n,b.b),0).rgb;
 result=mix(mix(mix(c000,c100,t.r),mix(c010,c110,t.r),t.g),mix(mix(c001,c101,t.r),mix(c011,c111,t.r),t.g),t.b);}
 o=vec4(mix(x,result,clamp(u_amount/100.,0.,1.))*c.a,c.a);`,
};
