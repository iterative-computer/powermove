export function selectionChannels(PM:any,layer:any,paths:string[]) {
  return paths.flatMap(path=>PM.inspectorTargets?PM.inspectorTargets(layer,path):[{layer,path,prop:PM.findProp(layer,path)}]);
}
export function animateSelection(PM:any,layer:any,paths:string[],disable:boolean,time:number,label:string,fallback?:any) {
  const commands:any[]=[];
  for(const target of selectionChannels(PM,layer,paths)){
    const p=target.prop,value=p?PM.evP(target.layer,p,time,target.path):fallback;
    if(value===undefined)continue;
    if(disable)commands.push({type:'replace_keyframes',target:target.layer.id,path:target.path,keyframes:[],preserveHandEdits:false});
    commands.push({type:'set_property',target:target.layer.id,path:target.path,value,time,mode:disable?'static':'keyframe',preserveHandEdits:false});
  }
  return (PM.inspectorApply||PM.Edit.apply)(commands,{label,origin:'inspector'});
}
