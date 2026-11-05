/* Frozen runner with a synthetic multiplexed peer. No account or provider is
 * launched. Inputs/outputs and causal positions remain exact. */
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),crypto=require('node:crypto');
const {execFileSync}=require('node:child_process'),{EventEmitter}=require('node:events'),{PassThrough}=require('node:stream');
const root=execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
const base=path.join(root,'apps/native/crates/agents');
const loader=fs.readFileSync(path.join(base,'scripts/capture-oracle.cjs'),'utf8');
const oracle=vm.runInNewContext(loader.slice(0,loader.lastIndexOf('(async()=>'))+'\n;({load})',{require,Buffer,TextDecoder,process,console,setImmediate,setTimeout,clearTimeout,queueMicrotask});
const frames=[]; let after=0;
const emit=(kind,value)=>frames.push({kind,value:JSON.parse(JSON.stringify(value).replace(/\/tmp\/powermove-codex-app-[A-Za-z0-9_-]+/g,'/workspace/agent')),after});
const stimulus=(kind,value)=>{after++;emit(kind,value);};
class Fake extends EventEmitter {
  constructor(){super();this.stdin=new PassThrough();this.stdout=new PassThrough();this.stderr=new PassThrough();this.killed=false;let pending='';this.stdin.on('data',chunk=>{pending+=chunk;let n;while((n=pending.indexOf('\n'))>=0){emit('out',JSON.parse(pending.slice(0,n)));pending=pending.slice(n+1);}});}
  send(value){stimulus('in',value);this.stdout.write(JSON.stringify(value)+'\n');}
  kill(){this.killed=true;return true;}
}
const tick=()=>new Promise(r=>setImmediate(r));
async function until(predicate){for(let i=0;i<10000;i++){if(predicate())return;await tick();}throw Error('Synthetic runner did not reach boundary');}
const methods=name=>frames.filter(f=>f.kind==='out'&&f.value.method===name);
const fixture=path.join(base,'fixtures/concurrent.json');
(async()=>{
  const child=new Fake();const {CodexAppServerRunner}=oracle.load('main/codex/app-server-runner');
  const runner=new CodexAppServerRunner({discoverBinary:async()=>'/fake/codex',prepareHome:async()=>'/tmp/synthetic-runtime-home',spawnProcess:()=>child,requestTimeoutMs:5000,turnTimeoutMs:5000});
  const done=[];
  const start=id=>{
    const request={id,threadId:`pm-${id}`,mode:'editor',prompt:`Synthetic ${id}`,schema:{type:'object'},images:[],access:'editor',projectId:'oracle-project',projectName:'Synthetic',projectJSON:null,attachments:[],consentToken:null};
    stimulus('run',{id,cwd:'/workspace/agent',prompt:request.prompt,schema:request.schema,model:null,reasoningEffort:null,imagePaths:[],config:{mcp_servers:{}},resumeThread:null});
    done.push(runner.run(request,{userData:'/tmp/synthetic',onTrace:value=>emit('trace',{run:id,value}),onProgress:value=>emit('progress',{run:id,value})}).then(value=>emit('result',{run:id,value})));
  };
  start('z-first');await until(()=>methods('initialize').length===1);
  start('a-second');await tick();
  child.send({id:1,result:{}});await until(()=>methods('thread/start').length===2);
  child.send({id:2,result:{thread:{id:'thread-z'}}});await until(()=>methods('turn/start').length===1);
  child.send({id:3,result:{thread:{id:'thread-a'}}});await until(()=>methods('turn/start').length===2);
  child.send({id:4,result:{turn:{id:'turn-z'}}});await tick();
  child.send({id:5,result:{turn:{id:'turn-a'}}});await tick();
  child.send({method:'warning',params:{message:'Reconnecting... 1/5'}});await tick();
  child.send({method:'item/tool/requestUserInput',id:'question-z',params:{threadId:'thread-z',turnId:'turn-z',itemId:'same-item',questions:[{id:'choice',question:'Which choice?',options:[]}]}});await tick();
  child.send({method:'item/tool/requestUserInput',id:'question-a',params:{threadId:'thread-a',turnId:'turn-a',itemId:'same-item',questions:[{id:'choice',question:'Which choice?',options:[]}]}});await tick();
  const answer={id:'a-second',itemId:'same-item',answers:{choice:['Second']}};
  stimulus('answer',answer);runner.answer(answer);await tick();
  stimulus('cancel','z-first');const cancelled=runner.cancel('z-first');await until(()=>methods('turn/interrupt').length===1);
  child.send({id:6,result:{}});await cancelled;await until(()=>frames.some(f=>f.kind==='result'&&f.value.run==='z-first'));
  child.send({method:'item/agentMessage/delta',params:{threadId:'thread-a',turnId:'turn-a',itemId:'answer',delta:'Authored second result'}});await tick();
  child.send({method:'item/completed',params:{threadId:'thread-a',turnId:'turn-a',item:{id:'answer',type:'agentMessage',text:'Authored second result'}}});await tick();
  child.send({method:'turn/completed',params:{threadId:'thread-a',turn:{id:'turn-a',status:'completed',error:null}}});
  await Promise.all(done);await runner.shutdown();
  const hash=v=>crypto.createHash('sha256').update(v).digest('hex');
  const sources=['apps/desktop/src/main/codex/app-server-runner.ts','apps/desktop/src/main/codex/connection-status.ts'];
  const value={baseline:'07ed5fa17ee640659cb075f9cad1af2095011d60',sources:sources.map(file=>({file,sha256:hash(execFileSync('git',['show',`v1.2.0:${file}`]))})),captureSha256:hash(fs.readFileSync(__filename)),frames};
  const bytes=JSON.stringify(value,null,2)+'\n';
  if(process.argv.includes('--check')){if(fs.readFileSync(fixture,'utf8')!==bytes)throw Error('Concurrent fixture differs');console.log('Concurrent oracle: identical');}
  else fs.writeFileSync(fixture,bytes);
})().catch(error=>{console.error(error);process.exitCode=1;});
