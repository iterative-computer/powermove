import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
function setup() {
  const source = readFileSync(new URL('../src/remote-dom.js', import.meta.url), 'utf8');
  const start = source.indexOf('  // Measure contribution content');
  const frames: (() => void)[] = [], sent: any[] = [];
  const inner: any = {scrollHeight:900, clientHeight:100, getBoundingClientRect:()=>({bottom:250}), style:{overflowY:'auto',position:'relative'}};
  const root: any = {scrollHeight:200, getBoundingClientRect:()=>({top:100}),querySelectorAll:()=>[inner], getAttribute:()=> 'easing'};
  const ctx: any = {document:{body:{},querySelectorAll:()=>[root],addEventListener(){}}, MutationObserver:class {observe(){}},requestAnimationFrame:(f:any)=>frames.push(f),getComputedStyle:(node:any)=>node.style, call:(event:any)=>sent.push(event)};
  runInNewContext(source.slice(start, source.indexOf('  // End contribution content measurement.', start)) + ';globalThis.measure = measureContributions;', ctx);
  return {inner,root,sent,frames,context:ctx,tick:()=>frames.splice(0).forEach(f=>f())};
}
test('compat measures nested list content rather than its squeezed viewport',()=>{
  const h=setup();h.tick();expect(h.sent[0].payload).toEqual({id:'easing',height:950});
});
test('content measurement coalesces, ignores floating controls and sends only changes',()=>{
  const h=setup();h.inner.style.position='fixed';h.tick();expect(h.sent[0].payload.height).toBe(200);
  h.context.measure();h.context.measure();
  expect(h.frames).toHaveLength(1);h.tick();expect(h.sent).toHaveLength(1);
  h.root.scrollHeight=400;h.context.measure();h.tick();expect(h.sent[1].payload.height).toBe(400);
});
