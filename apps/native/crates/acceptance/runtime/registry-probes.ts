// Public v1–3 contribution contracts, executed in native V8. No DOM is needed.
import { CompatRealm } from "../../../js/compat/src";
const assert = (ok: unknown, label: string) => { if (!ok) throw Error(label); };
const refuses = (action: () => unknown) => { let threw = false; try { action(); } catch { threw = true; } assert(threw, "invalid contribution refused"); };
export const registryIDs = ["A660", "A661", "A662", "A663", "A666", "A668", "A669", "A678", "A693", "A695", "A696", "A698"];
export async function registries(id: string): Promise<boolean> {
  for (const version of [1, 2, 3] as const) {
    const errors: any[] = [];
    const realm = new CompatRealm({ id: `registry-${version}`, generation: 1,
      initial: { revision: 0, document: {id:"synthetic",layers:[],comps:{}}, selection:{layers:[],keys:[]}, workspace:{}, facts:{historyBusy:false,canUndo:false,canRedo:false,editLive:false} },
      host: { call(request){assert(request.method === "realm.failed", "unexpected kernel call"); return {}}, query(){throw Error("unexpected native query")}, scheduleFlush(fn){queueMicrotask(fn)}, reportError(e){errors.push(e)}, pinGeneration(){}, unpinGeneration(){} } });
    const vars = {authored:"λ",empty:"",invalid:7} as any;
    const first = realm.extension("first", version, {vars}), second = realm.extension("second", version);
    try {
      if (id === "A660") {
        const api = first.api.keybindings, other = second.api.keybindings;
        refuses(() => api.bind({key:"",command:"one"})); refuses(() => api.bind({key:"x",command:""}));
        const base = api.bind({key:"Command+Shift+X",command:"one",args:[{exact:1.23456789012345}],priority:12,inFields:true,repeat:true});
        const held = other.list()[0];
        assert(held.chord === "cmd+shift+x" && held.args[0].exact === 1.23456789012345 && held.priority === 12 && held.inFields && held.repeat, "normalized binding retains flags and arguments");
        const top = other.bind({key:"cmd+shift+x",command:"one",priority:1});
        assert(api.list().length === 1 && api.list()[0].priority === 1, "binding override");
        api.unbind("cmd+shift+x"); assert(api.list().length === 1, "unbind respects active owner");
        top.dispose(); assert(api.list()[0] === held, "restore binding identity");
        assert(api.chordOf({key:"X",code:"KeyX",metaKey:true,shiftKey:true,altKey:false,ctrlKey:false}) === "cmd+shift+x", "event chord");
        api.unbind("cmd+shift+x"); base.dispose(); assert(api.list().length === 0, "unbind and stale disposal");
        api.bind({key:"x",command:"one"}); other.unbind("x",true); assert(api.list().length === 0, "explicit all-owner unbind");
        api.bind({key:"y",command:"owned"}); first.dispose(); assert(other.list().length === 0, "owner binding disposal");
      } else if (["A661", "A662", "A663"].includes(id)) {
        const family = id === "A661" ? "effects" : id === "A662" ? "transitions" : "layers";
        const api = first.api[family], other = second.api[family];
        const definition = id === "A663"
          ? {id:"synthetic.layer",label:"Synthetic",version:1,params:[],defaults:{exact:1.23456789012345},renderer:{kind:"fragment",fragment:"void main(){}"}}
          : {id:"Synthetic.effect",label:"Synthetic",group:"Synthetic",params:[{k:"amount",def:1}],frag:"outColor=vec4(1.);"};
        assert(api.get("missing") === undefined, "missing registry definition");
        refuses(() => api.register({...definition,id:""}));
        refuses(() => api.register({...definition,params:[{k:"amount"},{k:"amount"}]}));
        const base = api.register(definition);
        assert(other.get(definition.id) === definition && other.list()[0] === definition, "exact registered definition identity");
        const replacement = {...definition,label:"Replacement"}, top = other.register(replacement);
        assert(api.get(definition.id) === replacement, "definition override");
        top.dispose(); top.dispose(); assert(other.get(definition.id) === definition, "definition restoration");
        other.register(replacement); base.dispose(); assert(api.get(definition.id) === replacement, "buried definition disposal");
        second.dispose(); assert(api.list().length === 0, "owner definition disposal");
      } else if (id === "A666") {
        const api = first.api.theme;
        refuses(() => api.register({id:""})); refuses(() => api.register({id:"bad",tokens:{invalid:"synthetic"}}));
        // Authored opaque token values only; this check makes no design choice or pixels.
        const definition = {id:"synthetic",scheme:"dark",tokens:{"--synthetic-token":"authored"}};
        const base = api.register(definition), events: any[] = [];
        second.api.events.on("theme:changed", (e: any) => events.push(e));
        assert(api.list()[0] === definition, "theme identity"); api.activate("synthetic");
        assert(second.api.theme.active() === "synthetic" && events.at(-1).id === "synthetic" && events.at(-1).scheme === "dark", "theme activation and event");
        api.setScheme("light"); assert(second.api.theme.scheme() === "light" && events.at(-1).scheme === "light", "explicit scheme");
        api.setScheme("system"); assert(second.api.theme.scheme() === "system", "retain system scheme without claiming OS resolution");
        const replacement = {...definition,scheme:"light"}, top = second.api.theme.register(replacement);
        assert(api.list()[0] === replacement, "theme override"); top.dispose(); assert(api.list()[0] === definition, "theme restoration");
        base.dispose(); first.dispose(); assert(second.api.theme.list().length === 0, "theme disposal");
      } else if (id === "A668" || id === "A695") {
        for (const location of id === "A695" ? ["titlebar:right","panel:context","layer:context","timeline:context","viewer:context"] : ["synthetic"]) {
        const contributor = realm.extension(`contributor-${location}`,version);
        const api = first.api.menus, context = {exact:1.23456789012345}, calls: string[] = [];
        refuses(() => api.contribute(location, null));
        const item = {label:"Synthetic",run:()=>context};
        const base = api.contribute(location, (ctx: unknown) => {assert(ctx === context, "exact menu context");calls.push("first");return [item]});
        const asyncItem = {label:"Async synthetic"};
        const top = contributor.api.menus.contribute(location, async (ctx: unknown) => {assert(ctx === context, "async menu context");calls.push("second");return [asyncItem]});
        const collected = api.collect(location,context);
        assert(collected.length === 1 && collected[0] === item, "collect excludes asynchronous answer");
        calls.length = 0; const gathered = await api.gather(location,context);
        assert(gathered.length === 2 && gathered[0] === item && gathered[1] === asyncItem && calls.join(":") === "first:second", "gather retains contributor order and identity");
        top.dispose(); top.dispose(); assert((await api.gather(location,context)).length === 1, "menu disposal");
        const thrown = {}; contributor.api.menus.contribute(location, () => {throw thrown});
        assert(api.collect(location,context)[0] === item && errors.length === 1 && errors.pop().error === thrown, "menu exception attribution");
        base.dispose(); contributor.dispose(); assert(api.collect(location,context).length === 0, "owner menu disposal");
              }
      } else if (id === "A669" || id === "A698") {
        const api = first.api.status; let value = "first", clicks = 0;
        refuses(() => api.register({id:"",text:()=>""})); refuses(() => api.register({id:"bad",text:"bad"}));
        const base = api.register({id:"synthetic",side:"left",title:"Authored title",text:()=>value,onClick:()=>++clicks});
        const held = second.api.status.list()[0]; value = "changed";
        assert(held.side === "left" && held.title === "Authored title", "status metadata retained");
        assert(held.text() === "changed" && held.onClick() === 1 && clicks === 1, "live status callbacks");
        const top = second.api.status.register({id:"synthetic",text:()=>"override"});
        assert(api.list()[0].text() === "override", "status override");top.dispose();assert(api.list()[0] === held, "status restoration");
        const error = {}; second.api.status.register({id:"throws",text(){throw error}});
        assert(api.list()[1].text() === null && errors.length === 1 && errors.pop().error === error, "status exception attribution");
        base.dispose();
        const right = api.register({id:"right",side:"right",title:"Authored right title",text:()=>"right value"});
        const entry = api.list().find((item:any)=>item.id === "right"); assert(entry.side === "right" && entry.title === "Authored right title" && entry.text() === "right value", "right status metadata and callback");right.dispose();
        second.dispose(); assert(api.list().length === 0, "status owner disposal");
      } else if (id === "A678" || id === "A696") {
        const api = first.api.inspector; let cleaned = 0;
        refuses(() => api.registerSection({id:"bad",title:"Bad",build(){},after:"invalid"}));
        const context = {exact:"λ"}, target = {replaceChildren(){cleaned++}};
        const base = api.registerSection({id:"synthetic",title:"Synthetic",when:()=>true,build(el: unknown,ctx: unknown){assert(el === target && ctx === context,"inspector callback identities");return ()=>{cleaned++}}});
        const held = second.api.inspector.sections()[0]; assert(held.after === "transform" && held.when(), "inspector default placement and predicate");
        const mounted = held.build(target,context); mounted.dispose(); mounted.dispose(); assert(cleaned === 2,"mount cleanup once");
        held.build(target,context); base.dispose(); base.dispose(); assert(cleaned === 4 && api.sections().length === 0,"registration releases retained mount");
        for (const placement of ["content","transform","effects"]) {
          let visible = false;
          const registration = second.api.inspector.registerSection({id:"placed",title:"Synthetic",after:placement,when:()=>visible,build(){return ()=>{}}});
          const section = api.sections()[0]; assert(section.after === placement && section.when() === false,"explicit section placement and false condition"); visible = true; assert(section.when() === true,"live condition");registration.dispose();
        }
        second.api.inspector.registerSection({id:"owned",title:"Synthetic",build(){}}); second.dispose(); assert(api.sections().length === 0,"owner section disposal");
      } else if (id === "A693") {
        const api = first.api.vars;
        vars.authored = "changed after activation";
        assert(api.get("authored") === "λ" && api.get("empty") === "" && api.get("missing") === undefined, "activation-owned exact variable values");
        assert(api.has("empty") && !api.has("missing") && !api.has("invalid"), "own string-only variable presence");
        const keys = api.keys(); assert(keys.join(":") === "authored:empty", "variable key order"); keys.pop(); assert(api.keys().length === 2 && Object.isFrozen(api),"owned immutable variable API");
        assert(second.api.vars.keys().length === 0, "extension variable isolation");
      } else throw Error("unknown registry acceptance ID");
      assert(errors.length === 0, "no swallowed registry error");
    } finally { first.dispose(); second.dispose(); realm.dispose(); }
  }
  return true;
}
