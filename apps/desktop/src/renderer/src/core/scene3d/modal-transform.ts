export type TransformMode = 'translate'|'rotate'|'scale';
export type TransformAxis = 'x'|'y'|'z';
export interface ModalTransform { mode:TransformMode;axis:TransformAxis|null;plane:boolean;input:string;delta:number;local?:boolean }
export function startTransform(mode:TransformMode):ModalTransform { return {mode,axis:null,plane:false,input:'',delta:0,local:false}; }
/** Blender cycling: the first press uses the preferred space, pressing the same
    constraint again switches to the other space, a third press removes it. */
export function constrainTransform(state:ModalTransform,axis:TransformAxis,plane=false,preferLocal=false):ModalTransform {
  if(state.axis!==axis||state.plane!==plane)return {...state,axis,plane,local:preferLocal};
  if(!!state.local===preferLocal)return {...state,local:!preferLocal};
  return {...state,axis:null,plane:false,local:false};
}
export function typeTransform(state:ModalTransform,key:string):ModalTransform {
  const input=key==='Backspace'?state.input.slice(0,-1):state.input+key;
  return /^-?(?:\d*(?:\.\d*)?)$/.test(input)?{...state,input}:state;
}
export function transformAmount(state:ModalTransform,snap=false):number {
  const typed=state.input!==''&&state.input!=='-'&&state.input!=='.'&&state.input!=='-.';
  let amount=typed?Number(state.input):state.mode==='scale'?1+state.delta:state.delta;
  if(snap&&!typed) {const step=state.mode==='translate'?1:state.mode==='rotate'?15:.1;amount=Math.round(amount/step)*step;}
  if(!Number.isFinite(amount))throw new Error('Enter a finite transform value');
  return state.mode==='scale'?Math.max(.0001,amount):amount;
}
export function transformComponents(state:ModalTransform):TransformAxis[] {
  const axes:TransformAxis[]=['x','y','z'];
  return state.axis?axes.filter(a=>state.plane?a!==state.axis:a===state.axis):axes;
}
