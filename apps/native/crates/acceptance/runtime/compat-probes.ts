// Bundled for real V8, never run under a DOM or Node. Hosts below are explicit
// service/recorder doubles; these assertions certify no native pixels or panels.
import { CompatRealm, RecorderCanvas } from "../../../js/compat/src";
import { registries, registryIDs } from "./registry-probes";
import { controls, controlIDs } from "./control-probes";
import * as compat from "../../../js/compat/src";
import { RemoteDOM } from "../../../js/remote-dom/src/shim";
import type { RecorderHost, SurfaceVersion } from "../../../js/compat/src/canvas";
const assert = (condition: unknown, message: string) => { if (!condition) throw Error(message); };
Object.assign(globalThis, { PowermoveCompat: { ...compat, RemoteDOM } });
let remoteEvidence: unknown;
let remoteReceive: ((message: any) => void) | undefined;
export function run(id: string): boolean | Promise<boolean> {
  if (registryIDs.includes(id)) return registries(id);
  if (controlIDs.includes(id)) return controls(id);
  if (["A659", "A689", "A691"].includes(id)) {
    for (const version of [1, 2, 3] as const) {
      const errors: unknown[] = [];
      const realm = new CompatRealm({ id: `contract-${version}`, generation: 1,
        initial: { revision: 0, document: {id:"synthetic",layers:[],comps:{}}, selection:{layers:[],keys:[]}, workspace:{}, facts:{historyBusy:false,canUndo:false,canRedo:false,editLive:false} },
        host: { call(request){assert(request.method==="realm.failed","unexpected kernel call");return {}}, query(){throw Error("unexpected query")},scheduleFlush(fn){queueMicrotask(fn)},reportError(error){errors.push(error)},pinGeneration(){},unpinGeneration(){} } });
      const first = realm.extension("first", version), second = realm.extension("second", version);
      if (id === "A659") {
        const value = { authored: 12.3456789012345 }, argument = { keep: true };
        let caught = false;
        try { first.api.commands.register({ id: "", run() {} }); } catch { caught = true; }
        assert(caught, "invalid command refused");
        const base = first.api.commands.register({ id: "synthetic.command", run(arg: unknown) { assert(arg === argument, "exact command argument"); return value; } });
        assert(second.api.commands.has("synthetic.command"), "command lookup");
        assert(second.api.commands.list().some((command: any) => command.id === "synthetic.command"), "command enumeration");
        assert(second.api.commands.run("synthetic.command", argument) === value, "exact synchronous return");
        const top = second.api.commands.register({ id: "synthetic.command", run: () => false });
        assert(first.api.commands.run("synthetic.command") === false, "literal-false command result");
        top.dispose(); top.dispose();
        assert(second.api.commands.run("synthetic.command", argument) === value, "restore prior command");
        base.dispose(); assert(!second.api.commands.has("synthetic.command"), "registration disposal");
        first.api.commands.register({ id: "owned", run: () => 7 }); first.dispose();
        assert(!second.api.commands.has("owned"), "extension owns command lifetime");
        assert(second.api.commands.run("missing") === false, "unknown command declines");
        const error = {};
        second.api.commands.register({ id: "throws", run() { throw error; } });
        assert(second.api.commands.run("throws") === undefined, "guarded command failure return");
        assert(errors.length === 1 && (errors[0] as any).error === error, "command exception identity and owner attribution"); errors.length = 0;
      } else if (id === "A689") {
        const value = { n: 7, call() { return this.n; } };
        assert(first.api.services.get("absent") === null, "missing service");
        const base = first.api.services.register("synthetic", value);
        const held = second.api.services.get("synthetic");
        assert(held === value && held.call() === 7, "live identity and closure");
        const top = second.api.services.register("synthetic", { n: 19 });
        assert(first.api.services.get("synthetic").n === 19 && held === value && held.call() === 7, "held prior implementation survives override");
        base.dispose(); base.dispose(); assert(first.api.services.get("synthetic").n === 19, "buried idempotent disposal");
        top.dispose(); assert(first.api.services.get("synthetic") === null, "top removal after buried disposal");
        first.api.services.register("synthetic", value); second.api.services.register("synthetic", { n: 23 });
        second.dispose(); assert(first.api.services.get("synthetic") === value, "owner disposal restores predecessor");
        let caught = false; try { first.api.services.register("", value); } catch { caught = true; }
        assert(caught, "empty service name refused");
        first.dispose(); assert(realm.services.get("synthetic") === null, "all provider lifetimes released");
      } else {
        const payload = { exact: "λ", f64: 12.3456789012345 }, calls: string[] = [];
        const outer = first.api.events.on("outer", (value: unknown) => { assert(value === payload, "exact event payload"); calls.push("first"); second.api.events.emit("inner", value); });
        const inner = second.api.events.on("inner", (value: unknown) => { assert(value === payload, "reentrant event payload"); calls.push("inner"); });
        second.api.events.on("outer", () => calls.push("second"));
        first.api.events.emit("outer", payload); assert(calls.join(":") === "first:inner:second", "synchronous ordered reentrancy");
        inner.dispose(); inner.dispose(); calls.length = 0;
        first.api.events.emit("outer", payload); assert(calls.join(":") === "first:second", "subscription disposal");
        const error = {};
        first.api.events.on("throws", () => { throw error; });
        second.api.events.on("throws", () => calls.push("after-error"));
        first.api.events.emit("throws", payload);
        assert(calls.at(-1) === "after-error" && errors.length === 1 && (errors[0] as any).error === error, "event failure attribution preserves following subscribers"); errors.length = 0;
        outer.dispose(); first.dispose(); calls.length = 0;
        second.api.events.emit("outer", payload); assert(calls.join(":") === "second", "other owner remains");
        second.dispose(); calls.length = 0; realm.PM.bus.emit("outer", payload); assert(calls.length === 0, "owner cleanup");
      }
      first.dispose(); second.dispose(); realm.dispose();
      assert(errors.length === 0, "no swallowed callback errors");
    }
    return true;
  }
  if (id === "W040.compat-remote-dom") {
    assert(typeof process === "undefined" && typeof document === "undefined", "bare native V8 realm");
    const batches: any[] = [], errors: unknown[] = [];
    let writable = false;
    const dom = new RemoteDOM({ session: { panel: "synthetic", generation: 1 }, realmGlobal: globalThis,
      send(batch) { if (!writable) return false; batches.push(batch); return true; }, reportError(error) { errors.push(error); } });
    dom.installGlobals();
    assert(window === globalThis && window.document === document, "realm global identity");
    const input = document.createElement("input"), output = document.createElement("output");
    document.body.append(input, output);
    input.addEventListener("input", () => { output.textContent = input.value; });
    dom.flush(); assert(batches.length === 0, "refused publication retained");
    writable = true; dom.flush();
    assert(batches.length === 1 && batches[0].seq === 1, "same first batch retried");
    const idBefore = dom.id(input);
    remoteEvidence = structuredClone({ batches, input: idBefore });
    // The Rust router admits the mount and forwards its accepted synthetic
    // page event back into this same V8 realm before we produce the next batch.
    remoteReceive = message => {
      dom.receive(message); dom.flush();
      assert(output.textContent === "Authored λ text", "input state installed before callback");
      assert(dom.id(input) === idBefore, "stable input identity");
      assert(errors.length === 0 && batches.length === 2, "ordered callback publication");
      remoteEvidence = structuredClone({ batches, input: idBefore }); dom.dispose();
    };
    return true;
  }
  if (id === "W037.compat-services") {
    const realm = new CompatRealm({ id: "synthetic", generation: 1,
      initial: { revision: 0, document: {id:"synthetic",layers:[],comps:{}}, selection:{layers:[],keys:[]}, workspace:{}, facts:{historyBusy:false,canUndo:false,canRedo:false,editLive:false} },
      host: { call(request){assert(request.method==="realm.failed","unexpected kernel call");return {}}, query(){throw Error("unexpected query")}, scheduleFlush(fn){queueMicrotask(fn)},reportError(e){throw e},pinGeneration(){},unpinGeneration(){} },
    });
    const one=realm.extension("one",1),two=realm.extension("two",2),three=realm.extension("three",3);
    for (const [extension, version] of [[one,1],[two,2],[three,3]] as const) assert(extension.api.apiVersion===version,"declared apiVersion");
    const identity={value:7,call(){return this.value}};
    one.api.services.register("sync",identity);
    assert(two.api.services.get("sync")===identity,"live service identity");
    assert(two.api.services.get("sync").call()===7,"synchronous closure");
    three.api.services.register("sync",{value:19});
    assert(two.api.services.get("sync").value===19,"latest provider");
    three.dispose();assert(two.api.services.get("sync")===identity,"restore prior provider");
    one.dispose();assert(two.api.services.get("sync")===null,"dispose owns registration");
    two.dispose();realm.dispose();return true;
  }
  if (id === "W040.compat-recorder") {
    let next=0;const surfaces=new Map<string,{version:number,commands:unknown[]}>();const pins=new Map<string,unknown>();
    const host:RecorderHost={create(){const id=`synthetic-${next++}`;surfaces.set(id,{version:0,commands:[]});return id},reset(id){const s=surfaces.get(id)!;s.commands=[];s.version++},command(id,name,args){const s=surfaces.get(id)!;s.commands.push([name,args]);s.version++},read(){throw Error("pixels unavailable")},retainVersion(id){const s=surfaces.get(id)!;const ref={surface:id,version:s.version};pins.set(JSON.stringify(ref),structuredClone(s.commands));return ref},releaseVersion(ref:SurfaceVersion){pins.delete(JSON.stringify(ref))},release(id){surfaces.delete(id)}};
    const source=new RecorderCanvas(host,4,4),target=new RecorderCanvas(host,4,4);
    const context=source.getContext("2d");assert(context===source.getContext("2d"),"context identity");
    context.fillRect(0,0,4,4);target.getContext("2d").drawImage(source,0,0);
    const saved=JSON.stringify([...pins.values()]);source.width=8;source.dispose();
    assert(pins.size===1&&JSON.stringify([...pins.values()])===saved,"immutable source version pin");
    target.dispose();assert(pins.size===0&&surfaces.size===0,"dispose releases pins and surfaces");return true;
  }
  throw Error("unknown compat probe");
}
Object.assign(globalThis, { PowermoveAcceptance: { run, evidence: () => remoteEvidence,
  receive(message: unknown) { const callback = remoteReceive; remoteReceive = undefined;
    if (!callback) throw Error("No pending native router event"); callback(message); return remoteEvidence; } } });
