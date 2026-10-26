import type {EffectDefinition} from 'powermove';
/** Independent geometric stroke; references live outside animation channels. */
const STROKE_V1:EffectDefinition={id:'stroke',label:'Stroke',group:'Generate',viewportSafe:false,
 sourceProcessing:{padding:['brushSize'],factor:.5,pixels:1},
 params:[{k:'path',label:'Path',type:'path',def:'',pathWidth:'brushSize',advanced:false},
 {k:'color',label:'Color',type:'color',def:'#FFFFFF',advanced:false},
 {k:'brushSize',label:'Brush Size',def:8,min:0,max:1000,unit:'px',advanced:false},
 {k:'opacity',label:'Opacity',def:100,min:0,max:100,unit:'%',advanced:false},
 {k:'paintStyle',label:'Paint Style',def:0,min:0,max:2,advanced:false,options:[{v:0,label:'On Original'},{v:1,label:'On Transparent'},{v:2,label:'Reveal Original'}]}],
 frag:`vec4 original=texture(u_tex,v_st);if(u_has_path<.5){o=original;return;}
 float a=sample_path(vec2(v_st.x,1.-v_st.y)).a*clamp(u_opacity/100.,0.,1.);vec4 ink=vec4(u_color*a,a);
 o=u_paintStyle>1.5?original*a:u_paintStyle>.5?ink:ink+original*(1.-a);`,
};

/** Revision 1 keeps its original Canvas outline and width-only bounds. */
export const STROKE:EffectDefinition={...STROKE_V1,version:2,legacy:[{...STROKE_V1,version:1}],
 sourceProcessing:{geometry:['path']},
 params:STROKE_V1.params.map(p=>p.type==='path'?{...p,pathBrush:{hardness:'hardness',spacing:'spacing',start:'start',end:'end',allMasks:'allMasks',sequential:'sequential'}}:p).concat([
 {k:'hardness',label:'Brush Hardness',def:100,min:0,max:100,unit:'%',advanced:false},
 {k:'spacing',label:'Spacing',def:0,min:0,max:400,unit:'%',advanced:false},
 {k:'start',label:'Start',def:0,min:0,max:100,unit:'%',advanced:false},
 {k:'end',label:'End',def:100,min:0,max:100,unit:'%',advanced:false},
 {k:'allMasks',label:'All Masks',type:'toggle',def:false,advanced:false},
 {k:'sequential',label:'Stroke Sequentially',type:'toggle',def:false,advanced:false},
 ] as EffectDefinition['params']),
};
