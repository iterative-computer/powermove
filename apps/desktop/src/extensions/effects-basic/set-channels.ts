import type { EffectDefinition, EffectParamDefinition } from 'powermove';
const channels=['Alpha','Red','Green','Blue','Luminance','Full On','Full Off'];
const outputs=[['red','Red',1],['green','Green',2],['blue','Blue',3],['alpha','Alpha',0]] as const;
const params: EffectParamDefinition[]=outputs.flatMap(([key,label,def])=>[
 {k:key+'Layer',label:label+' Layer',type:'layer',def:'',group:label,advanced:false},
 {k:key+'Channel',label:label+' Channel',def,min:0,max:6,group:label,advanced:false,options:channels.map((label,v)=>({label,v}))},
]);
/** Independent channel copying, with four separately scheduled layer inputs. */
export const SET_CHANNELS: EffectDefinition={
 id:'setChannels',label:'Set Channels',group:'Channel',viewportSafe:false,passes:1,params,
 frag:`
 vec4 original=texture(u_tex,v_st);
 vec3 point=u_layerUV*vec3(v_px,1.);vec2 q=point.xy/point.z;
 if(any(lessThan(q,vec2(0.)))||any(greaterThan(q,vec2(1.)))){o=vec4(0.);return;}
 float result[4];
 `+outputs.map(([key],index)=>`{
   vec4 sampleColor=u_has_${key}Layer>.5?sample_${key}Layer(q):original;
   vec3 color=sampleColor.a>1e-7?sampleColor.rgb/sampleColor.a:vec3(0.);
   float choices[7]=float[7](sampleColor.a,color.r,color.g,color.b,dot(color,vec3(.2126,.7152,.0722)),1.,0.);
   result[${index}]=choices[int(clamp(round(u_${key}Channel),0.,6.))];
 }`).join('\n')+`
 float alpha=clamp(result[3],0.,1.);
 o=vec4(vec3(result[0],result[1],result[2])*alpha,alpha);
 `,
};
