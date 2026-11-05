// Exact data/dispatch unit contracts from the tag. API sinks are explicit
// recording doubles, as in the original tests; no native gesture UI is claimed.
import { EditGesture } from "../../../js/compat/src/gesture";
import { channelBinding, layerFieldBinding, contentBinding, compositionBinding } from "../../../js/compat/src/frozen/host/binding";
export const controlIDs = ["B568","B569","B570","B610","B611","B612","B613","B614"];
const same = (a: unknown,b: unknown) => {if (JSON.stringify(a) !== JSON.stringify(b)) throw Error(`unit contract differs: ${JSON.stringify(a)} != ${JSON.stringify(b)}`)};
export function controls(id: string): boolean {
  const calls: unknown[] = [], record = (name: string) => (...args: unknown[]) => {calls.push([name,...args]);return "exact-result"};
  const api: any = {edit:{begin:record("begin"),dispatch:record("dispatch"),commit:record("commit"),cancel:record("cancel"),apply:record("apply")},history:{begin:record("history.begin"),commit:record("history.commit"),cancel:record("history.cancel"),do(label: string,fn: ()=>unknown){calls.push(["history.do",label]);return fn()}}};
  if (["B568","B569","B570"].includes(id)) {
    let time = 1;
    const source = {project:{get:()=>({dur:10,fps:20,work:[2,8]})},transport:{time:()=>time},model:{layer:()=>null},anim:{ev:()=>null},util:{clamp:(n:number,a:number,b:number)=>Math.max(a,Math.min(b,n))}};
    const command = (binding: any,value: unknown) => binding.command(value);
    if (id === "B568") {const b = channelBinding(source,"L1","position.x",{label:"X"});time=4.5;same(command(b,20),{type:"set_property",target:"L1",path:"position.x",value:20,time:4.5,mode:"auto",preserveHandEdits:false,markIntent:"human"});}
    if (id === "B569") {same(command(layerFieldBinding(source,"L1","name"),"Title"),{type:"set_layer",target:"L1",patch:{name:"Title"}});same(command(contentBinding(source,"L1","text"),"Hello"),{type:"set_content",target:"L1",patch:{text:"Hello"}});same(command(compositionBinding(source,"fps"),60),{type:"set_composition",patch:{fps:60}});}
    if (id === "B570") {same(command(compositionBinding(source,"workArea.start"),9),{type:"set_composition",patch:{workArea:[7.95,8]}});same(command(compositionBinding(source,"workArea.end"),1),{type:"set_composition",patch:{workArea:[2,2.05]}});}
  } else if (id === "B610") {
    const gesture = new EditGesture(api,{mode:"command",label:"Opacity",origin:"inspector",command:{type:"set_property",target:"L1",path:"opacity",value:0,mode:"auto"}});
    gesture.begin();gesture.write(50);gesture.commit();
    same(calls,[["begin","Opacity",{origin:"inspector"}],["dispatch",{type:"set_property",target:"L1",path:"opacity",value:50,mode:"auto"}],["commit","Opacity"]]);
  } else if (id === "B611") {
    const gesture = new EditGesture(api,{mode:"command",label:"X",command:value=>({type:"set_layer",target:"L1",patch:{name:value}})});
    gesture.begin();gesture.cancel();same(calls,[["begin","X",{origin:"interface"}],["cancel"]]);
  } else if (id === "B612") {
    const gesture = new EditGesture(api,{mode:"command",label:"Toggle",command:{type:"set_layer",target:"L1",patch:{}}});
    same(gesture.once(true),"exact-result");same(calls,[["apply",{type:"set_layer",target:"L1",patch:{},value:true},{label:"Toggle",origin:"interface"}]]);
  } else if (id === "B613" || id === "B614") {
    const mode = id === "B613" ? "local" : "set", gesture = new EditGesture(api,{mode,label:mode === "local" ? "Tool" : "Raw",set:record("set")});
    gesture.begin();gesture.write(3);gesture.commit();gesture.once(4);gesture.cancel();
    same(calls,mode === "local" ? [["set",3],["set",4]] : [["history.begin","Raw"],["set",3],["history.commit","Raw"],["history.do","Raw"],["set",4],["history.cancel"]]);
  } else throw Error("unknown control unit contract");
  return true;
}
