import type { EffectDefinition, EffectParamDefinition } from 'powermove';

const layer = (k: string, label: string): EffectParamDefinition => ({ k, label, type: 'layer', def: '' });
const number = (k: string, label: string, def: number, min: number, max: number, unit = ''): EffectParamDefinition => ({ k, label, def, min, max, unit });
const toggle = (k: string, label: string, def: boolean): EffectParamDefinition => ({ k, label, def, type: 'toggle' });
const channel = (k: string, label: string, def: number): EffectParamDefinition => ({ ...number(k,label,def,0,8), options: ['Red','Green','Blue','Alpha','Luminance','Hue','Lightness','Saturation','Off'].map((label,v)=>({v,label})) });
// Independent implementations from Adobe's public Channel/Distort descriptions.
// Geometry is sampled from transformed composition buffers, not AE source pixels.
const mapSetup = `
 vec4 original=texture(u_tex,v_st);
 vec3 ownerH=u_layerUV*vec3(v_px,1.);vec2 owner=ownerH.xy/ownerH.z;
 vec2 mapPoint=owner;
 if(u_behavior<.5)mapPoint=(owner-.5)*u_layerRes/u_mapRes+.5;
 if(u_behavior>1.5)mapPoint=fract((owner-.5)*u_layerRes/u_mapRes+.5);
 vec4 m=sample_map(mapPoint);
 vec3 rgb=m.a>1e-7?m.rgb/m.a:vec3(0.);
 float hi=max(max(rgb.r,rgb.g),rgb.b),lo=min(min(rgb.r,rgb.g),rgb.b),delta=hi-lo;
 float light=(hi+lo)*.5,hue=0.,sat=0.;
 if(delta>1e-7){sat=delta/max(1e-7,1.-abs(2.*light-1.));hue=hi==rgb.r?mod((rgb.g-rgb.b)/delta,6.):hi==rgb.g?(rgb.b-rgb.r)/delta+2.:(rgb.r-rgb.g)/delta+4.;hue=fract(hue/6.);}
 float channels[9]=float[9](rgb.r,rgb.g,rgb.b,m.a,dot(rgb,vec3(.2126,.7152,.0722)),hue,light,sat,.5);
`;

const ORIGINAL_LAYER_EFFECTS: EffectDefinition[] = [
  {
    id:'setMatte',label:'Set Matte',group:'Channel',viewportSafe:false,
    params:[layer('map','Take Matte From Layer'),channel('channel','Use For Matte',3),toggle('invert','Invert Matte',false),toggle('stretch','Stretch Matte To Fit',true),toggle('composite','Composite Matte With Original',true),toggle('premultiply','Premultiply Matte Layer',false)],
    frag:mapSetup.replaceAll('u_behavior','(u_stretch>.5?1.:0.)')+`
      if(u_has_map<.5){o=original;return;}
      float a=channels[int(clamp(round(u_channel),0.,7.))];
      if(u_premultiply>.5&&int(round(u_channel))!=3)a*=m.a;
      a=clamp(a,0.,1.);if(u_invert>.5)a=1.-a;
      if(u_composite>.5)a*=original.a;
      if(any(lessThan(owner,vec2(0.)))||any(greaterThan(owner,vec2(1.))))a=0.;
      o=vec4((original.a>1e-7?original.rgb/original.a:vec3(0.))*a,a);`,
  },
  {
    id:'displacementMap',label:'Displacement Map',group:'Distort',viewportSafe:false,
    params:[layer('map','Displacement Map Layer'),channel('horizontal','Use For Horizontal Displacement',0),number('maxHorizontal','Max Horizontal Displacement',5,-2000,2000,'px'),channel('vertical','Use For Vertical Displacement',1),number('maxVertical','Max Vertical Displacement',5,-2000,2000,'px'),{...number('behavior','Displacement Map Behavior',0,0,2),options:[{v:0,label:'Center Map'},{v:1,label:'Stretch Map To Fit'},{v:2,label:'Tile Map'}]},toggle('wrap','Wrap Pixels Around',false),toggle('expand','Expand Output',false)],
    frag:mapSetup+`
      if(u_has_map<.5){o=original;return;}
      float x=channels[int(clamp(round(u_horizontal),0.,8.))],y=channels[int(clamp(round(u_vertical),0.,8.))];
      vec2 offset=(clamp(vec2(x,y),0.,1.)*2.-1.)*vec2(u_maxHorizontal,u_maxVertical);
      vec2 q=owner+offset/u_layerRes;
      if(u_wrap>.5)q=fract(q);
      vec3 px=u_ownerFromUV*vec3(q,1.);vec2 st=px.xy/px.z/u_res;st.y=1.-st.y;
      o=texture(u_tex,st);
      if(u_wrap<.5&&(any(lessThan(q,vec2(0.)))||any(greaterThan(q,vec2(1.)))))o=vec4(0.);
      if(u_expand<.5&&(any(lessThan(owner,vec2(0.)))||any(greaterThan(owner,vec2(1.)))))o=vec4(0.);
      if(any(lessThan(st,vec2(0.)))||any(greaterThan(st,vec2(1.))))o=vec4(0.);`,
  },
];


/** Byte maps have a representable neutral 128; float maps use exact 0.5.
 * Preserve revision 1's original continuous mapping for saved projects. */
export const LAYER_EFFECTS:EffectDefinition[]=ORIGINAL_LAYER_EFFECTS.map(def=>def.id!=='displacementMap'?def:{
 ...def,version:2,legacy:[{...def,version:1}],
 frag:def.frag.replace(
  'vec2 offset=(clamp(vec2(x,y),0.,1.)*2.-1.)*vec2(u_maxHorizontal,u_maxVertical);',
  `vec2 value=clamp(vec2(x,y),0.,1.),mapDelta=value-u_mapMidpoint;
   vec2 amount=mapDelta/mix(vec2(u_mapMidpoint),vec2(1.-u_mapMidpoint),step(vec2(u_mapMidpoint),value));
   if(int(round(u_horizontal))==8)amount.x=0.;if(int(round(u_vertical))==8)amount.y=0.;
   vec2 offset=amount*vec2(u_maxHorizontal,u_maxVertical);`),
});
