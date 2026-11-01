// Bundled for real V8, never run under a DOM or Node. Hosts below are explicit
// service/recorder doubles; these assertions certify no native pixels or panels.
import { CompatRealm, RecorderCanvas } from "../../../js/compat/src";
import { RemoteDOM } from "../../../js/remote-dom/src/shim";
import type { RecorderHost, SurfaceVersion } from "../../../js/compat/src/canvas";
const assert = (condition: unknown, message: string) => { if (!condition) throw Error(message); };
let remoteEvidence: unknown;
let remoteReceive: ((message: any) => void) | undefined;
export function run(id: string): boolean {
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
