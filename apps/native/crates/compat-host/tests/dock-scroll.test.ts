import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
function setup() {
  const listeners: Record<string, (e:any)=>void> = {}, sent:any[]=[];
  const source=readFileSync(new URL('../src/remote-dom.js',import.meta.url),'utf8');
  const start=source.indexOf('  // Route each momentum/wheel event');
  runInNewContext(source.slice(start,source.indexOf('  const panelLayouts =',start)), {
    document:{addEventListener:(kind:string,fn:any)=>listeners[kind]=fn},
    getComputedStyle:(node:any)=>({overflowY:node.overflow ?? 'auto'}), call:(event:any)=>sent.push(event),
  });
  const root:any={scrollHeight:200,clientHeight:200,scrollTop:0,getAttribute:()=> 'notes',getBoundingClientRect:()=>({top:100})};
  root.closest=()=>root;
  const inner:any={scrollHeight:500,clientHeight:100,scrollTop:0,parentElement:root,closest:()=>root,getBoundingClientRect:()=>({top:450,height:24})};
  const wheel=(deltaY:number,extra={})=>{const e:any={target:inner,deltaY,deltaMode:0,preventDefault(){this.defaultPrevented=true},...extra};listeners.wheel(e);return e};
  return {root,inner,sent,wheel,listeners};
}
test('inner list takes wheel and momentum until its bottom edge',()=>{
 const h=setup(); h.wheel(30); expect(h.sent).toHaveLength(0);
 h.inner.scrollTop=400; expect(h.wheel(30).defaultPrevented).toBe(true);
 expect(h.sent[0].payload).toEqual({id:'notes',delta:30});
});
test('reverse hands off at top but inner list takes reverse while it has room',()=>{
 const h=setup();h.inner.scrollTop=10;h.wheel(-5);expect(h.sent).toHaveLength(0);
 h.inner.scrollTop=0;h.wheel(-5);expect(h.sent[0].payload.delta).toBe(-5);
});
test('panel root takes wheel after nested list reaches edge',()=>{
 const h=setup();h.root.scrollHeight=600;h.inner.scrollTop=400;h.wheel(20);expect(h.sent).toHaveLength(0);
 h.root.scrollTop=400;h.wheel(20);expect(h.sent).toHaveLength(1);
});
test('line and page wheel units normalize and modified gestures pass through',()=>{
 const h=setup();h.inner.scrollTop=400;h.wheel(2,{deltaMode:1});h.wheel(1,{deltaMode:2});
 expect(h.sent.map(e=>e.payload.delta)).toEqual([32,200]);
 for (const modifier of ['ctrlKey','metaKey','shiftKey','defaultPrevented'])h.wheel(2,{[modifier]:true});
 expect(h.sent).toHaveLength(2);
});
test('focus forwards panel-relative bounds for native reveal',()=>{
 const h=setup();h.listeners.focusin({target:h.inner});
 expect(h.sent[0]).toEqual({kind:'dock_reveal',panel:0,payload:{id:'notes',top:350,height:24}});
});
