import{tag,evaluate}from './main-oracle.mjs';import fs from'node:fs';
const input=JSON.parse(fs.readFileSync(0,'utf8'));const source=tag('apps/desktop/src/main/agent-tools/bridge.ts');
const method=source.slice(source.indexOf('  private async exportVideo('),source.indexOf('  async callRenderer('));
const {exportVideo}=evaluate(method.replace('  private async exportVideo(','async function exportVideo('),['exportVideo'],{
 randomUUID:()=>input.uuid??'synthetic',withAgentExport:(_,__,fn)=>fn(),EXPORT_WAIT_MS:0,
 setTimeout:(fn,ms)=>({handle:setTimeout(fn,ms),unref(){}}),clearTimeout:timer=>{if(timer)clearTimeout(timer.handle)},
});
const owner={};const context={exportJobs:new Map(),options:{exportDirectory:'/synthetic',exportWaitMs:0,exportProgress:()=>input.progress??null},callRenderer:()=>new Promise(()=>{})};
if(input.jobId)context.exportJobs.set(input.jobId,{id:input.jobId,owner,settled:!!input.result,result:input.result?Promise.resolve(input.result):new Promise(()=>{})});
try{process.stdout.write(JSON.stringify({result:await exportVideo.call(context,{owner,runId:'synthetic'},input.arguments??{})}));}catch(e){process.stdout.write(JSON.stringify({error:e.message}));}
