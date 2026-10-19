import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc } from './install';
import { parseMode } from './flags';
import { DetachedScopes } from './detached';

const module = WebAssembly.compile(readFileSync(fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url))));
async function fixture() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule: () => module, sink: () => {} });
  PM.proj = PM.mkProject({ dur: 10 });
  expect(await doc.whenReady).toBe(true);
  const comp = { id:'detached', fps:30, w:1920, h:1080, layers:Array.from({length:6}, (_,i) => {
    const L = PM.mkLayer('solid', {name:'Layer '+i});
    L.p.opacity = PM.P(i, {kf:[PM.KF(0,i), PM.KF(2,i+10)]});
    return L;
  }) };
  return { PM, doc, comp, cache:new DetachedScopes() };
}
const copy = (v:any) => JSON.parse(JSON.stringify(v));
const body = (v:any) => { const {reads,...answer} = v; return answer; };

describe('detached input reads through real pm-wasm', () => {
  it('sends nothing for unchanged reads, patches changed inputs, and resets structural edits', async () => {
    const { PM, doc, comp, cache } = await fixture();
    const address = cache.address(comp), wasm = doc.wasm!;
    const request = {op:'ev',comp:address,open:PM.proj.compId,layer:0,key:'opacity',t:0.5};
    const sent:any[]=[];
    const real=wasm.query.bind(wasm);
    const query = vi.spyOn(wasm,'query').mockImplementation(request => { sent.push(copy(request)); return real(request); });
    const ask = () => cache.query(address,request,wasm,doc.barrier!.revision);
    const first = ask();
    expect(first.reads).toContainEqual(['layers',0,'p','opacity','kf',0,'v']);
    query.mockClear(); sent.length=0;
    expect(ask()).toEqual(first);
    expect(query).not.toHaveBeenCalled();
    comp.layers[5].p['position.x'].v = 1000;
    expect(ask()).toEqual(first);
    expect(query).not.toHaveBeenCalled();
    comp.layers[0].p.opacity.kf[0].v = 99;
    expect(ask().ok).not.toBe(first.ok);
    const payload = sent[0] as any;
    expect(payload.compJson).toBeUndefined();
    expect(payload.scopePatches.length).toBeGreaterThan(0);
    expect(payload.scopePatches.every((p:any) => p.path.length > 0)).toBe(true);
    query.mockClear(); sent.length=0;
    comp.layers.reverse();
    ask();
    expect((sent[0] as any).compJson.layers).toHaveLength(6);
  });

  it('matches fresh serialization after 1,200 deterministic arbitrary-reference mutations', async () => {
    const { PM, doc, comp, cache } = await fixture();
    const address = cache.address(comp), wasm=doc.wasm!;
    let seed = 0x71f00d;
    const rand = () => { seed ^= seed<<13; seed ^= seed>>>17; seed ^= seed<<5; return (seed>>>0)/4294967296; };
    const int = (n:number) => Math.floor(rand()*n);
    const original = comp.layers[0].p.opacity;
    for (let n=0;n<1200;n++) {
      const i=int(comp.layers.length), L=comp.layers[i], p=L.p.opacity;
      switch (n%20) {
        case 0: p.v=int(100); break;
        case 1: if(p.kf.length) p.kf[int(p.kf.length)].v=int(100); break;
        case 2: p.kf.push(PM.KF(rand()*3,int(100))); break;
        case 3: p.kf.splice(int(p.kf.length),1); break;
        case 4: p.kf.reverse(); break;
        case 5: p.expr=rand()>.5?'value + time':'time < 0.6 ? value : value * 2'; break;
        case 6: delete p.expr; break;
        case 7: L.from=rand(); break;
        case 8: L.parent=rand()>.5?comp.layers[(i+1)%comp.layers.length].id:null; break;
        case 9: comp.fps=[24,30,60][int(3)]!; break;
        case 10: comp.layers.reverse(); break;
        case 11: comp.layers.splice(i,1,copy(L)); break;
        case 12: if(comp.layers.length<9) { const a=copy(PM.mkLayer('solid')); a.p.opacity=PM.P(20); comp.layers.push(a); } break;
        case 13: if(comp.layers.length>2) comp.layers.splice(i,1); break;
        case 14: L.p.opacity=copy(p); break;
        case 15: p.kf=copy(p.kf); break;
        case 16: p.kf.forEach((k:any)=>k.hold=!k.hold); break;
        case 17: original.v=int(100); original.kf.forEach((k:any)=>k.v=int(100)); break;
        case 18: comp.w=int(4000)+1; break;
        case 19: L.p['unread'+int(10)]=PM.P(int(100)); break;
      }
      for (const op of ['ev','active','worldMatrix']) {
        const request={op,comp:address,open:PM.proj.compId,layer:0,key:'opacity',t:0.5};
        const got=body(cache.query(address,request,wasm,doc.barrier!.revision));
        const fresh=body(wasm.query({...request,comp:'@detached',compJson:copy(comp)}));
        expect(got, JSON.stringify({n,op,seed})).toEqual(fresh);
      }
    }
  }, 120_000);

  it('reuses the scope across times and operations without full serialization', async () => {
    const { PM, doc, comp, cache }=await fixture();
    const address=cache.address(comp), wasm=doc.wasm!;
    const sent:any[]=[]; const real=wasm.query.bind(wasm);
    vi.spyOn(wasm,'query').mockImplementation(request => {sent.push(copy(request)); return real(request);});
    const req={op:'ev',comp:address,open:PM.proj.compId,layer:0,key:'opacity',t:0.5};
    cache.query(address,req,wasm,doc.barrier!.revision);
    for(const t of [0.7,1,1.5]) cache.query(address,{...req,t},wasm,doc.barrier!.revision);
    cache.query(address,{...req,op:'worldMatrix'},wasm,doc.barrier!.revision);
    expect(sent.filter(r=>'compJson' in r)).toHaveLength(1);
    expect(sent.slice(1).every(r=>!('scopePatches' in r))).toBe(true);
  });

  it('preserves JSON serialization semantics for host objects without retaining stale answers', async () => {
    const { PM, doc, comp, cache }=await fixture();
    const address=cache.address(comp), wasm=doc.wasm!;
    const stamp=new Date('2020-01-01T00:00:00Z');
    comp.layers[0].d.stamp=stamp;
    comp.layers[0].p.opacity.expr='layer.d.stamp';
    const req={op:'ev',comp:address,open:PM.proj.compId,layer:0,key:'opacity',t:0.5};
    for(const year of [2020,2025,2030]) {
      stamp.setUTCFullYear(year);
      expect(body(cache.query(address,req,wasm,doc.barrier!.revision))).toEqual(
        body(wasm.query({...req,comp:'@detached',compJson:copy(comp)})));
    }
  });

  it('keeps the facade exact after reorder, retained keyframe writes, and module reset', async () => {
    const { PM, doc, comp }=await fixture();
    PM.scope=[{}]; PM.curComp=()=>comp;
    const L=comp.layers[0], p=L.p.opacity;
    const ask=()=>doc.api!.evP(L,p,0.5,'opacity');
    expect(ask()).toBe(PM.evP(L,copy(p),0.5,'opacity'));
    expect(ask()).toBe(PM.evP(L,copy(p),0.5,'opacity'));
    p.kf[0].v=100;
    expect(ask()).toBe(PM.evP(L,copy(p),0.5,'opacity'));
    comp.layers.reverse();
    expect(ask()).toBe(PM.evP(L,copy(p),0.5,'opacity'));
    doc.api!.reset();
    expect(ask()).toBe(PM.evP(L,copy(p),0.5,'opacity'));
    expect(doc.failures()).toEqual([]);
  });

  it('validates detached structural answers and preserves explicit list scopes', async () => {
    const { PM, doc, comp }=await fixture();
    PM.scope=[{}]; PM.curComp=()=>comp;
    const [A,B]=comp.layers;
    expect(doc.api!.transformRoots([A.id,B.id])).toEqual([A,B]);
    B.parent=A.id;
    expect(doc.api!.transformRoots([A.id,B.id])).toEqual([A]);
    const G=PM.mkLayer('group',{id:'G'});
    A.group=G.id;
    const own=[G,A];
    expect(doc.api!.groupAncestors(A,own)).toEqual(PM.groupAncestors(A,own));
    expect(doc.api!.groupAncestors(A,own)).toEqual([G]);
  });

  it('preserves sparse-array JSON semantics for retained array mutations', async () => {
    const { PM, doc, comp, cache }=await fixture();
    const address=cache.address(comp), wasm=doc.wasm!;
    const req={op:'ev',comp:address,open:PM.proj.compId,layer:0,key:'opacity',t:0.5};
    cache.query(address,req,wasm,doc.barrier!.revision);
    const kf=comp.layers[0].p.opacity.kf;
    for(const edit of [()=>{delete kf[0];},()=>{kf[0]=undefined;},()=>{kf[0]=()=>1;},()=>{kf[0]=PM.KF(0,33);}]) {
      edit();
      expect(body(cache.query(address,req,wasm,doc.barrier!.revision))).toEqual(
        body(wasm.query({...req,comp:'@detached',compJson:copy(comp)})));
    }
    delete comp.layers[2];
    expect(body(cache.query(address,req,wasm,doc.barrier!.revision))).toEqual(
      body(wasm.query({...req,comp:'@detached',compJson:copy(comp)})));
  });

  it('hydrates newly read inputs after changing expression branches', async () => {
    const { PM, doc, comp, cache }=await fixture();
    const address=cache.address(comp), wasm=doc.wasm!;
    const L=comp.layers[0];
    L.p.opacity.expr='comp.w > 100 ? layer.d.w : layer.d.h';
    comp.w=1920; L.d.w=100; L.d.h=200;
    const req={op:'ev',comp:address,open:PM.proj.compId,layer:0,key:'opacity',t:0.5};
    cache.query(address,req,wasm,doc.barrier!.revision);
    comp.w=10; L.d.h=900;
    const got=body(cache.query(address,req,wasm,doc.barrier!.revision));
    expect(got).toEqual({ok:900});
    expect(got).toEqual(body(wasm.query({...req,comp:'@detached',compJson:copy(comp)})));
  });
});
