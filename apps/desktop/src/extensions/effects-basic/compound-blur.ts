import type { EffectDefinition } from 'powermove';

/** Independent output-centered variable Gaussian. Each output uses one radius
 * for its entire 2D gather; varying radii are not treated as a separable blur. */
export const COMPOUND_BLUR: EffectDefinition = {
 id:'compoundBlur',label:'Compound Blur',group:'Blur & Sharpen',passes:1,
 rawShader:true,viewportSafe:false,
 sampling:{radius:'maximumBlur',support:1.5,maxRadius:8,space:'layer'},
 params:[
  {k:'map',label:'Blur Layer',type:'layer',def:'',advanced:false},
  {k:'maximumBlur',label:'Maximum Blur',def:20,min:0,max:400,step:.1,unit:'px',advanced:false},
  {k:'stretch',label:'Stretch Map',type:'toggle',def:true,advanced:false},
  {k:'invert',label:'Invert Blur',type:'toggle',def:false,advanced:false},
 ],
 frag:`#version 300 es
 precision highp float;
 in vec2 v_st; in vec2 v_px;
 uniform sampler2D u_tex,u_map;
 uniform float u_has_map,u_maximumBlur,u_stretch,u_invert,u_sampleLod;
 uniform vec2 u_res,u_layerRes,u_mapRes;
 uniform mat3 u_layerUV,u_ownerFromUV,u_mapFromUV;
 out vec4 o;
 vec4 mapAt(vec2 q){
  if(u_stretch>.5)q=clamp(q,vec2(0.),vec2(1.));
  else q=(q-.5)*u_layerRes/u_mapRes+.5;
  if(any(lessThan(q,vec2(0.)))||any(greaterThan(q,vec2(1.))))return vec4(0.);
  vec3 px=u_mapFromUV*vec3(q,1.);vec2 st=px.xy/px.z/u_res;st.y=1.-st.y;
  if(any(lessThan(st,vec2(0.)))||any(greaterThan(st,vec2(1.))))return vec4(0.);
  return texture(u_map,st);
 }
 void main(){
  vec4 original=texture(u_tex,v_st);
  if(u_has_map<.5||u_maximumBlur<=0.){o=original;return;}
  vec3 local=u_layerUV*vec3(v_px,1.);vec2 uv=local.xy/local.z;
  float amount=clamp(dot(mapAt(uv).rgb,vec3(.2126,.7152,.0722)),0.,1.);
  if(u_invert>.5)amount=1.-amount;
  float sigma=max(0.,u_maximumBlur)*.5*amount;
  if(sigma<.01){o=original;return;}
  float density=max(length(u_ownerFromUV[0].xy)/u_layerRes.x,length(u_ownerFromUV[1].xy)/u_layerRes.y);
  density=max(density,1e-6);
  float lod=min(u_sampleLod,max(0.,ceil(log2(max(1.,3.*sigma*density/8.)))));
  float stride=exp2(lod)/density;
  int taps=int(clamp(ceil(3.*sigma/stride),1.,8.));
  vec4 sum=vec4(0.);float total=0.;
  for(int y=-8;y<=8;y++){
   if(abs(y)>taps)continue;
   for(int x=-8;x<=8;x++){
    if(abs(x)>taps)continue;
    vec2 offset=vec2(float(x),float(y))*stride;
    float weight=exp(-.5*dot(offset,offset)/(sigma*sigma));
    vec3 pixel=u_ownerFromUV*vec3(uv+offset/u_layerRes,1.);vec2 st=pixel.xy/pixel.z/u_res;st.y=1.-st.y;
    vec4 color=vec4(0.);
    if(all(greaterThanEqual(st,vec2(0.)))&&all(lessThanEqual(st,vec2(1.))))color=textureLod(u_tex,st,lod);
    sum+=color*weight;total+=weight;
   }
  }
  o=sum/max(total,1e-8);
 }`,
};
