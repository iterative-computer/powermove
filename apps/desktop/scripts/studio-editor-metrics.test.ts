import {expect,it} from 'vitest';
import {distribution,presentationMetrics,memoryTrend,overlappingTasks,inputLatencyMetrics} from './studio-editor-metrics.mjs';
it('matches host time and the changed depth set, excluding stale pictures and unfinished GPU fences',()=>{
 const stale={slot:'',inputAt:1,native:{host:{time:0}}};
 const current={slot:'',inputAt:12,native:{host:{time:1,timings:{hostMs:4}},timestamps:{queuedAt:13,startedAt:14,uploadedAt:15,sentAt:16,textureAt:20,replyAt:21,readyAt:22}}};
 const frames=[{at:15,pixelAt:16,jobs:[stale]}, {at:20,jobs:[current]}, {at:25,pixelAt:30,jobs:[stale,current]}];
 const result=inputLatencyMetrics(frames,[{at:10,kind:'time',time:1},{at:11,kind:'pointer'},{at:31,kind:'pointer'}]);
 expect(result.inputToPixelMs).toMatchObject({count:2,p50:19,p95:20});
 expect(result).toMatchObject({matchedInputs:2,totalInputs:3});
 expect(Object.fromEntries(Object.entries(result.pipelineMs).map(([key,value]:any)=>[key,value.p95]))).toEqual({inputToPatch:2,patchToRequest:1,queue:1,upload:1,ipcHostSurface:5,host:4,surfaceToReady:2,readyToComposite:3,compositeToFence:5});
});
it('does not mistake a new compositor time for a new native playback image',()=>{
 const result=inputLatencyMetrics([{at:20,pixelAt:21,time:5,jobs:[{slot:'',inputAt:12,time:5,native:{host:{time:0}}}]}],[{at:10,kind:'time',time:1}]);
 expect(result.matchedInputs).toBe(0);expect(result.pipelineMs).toEqual({});
});
it('counts a phase-start stall even when its browser task started before the action timestamp',()=>{
 const previous={at:-100,ms:50},play={at:0,ms:80},drag={at:100,ms:60},later={at:200,ms:70};
 expect(overlappingTasks([previous,play,drag,later],.1,200)).toEqual([play,drag]);
 expect(overlappingTasks([play],80,100)).toEqual([]);
});
it('counts changed composites, including pauses and repeated old native frames',()=>{
 const rows=Array.from({length:120},(_,i)=>({at:i*1000/60,signature:String(Math.floor(i/2))}));
 const m=presentationMetrics(rows,0,2000);expect(m.presentations).toBe(60);expect(m.fps).toBe(30);expect(m.droppedFrames).toBe(60);expect(m.pass).toBe(false);
 expect(distribution([]).p95).toBeNull();expect(distribution([1,2,100]).p95).toBe(100);
});
it('fails an increasing retained-memory trend and accepts a plateau',()=>{
 expect(memoryTrend(Array.from({length:30},(_,i)=>({at:i*60000,rssMiB:1000+i*12}))).pass).toBe(false);
 expect(memoryTrend(Array.from({length:30},(_,i)=>({at:i*60000,rssMiB:1000+i%2}))).pass).toBe(true);
});
