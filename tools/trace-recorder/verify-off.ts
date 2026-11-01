/** Build both modes and compare a small behaviour probe through the same hidden harness. */
import {execFileSync} from 'node:child_process';
import path from 'node:path';
const ref=path.resolve(process.argv[2]??'/tmp/pm-ref-v1.2.0');
delete process.env.ELECTRON_RUN_AS_NODE;
delete process.env.POWERMOVE_E2E_LIVE;
const {launchApp}=await import(path.join(ref,'apps/desktop/e2e/helpers/app.trace.mjs'));
const probe=()=>{
  const PM=(window as any).PM;
  const project=PM.mkProject({name:'Synthetic off-mode fixture'});project.created=0;
  PM.replaceProject(project);PM.hist.clear();
  const output:any[]=[];
  const held=PM.proj;
  output.push(PM.hist.begin('outer')===PM.hist.begin('inner'));
  output.push(PM.hist.do('fold',()=>{PM.proj.name='folded';return 7;}));
  output.push(PM.hist.commit(),PM.hist.commit());
  output.push(PM.hist.undo(),PM.proj.name,PM.hist.redo(),PM.proj.name);
  PM.hist.begin('cancel');PM.proj.name='kept';output.push(PM.hist.cancel(),PM.proj.name);
  PM.Edit.begin('live');let ran=false;
  output.push(PM.Edit.mutate('refused',()=>{ran=true;}),ran,PM.Edit.cancel());
  const sentinel={};output.push(PM.hist.do('identity',()=>sentinel)===sentinel);
  return {output,projectRetained:held===PM.proj,history:PM.hist.list(),tracePresent:!!(window as any).__PMTrace};
};
const results:any[]=[];
for(const flag of ['1','0']){
  const env={...process.env,POWERMOVE_TRACE_RECORDER:flag};
  execFileSync('bun',['run','build'],{cwd:path.join(ref,'apps/desktop'),env,maxBuffer:10*1024*1024,stdio:'pipe'});
  const session=await launchApp({env:{POWERMOVE_TRACE_RECORDER:flag}});
  try{
    await session.page.evaluate(async()=>{await (window as any).PM.Kernel.loader.builtinsReady;});
    results.push(await session.page.evaluate(probe));
  }finally{await session.close();}
}
if(!results[0].tracePresent||results[1].tracePresent)throw new Error('Instrumentation flag was not respected');
for(const result of results)delete result.tracePresent;
if(JSON.stringify(results[0])!==JSON.stringify(results[1]))throw new Error('Enabled/disabled behaviour differs');
console.log('Enabled and disabled builds: identical probe results; disabled build has no trace controller.');
