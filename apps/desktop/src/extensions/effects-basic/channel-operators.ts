import type {EffectDefinition} from 'powermove';
export const SOLID_COMPOSITE:EffectDefinition={id:'solidComposite',coordinates:'layer',label:'Solid Composite',group:'Channel',viewportSafe:true,sourceProcessing:{},
 params:[{k:'sourceOpacity',label:'Source Opacity',def:100,min:0,max:100,unit:'%',advanced:false},{k:'color',label:'Color',type:'color',def:'#FFFFFF',advanced:false},{k:'opacity',label:'Opacity',def:100,min:0,max:100,unit:'%',advanced:false},{k:'mode',label:'Blending Mode',def:0,min:0,max:8,advanced:false,options:['Normal','Multiply','Screen','Overlay','Darken','Lighten','Difference','Add','Subtract'].map((label,v)=>({label,v}))}],
 frag:`vec4 original=texture(u_tex,v_st),s=original*clamp(u_sourceOpacity/100.,0.,1.);vec3 p=u_layerUV*vec3(v_px,1.);vec2 q=p.xy/p.z;
 if(any(lessThan(q,vec2(0.)))||any(greaterThan(q,vec2(1.)))){o=s;return;}
 float a=clamp(u_opacity/100.,0.,1.);vec3 b=u_color,x=s.a>1e-7?s.rgb/s.a:vec3(0.),blend=x;
 int mode=int(round(u_mode));if(mode==1)blend=b*x;else if(mode==2)blend=1.-(1.-b)*(1.-x);else if(mode==3)blend=mix(2.*b*x,1.-2.*(1.-b)*(1.-x),step(vec3(.5),b));else if(mode==4)blend=min(b,x);else if(mode==5)blend=max(b,x);else if(mode==6)blend=abs(b-x);else if(mode==7)blend=b+x;else if(mode==8)blend=b-x;
 o=vec4((1.-a)*s.rgb+(1.-s.a)*b*a+blend*s.a*a,s.a+a*(1.-s.a));`,
};
export const ARITHMETIC:EffectDefinition={id:'arithmetic',label:'Arithmetic',group:'Channel',viewportSafe:true,sourceProcessing:{},
 params:[{k:'operator',label:'Operator',def:0,min:0,max:11,advanced:false,options:['Add','Subtract','Multiply','Difference','Max','Min','Block Above','Block Below','Screen','And','Or','Xor'].map((label,v)=>({label,v}))},...(['red','green','blue'] as const).map(k=>({k,label:k[0]!.toUpperCase()+k.slice(1)+' Value',def:0,min:0,max:255,step:1,advanced:false})),{k:'clip',label:'Clip Result Values',type:'toggle',def:true,advanced:false}],
 frag:`vec4 c=texture(u_tex,v_st);if(c.a<=0.){o=c;return;}vec3 x=c.rgb/c.a,v=clamp(vec3(u_red,u_green,u_blue),0.,255.)/255.,r=x;int op=int(round(u_operator));
 if(op==0)r=x+v;else if(op==1)r=x-v;else if(op==2)r=x*v;else if(op==3)r=abs(x-v);else if(op==4)r=max(x,v);else if(op==5)r=min(x,v);else if(op==6)r=x*step(x,v);else if(op==7)r=x*step(v,x);else if(op==8)r=1.-(1.-x)*(1.-v);
 else{ivec3 a=ivec3(round(clamp(x,0.,1.)*255.)),b=ivec3(round(v*255.));r=vec3(op==9?(a&b):op==10?(a|b):(a^b))/255.;}
 r=u_clip>.5?clamp(r,0.,1.):mod(round(r*255.),256.)/255.;o=vec4(r*c.a,c.a);`,
};
