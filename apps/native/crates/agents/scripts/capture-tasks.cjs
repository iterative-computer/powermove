/* Execute frozen orchestration with synthetic provider discovery/execution.
 * Inspect the durable reservation before any provider work is acknowledged. */
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),crypto=require('node:crypto');
const {execFileSync}=require('node:child_process');
const root=execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
const base=path.join(root,'apps/native/crates/agents');
const ts=require(path.join(root,'work/oracle/node_modules/typescript'));
const zod=require(path.join(root,'work/oracle/node_modules/zod'));
const hash=v=>crypto.createHash('sha256').update(v).digest('hex');
const cache=new Map(),sources=new Map();
const uuids=['11111111-1111-4111-8111-111111111111','22222222-2222-4222-8222-222222222222','33333333-3333-4333-8333-333333333333'];
let uuid=0;let now=0;
class Clock extends Date {static now(){return now;}}
function load(file){
  if(cache.has(file))return cache.get(file);
  const bytes=execFileSync('git',['show',`v1.2.0:${file}`]);sources.set(file,hash(bytes));
  const module={exports:{}};cache.set(file,module.exports);
  const req=name=>{
    if(name==='zod')return zod;
    if(name==='node:crypto')return {...crypto,randomUUID:()=>{if(uuid>=uuids.length)throw Error('Unexpected UUID');return uuids[uuid++];}};
    if(name.startsWith('node:'))return require(name);
    if(name.startsWith('.'))return load(path.posix.normalize(path.posix.join(path.posix.dirname(file),name))+'.ts');
    throw Error(`Unsupported synthetic dependency ${name}`);
  };
  vm.runInNewContext(ts.transpileModule(bytes.toString(),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,esModuleInterop:true}}).outputText,{module,exports:module.exports,require:req,Date:Clock,setTimeout,clearTimeout,AbortController});
  cache.set(file,module.exports);return module.exports;
}
const destination=path.join(base,'fixtures/tasks.json');
(async()=>{
  const {AgentOrchestrator}=load('apps/desktop/src/main/agent-orchestrator.ts');
  const directory=fs.mkdtempSync(path.join(root,'work/task-oracle-'));
  const requests=[],traces=[];
  const original={id:'root',threadId:'root-thread',projectId:'project',provider:'chatgpt',mode:'autonomous',access:'project',prompt:'Original authored intent',planning:true,images:['image'],attachments:[{id:'a'}],future:{preserve:true}};
  const input={task:' Complete child brief ',role:'test',clientRequestId:'one',target:{providerInstanceId:'claude',model:'fixture-model',options:{reasoningEffort:'high'}}};
  const orchestrator=new AgentOrchestrator({directory,providers:async()=>[{providerInstanceId:'claude',displayName:'Fixture Claude',canRunChildTask:true,constraints:[],models:[{id:'fixture-model',options:[{id:'reasoningEffort',options:[{id:'high'}]}]}]}],execute:async(request)=>{requests.push(request);return new Promise(()=>{});},cancel:async()=>{}});
  void orchestrator.run(original,trace=>traces.push(JSON.parse(JSON.stringify(trace))),()=>{});
  for(let i=0;i<10000&&!requests.length;i++)await new Promise(r=>setImmediate(r));
  if(requests.length!==1)throw Error('Root not started');
  now=1;await orchestrator.call('root','delegate_task',input);
  const queued=traces.find(trace=>trace.kind==='task'&&trace.task.status==='queued');
  if(!queued||requests.length!==2)throw Error('Task was not reserved/started');
  const relative=path.join(hash('project'),hash('project/root-thread')+'.json');
  const archive=JSON.parse(fs.readFileSync(path.join(directory,relative),'utf8'));
  const fixture={baseline:'07ed5fa17ee640659cb075f9cad1af2095011d60',sources:[...sources].map(([file,sha256])=>({file,sha256})),captureSha256:hash(fs.readFileSync(__filename)),root:original,input,ids:{task:uuids[0],thread:uuids[1],run:'subagent-'+uuids[2]},path:path.posix.join('Agent Tasks',...relative.split(path.sep)),queued:queued.task,request:requests[1],archive};
  const bytes=JSON.stringify(fixture,null,2)+'\n';
  if(process.argv.includes('--check')){if(fs.readFileSync(destination,'utf8')!==bytes)throw Error('Task oracle differs');console.log('Task reservation oracle: identical');}else fs.writeFileSync(destination,bytes);
})().catch(error=>{console.error(error);process.exitCode=1;});
