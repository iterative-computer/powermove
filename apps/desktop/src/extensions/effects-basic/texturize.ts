import type {EffectDefinition} from 'powermove';

/** Independent height-map relief; direction and kernel still require AE calibration. */
const ORIGINAL_TEXTURIZE:EffectDefinition={
 id:'texturize',label:'Texturize',group:'Stylize',viewportSafe:false,passes:1,
 params:[
  {k:'map',label:'Texture Layer',type:'layer',def:'',advanced:false},
  {k:'direction',label:'Light Direction',def:315,min:-3600,max:3600,unit:'°',advanced:false},
  {k:'contrast',label:'Texture Contrast',def:100,min:0,max:1000,unit:'%',advanced:false},
  {k:'placement',label:'Placement',def:0,min:0,max:2,advanced:false,options:[{v:0,label:'Tile Texture'},{v:1,label:'Center Texture'},{v:2,label:'Stretch To Fit'}]},
 ],
 frag:`
 vec4 original=texture(u_tex,v_st);
 if(u_has_map<.5||u_contrast<=0.){o=original;return;}
 vec3 local=u_layerUV*vec3(v_px,1.);vec2 q=local.xy/local.z;
 vec2 dx=vec2(1./u_layerRes.x,0.),dy=vec2(0.,1./u_layerRes.y);
 vec2 points[4]=vec2[4](q-dx,q+dx,q-dy,q+dy);float heights[4];
 for(int i=0;i<4;i++){
  vec2 at=points[i];
  if(u_placement<1.5)at=(at-.5)*u_layerRes/u_mapRes+.5;
  if(u_placement<.5)at=fract(at);
  else if(u_placement>1.5)at=clamp(at,vec2(0.),vec2(1.));
  heights[i]=dot(sample_map(at).rgb,vec3(.2126,.7152,.0722));
 }
 vec2 slope=vec2(heights[1]-heights[0],heights[3]-heights[2])*max(0.,u_contrast)*.04;
 vec3 normal=normalize(vec3(-slope,1.));float angle=radians(u_direction);
 float gain=max(0.,dot(normal,vec3(cos(angle),sin(angle),1.)));
 o=vec4(original.rgb*gain,original.a);
 `,
};

export const TEXTURIZE:EffectDefinition={...ORIGINAL_TEXTURIZE,version:2,legacy:[{...ORIGINAL_TEXTURIZE,version:1}],
 params:[...ORIGINAL_TEXTURIZE.params,{k:'texture',label:'Texture Asset',type:'asset',def:'',advanced:false}],
 frag:ORIGINAL_TEXTURIZE.frag.replace('vec4 original=texture(u_tex,v_st);','vec4 original=texture(u_tex,v_st);vec2 textureSize=u_has_texture>.5?u_textureRes:u_mapRes;').replaceAll('u_mapRes','textureSize').replace('u_textureRes:textureSize','u_textureRes:u_mapRes').replace('u_has_map<.5','(u_has_map<.5&&u_has_texture<.5)').replace('sample_map(at).rgb','(u_has_texture>.5?sample_texture(at):sample_map(at)).rgb'),
};
