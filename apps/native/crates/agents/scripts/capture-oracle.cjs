/* Executes unmodified v1.2.0 TS methods with injected fake processes. Synthetic
 * data only. npm install --prefix work/oracle typescript@5.9.3 first. */
const fs = require('node:fs');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {EventEmitter} = require('node:events');
const {PassThrough} = require('node:stream');
const root = execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
const ts = require(path.join(root,'work/oracle/node_modules/typescript'));
const dest = path.join(root,'apps/native/crates/agents/fixtures');
const baseline='v1.2.0';
const cache = new Map();
function load(file) {
  if (cache.has(file)) return cache.get(file);
  const src=execFileSync('git',['show',`${baseline}:apps/desktop/src/${file}.ts`],{encoding:'utf8'});
  const module={exports:{}}; cache.set(file,module.exports);
  const fallback = new Proxy({}, {get:(_,key) => key === 'LIMITS' ? {} : key === 'POWERMOVE_LIVE_INSPECTION_TOOL_NAMES' ? [] : () => null});
  function req(name) {
    if(name.startsWith('node:')) return require(name);
    if(name.endsWith('/ipc')) return load('shared/ipc');
    if(name.endsWith('/guards')) return {isRecord:v=>!!v&&typeof v==='object'&&!Array.isArray(v),isString:(v,n=Infinity)=>typeof v==='string'&&v.length<=n};
    if(name.endsWith('/trace-format')) return load('main/agent-tools/trace-format');
    if(name.endsWith('/connection-status')) return load('main/codex/connection-status');
    if(name.endsWith('/user-mcp')) return {loadUserMcpServers:async()=>({})};
    if(name.endsWith('/media-tools')) return load('shared/media-tools');
    if(name.endsWith('/image-extension')) return {imageExtension:()=> 'png'};
    if(name.endsWith('/events')) return load('main/claude/events');
    return fallback;
  }
  Function('require','module','exports',ts.transpileModule(src,{compilerOptions:{esModuleInterop:true,module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText)(req,module,module.exports);
  cache.set(file,module.exports);return module.exports;
}
const tick=()=>new Promise(r=>setImmediate(r));
function norm(v) { return JSON.parse(JSON.stringify(v).replace(/\/tmp\/powermove-codex-app-[A-Za-z0-9_-]+/g,'/workspace/agent')); }
async function codex(scenario) {
  const frames=[];const emit=(kind,value)=>frames.push({kind,value:norm(value)});
  class Fake extends EventEmitter {
    constructor(){super();this.stdin=new PassThrough();this.stdout=new PassThrough();this.stderr=new PassThrough();this.killed=false;let pending='';this.stdin.on('data',chunk=>{pending+=chunk;for(;;){const n=pending.indexOf('\n');if(n<0)break;const line=pending.slice(0,n);pending=pending.slice(n+1);const m=JSON.parse(line);emit('out',m);if(typeof m.method!=='string')continue;let result;if(m.method==='initialize')result={};if(m.method==='thread/start')result={thread:{id:'thr_123'}};if(m.method==='turn/start')result={turn:{id:'turn_456',status:'inProgress'}};if(m.method==='turn/interrupt')result={};if(m.method==='turn/steer')result={turnId:'turn_456'};if(result!==undefined)queueMicrotask(()=>this.send({id:m.id,result}));}});}
    send(v){emit('in',v);this.stdout.write(JSON.stringify(v)+'\n');}
    kill(){this.killed=true;return true;}
  }
  const child=new Fake();const {CodexAppServerRunner}=load('main/codex/app-server-runner');
  const runner=new CodexAppServerRunner({discoverBinary:async()=>'/fake/codex',prepareHome:async()=>'/tmp/synthetic-runtime-home',spawnProcess:()=>child,requestTimeoutMs:5000,turnTimeoutMs:5000});
  const req={id:'oracle-run',threadId:'pm-thread',mode:'editor',prompt:'Make a progressive blur effect',schema:{type:'object'},images:[],model:'gpt-6.1-sol',reasoningEffort:'ultra',access:'editor',projectId:'oracle-project',projectName:'Synthetic',projectJSON:null,attachments:[],consentToken:null};
  emit('run',{id:req.id,cwd:'/workspace/agent',prompt:req.prompt,schema:req.schema,model:req.model,reasoningEffort:req.reasoningEffort,imagePaths:[],config:{mcp_servers:{}},resumeThread:null});
  const result=runner.run(req,{userData:'/tmp/synthetic',onTrace:v=>emit('trace',v),onProgress:v=>emit('progress',v)}).then(v=>emit('result',v));
  for(let i=0;i<100&&!frames.some(f=>f.kind==='out'&&f.value.method==='turn/start');i++)await tick();
  const scope={threadId:'thr_123',turnId:'turn_456'};
  if(scenario==='question') {
    child.send({id:'q-rpc',method:'item/tool/requestUserInput',params:{...scope,itemId:'q1',questions:[{id:'style',header:'Style',question:'Which style?',options:[{label:'Soft',description:'Subtle'}],isOther:true,isSecret:false}]}});
    emit('answer',{id:req.id,itemId:'q1',answers:{style:['Soft']}});runner.answer({id:req.id,itemId:'q1',answers:{style:['Soft']}});
    child.send({id:1,method:'mcpServer/elicitation/request',params:scope});
    child.send({id:'unknown',method:'unsupported/request',params:scope});
  }
  if(scenario==='steer'){emit('steer',{id:req.id,prompt:'Make the edge softer',images:[]});await runner.steer({id:req.id,prompt:'Make the edge softer',images:[]});}
  if(scenario==='cancel'){emit('cancel',req.id);await runner.cancel(req.id);}
  else {
    if(scenario==='tools') {
      const items=[
        {id:'cmd',type:'commandExecution',command:'printf first\nprintf second',aggregatedOutput:'one\ntwo',status:'completed'},
        {id:'files',type:'fileChange',changes:[{path:'/staged/a.ts'},{path:'b.ts'},{path:'c.ts'},{path:'d.ts'}],status:'completed'},
        {id:'search',type:'webSearch',query:'motion graphics',status:'completed'},
        {id:'mcp',type:'mcpToolCall',tool:'mcp__powermove__sample_media_frames',arguments:{assetId:'asset-1',times:[0,1],target:'source'},result:{content:[{type:'text',text:'sampled'}]},status:'completed'},
        {id:'dynamic',type:'dynamicToolCall',tool:'read_file',contentItems:[{text:'contents'}],status:'completed'},
        {id:'wait',type:'sleep',durationMs:1500,status:'completed'},
        {id:'collab',type:'collabAgentToolCall',tool:'spawnAgent',prompt:'inspect composition',status:'completed'},
        {id:'view',type:'imageView',path:'/artifacts/frame.png',status:'completed'},
        {id:'computer',type:'computerUse',status:'completed'},
        {id:'image',type:'imageGeneration',status:'completed'},
        {id:'failed',type:'commandExecution',command:'synthetic failure',status:'failed',error:{message:'refused'}}
      ];
      for(const item of items){child.send({method:'item/started',params:{...scope,item}});child.send({method:'item/completed',params:{...scope,item}});}
    }
    if(scenario==='retry') {
      child.send({method:'warning',params:{threadId:'thr_123',message:'Reconnecting... 1/5'}});
      child.send({method:'error',params:{...scope,willRetry:true,error:{message:'stream disconnected'}}});
      child.send({method:'item/reasoning/summaryTextDelta',params:{...scope,delta:'Inspecting the project'}});
    }
    if(scenario==='compact') {
      child.send({method:'item/started',params:{...scope,item:{id:'cmp',type:'contextCompaction'}}});
      child.send({method:'item/completed',params:{...scope,item:{id:'cmp',type:'contextCompaction',status:'completed'}}});
      child.send({method:'model/rerouted',params:{...scope,toModel:'gpt-6.1-sol'}});
    }
    if(scenario==='async-question') {
      const item={id:'async-q',type:'agentMessage',delivery:'async',text:'Choose one',questions:[{title:'Choose a colour',options:['Blue','Red']}]};
      child.send({method:'item/started',params:{...scope,item}});child.send({method:'item/agentMessage/delta',params:{...scope,itemId:'async-q',delta:'Choose one'}});child.send({method:'item/completed',params:{...scope,item}});
    }
    child.send({method:'item/agentMessage/delta',params:{...scope,itemId:'answer',delta:'{"kind":"scene"}'}});
    child.send({method:'item/completed',params:{...scope,item:{id:'answer',type:'agentMessage',text:'{"kind":"scene"}'}}});
    child.send({method:'turn/completed',params:{threadId:scope.threadId,turn:{id:scope.turnId,status:'completed',error:null}}});
  }
  await result;await runner.shutdown();
  fs.writeFileSync(path.join(dest,`codex-${scenario}.json`),JSON.stringify({baseline,source:'apps/desktop/src/main/codex/app-server-runner.test.ts; unmodified runner, FakeAppServer test pattern',frames},null,2)+'\n');
}
async function claude() {
  const frames=[];const emit=(kind,value)=>frames.push({kind,value});
  const {ClaudeRunner}=load('main/claude/runner');
  const runner=Object.create(ClaudeRunner.prototype);
  runner.active=new Map();
  const state={request:{id:'oracle-run'},child:{stdin:{destroyed:false,writableEnded:false,write:s=>emit('out',JSON.parse(s))}},questions:new Map(),approvals:new Map()};
  runner.active.set('oracle-run',state);
  const controls=[{type:'control_request',request_id:'question-1',request:{subtype:'can_use_tool',tool_name:'AskUserQuestion',tool_use_id:'ask-1',input:{questions:[{header:'Colour',question:'Pick a colour?',options:[{label:'Blue',description:'Calm'}],multiSelect:true}]}}},{type:'control_request',request_id:'deny-1',request:{subtype:'can_use_tool',tool_name:'Bash',tool_use_id:'bash-1',input:{command:'pwd'}}},{type:'control_request',request_id:'unsupported',request:{subtype:'other'}}];
  for(const v of controls){emit('in',v);runner.control(state,v,v=>emit('trace',v));if(v.request_id==='question-1'){const ans={id:'oracle-run',itemId:'ask-1',answers:{'ask-1-0':['Blue','Green']}};emit('answer',ans);runner.answer(ans);}}
  fs.writeFileSync(path.join(dest,'claude-control.json'),JSON.stringify({baseline,source:'apps/desktop/src/main/claude/runner.test.ts; unmodified control/answer methods',frames},null,2)+'\n');
  // Execute the shipped shell fixture and parse its actual bytes with the TS parser.
  const fixture=execFileSync('git',['show',`${baseline}:apps/desktop/src/main/claude/__fixtures__/fake-claude.sh`],{encoding:'utf8'});
  const lines=execFileSync('/bin/sh',['-c',fixture],{encoding:'utf8'}).trim().split('\n');
  const events=[];const {ClaudeEventParser}=load('main/claude/events');const parser=new ClaudeEventParser({onTrace:v=>events.push({kind:'trace',value:v}),onProgress:v=>events.push({kind:'progress',value:v})});
  for(const line of lines){events.push({kind:'in',value:JSON.parse(line)});parser.push(Buffer.from(line+'\n'));}parser.finish();
  fs.writeFileSync(path.join(dest,'claude-stream.json'),JSON.stringify({baseline,source:'apps/desktop/src/main/claude/__fixtures__/fake-claude.sh; ClaudeEventParser',frames:events,output:parser.output,text:parser.text},null,2)+'\n');
}
function execTranscript(){
 const frames=[];
 const {CodexEventParser}=load('main/codex/events');
 const parser=new CodexEventParser({onTrace:v=>frames.push({kind:'trace',value:v}),onProgress:v=>frames.push({kind:'progress',value:v})});
 const raw=execFileSync('git',['show',`${baseline}:apps/desktop/src/main/codex/__fixtures__/recorded-transcript.ndjson`],{encoding:'utf8'});
 for(const line of raw.trim().split('\n')){try{frames.push({kind:'in',value:JSON.parse(line)});}catch{}parser.push(Buffer.from(line+'\n'));}parser.finish();
 fs.writeFileSync(path.join(dest,'codex-exec.json'),JSON.stringify({baseline,source:'apps/desktop/src/main/codex/__fixtures__/recorded-transcript.ndjson; CodexEventParser',frames},null,2)+'\n');
}
(async()=>{fs.mkdirSync(dest,{recursive:true});execTranscript();for(const s of ['happy','question','steer','cancel','compact','async-question','tools','retry'])await codex(s);await claude();console.log('Recorded 11 synthetic TS transcripts.');})().catch(e=>{console.error(e);process.exitCode=1;});
