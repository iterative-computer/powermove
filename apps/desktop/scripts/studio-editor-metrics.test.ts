import {expect,it} from 'vitest';
import {distribution,presentationMetrics,memoryTrend} from './studio-editor-metrics.mjs';
it('counts changed composites, including pauses and repeated old native frames',()=>{
 const rows=Array.from({length:120},(_,i)=>({at:i*1000/60,signature:String(Math.floor(i/2))}));
 const m=presentationMetrics(rows,0,2000);expect(m.presentations).toBe(60);expect(m.fps).toBe(30);expect(m.droppedFrames).toBe(60);expect(m.pass).toBe(false);
 expect(distribution([]).p95).toBeNull();expect(distribution([1,2,100]).p95).toBe(100);
});
it('fails an increasing retained-memory trend and accepts a plateau',()=>{
 expect(memoryTrend(Array.from({length:30},(_,i)=>({at:i*60000,rssMiB:1000+i*12}))).pass).toBe(false);
 expect(memoryTrend(Array.from({length:30},(_,i)=>({at:i*60000,rssMiB:1000+i%2}))).pass).toBe(true);
});
