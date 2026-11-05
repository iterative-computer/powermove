/* Synthetic oracle only. Executes frozen c7f46a05641b23944d3692c9d80f9677c7e1ef1d methods, never native output.
 * Run from the repo after npm install --prefix work/oracle typescript@5.9.3.
 * Existing provider recorder functions are reused without running its writer.
 */
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const crypto = require('node:crypto');
const {execFileSync} = require('node:child_process');
const {PassThrough} = require('node:stream');
const {EventEmitter} = require('node:events');
const root = execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
const base = path.join(root,'apps/native/crates/agents');
const committed = path.join(base,'fixtures/main/replay');
const checking = process.argv.includes('--check');
const dest = checking ? fs.mkdtempSync(path.join(root,'work/replay-capture-')) : committed;
const tag = file => execFileSync('git',['show',`c7f46a05641b23944d3692c9d80f9677c7e1ef1d:${file}`],{cwd:root,encoding:'utf8',maxBuffer:8*1024*1024});
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const write = (file,value) => fs.writeFileSync(path.join(dest,file),JSON.stringify(value,null,2)+'\n');
const harness = fs.readFileSync(path.join(base,'scripts/capture-main-oracle.cjs'),'utf8');
// Redirect only the test harness destination, not any tagged code under test.
const prefix = harness.slice(0,harness.lastIndexOf('(async()=>'))
  .replace("const dest = path.join(root,'apps/native/crates/agents/fixtures/main/recorded');",`const dest = ${JSON.stringify(path.join(dest,'providers'))};`);
const oracle = vm.runInNewContext(prefix+'\n;({load,codex,claude,execTranscript})',{
  require,Buffer,TextDecoder,process,console,setImmediate,setTimeout,clearTimeout,queueMicrotask
});
const sources = [
  'apps/desktop/src/main/codex/app-server-runner.ts',
  'apps/desktop/src/main/codex/app-server-runner.test.ts',
  'apps/desktop/src/main/codex/events.ts',
  'apps/desktop/src/main/codex/__fixtures__/recorded-transcript.ndjson',
  'apps/desktop/src/main/claude/runner.ts',
  'apps/desktop/src/main/claude/runner.test.ts',
  'apps/desktop/src/main/claude/events.ts',
  'apps/desktop/src/main/claude/__fixtures__/fake-claude.sh',
  'apps/desktop/src/main/agent-tools/mcp-stdio.mjs',
  'apps/desktop/src/main/agent-tools/mcp-stdio.test.ts',
  'apps/desktop/src/main/agent-tools/external-spec.ts',
  'apps/desktop/src/main/agent-tools/external-session.ts',
  'apps/desktop/src/main/agent-tools/spec.ts',
  'apps/desktop/src/main/agent-tools/trace-format.ts',
  'apps/desktop/src/main/codex/connection-status.ts',
  'apps/desktop/src/shared/ipc.ts',
  'apps/desktop/src/shared/media-tools.ts',
  'apps/desktop/src/renderer/src/legacy/assistant/mcp-project-tools.ts'
];
const cases=[];
function register(name,surface,file) { cases.push({name,surface,file,sha256:hash(fs.readFileSync(path.join(dest,file)))}); }
function claudeControls() {
  const frames=[]; const emit=(kind,value)=>frames.push({kind,value});
  const {ClaudeRunner}=oracle.load('main/claude/runner');
  const runner=Object.create(ClaudeRunner.prototype);runner.active=new Map();
  const state={request:{id:'oracle-run',access:'editor'},child:{stdin:{destroyed:false,writableEnded:false,write:s=>emit('out',JSON.parse(s))}},questions:new Map(),approvals:new Map()};
  runner.active.set('oracle-run',state);
  const send=v=>{emit('in',v);runner.control(state,v,v=>emit('trace',v));};
  const question=id=>({type:'control_request',request_id:`rpc-${id}`,request:{subtype:'can_use_tool',tool_name:'AskUserQuestion',tool_use_id:id,input:{questions:[{question:'Which?',options:[]}]}}});
  send(question('skip'));
  const answer={id:'oracle-run',itemId:'skip',answers:{}};
  emit('answer',answer);runner.answer(answer);
  send(question('withdraw'));
  send({type:'control_cancel_request',request_id:'rpc-withdraw'});
  // A duplicate reply cannot reopen a withdrawn control.
  emit('answer-return',{itemId:'withdraw',answers:{},accepted:runner.answer({id:'oracle-run',itemId:'withdraw',answers:{}})});
  write('providers/claude-skipped-withdrawn.json',{baseline:'c7f46a05641b23944d3692c9d80f9677c7e1ef1d',source:'ClaudeRunner.control/answer; runner.test.ts skipped/withdrawn question test',frames});
  register('claude-skipped-withdrawn','claude','providers/claude-skipped-withdrawn.json');
}
function claudeParser(name,inputs) {
  const frames=[]; const emit=(kind,value)=>frames.push({kind,value});
  const {ClaudeEventParser}=oracle.load('main/claude/events');
  const parser=new ClaudeEventParser({onTrace:v=>emit('trace',v),onProgress:v=>emit('progress',v),onSessionId:v=>emit('session',v),onWarning:v=>emit('warning',v),onResult:()=>emit('close-stdin',true)});
  for(const v of inputs){emit('in',v);parser.push(Buffer.from(JSON.stringify(v)+'\n'));}
  parser.finish();
  // Getter results are captured from the tagged parser, not inferred from Rust.
  write(`providers/${name}.json`,{baseline:'c7f46a05641b23944d3692c9d80f9677c7e1ef1d',source:'ClaudeEventParser with synthetic stream-json',frames,terminal:{output:parser.output??null,text:parser.text,error:parser.error}});
  register(name,'claude',`providers/${name}.json`);
}
async function codexFailure(scenario) {
  const frames=[];
  const emit=(kind,v)=>frames.push({kind,value:JSON.parse(JSON.stringify(v).replace(/\/tmp\/powermove-codex-app-[A-Za-z0-9_-]+/g,'/workspace/agent'))});
  class Fake extends EventEmitter {
    constructor(){super();this.stdin=new PassThrough();this.stdout=new PassThrough();this.stderr=new PassThrough();this.killed=false;
      let pending='';this.stdin.on('data',chunk=>{pending+=chunk;let n;while((n=pending.indexOf('\n'))>=0){const m=JSON.parse(pending.slice(0,n));pending=pending.slice(n+1);emit('out',m);if(!m.method)continue;
        let result=m.method==='thread/start'?{thread:{id:'thr_123'}}:m.method==='turn/start'?{turn:{id:'turn_456'}}:{};
        if(m.method==='initialized')continue;
        if((scenario==='initialize-error'&&m.method==='initialize')||(scenario==='thread-error'&&m.method==='thread/start')||(scenario==='turn-error'&&m.method==='turn/start'))queueMicrotask(()=>this.send({id:m.id,error:{code:-32000,message:'Synthetic provider refusal'}}));
        else {if(scenario==='missing-thread'&&m.method==='thread/start')result={};if(scenario==='missing-turn'&&m.method==='turn/start')result={};queueMicrotask(()=>this.send({id:m.id,result}));}
      }});
    }
    send(v){emit('in',v);this.stdout.write(JSON.stringify(v)+'\n');}
    kill(){this.killed=true;return true;}
  }
  const child=new Fake();
  const {CodexAppServerRunner}=oracle.load('main/codex/app-server-runner');
  const runner=new CodexAppServerRunner({discoverBinary:async()=>'/fake/codex',prepareHome:async()=>'/tmp/synthetic-runtime-home',spawnProcess:()=>child,requestTimeoutMs:5000,turnTimeoutMs:5000});
  const req={id:'oracle-run',threadId:'pm-thread',mode:'editor',prompt:'Synthetic error probe',schema:{type:'object'},images:[],access:'editor',projectId:'oracle-project',projectName:'Synthetic',projectJSON:null,attachments:[],consentToken:null};
  emit('run',{id:req.id,cwd:'/workspace/agent',prompt:req.prompt,schema:req.schema,model:null,reasoningEffort:null,imagePaths:[],config:{mcp_servers:{}},resumeThread:null});
  const done=runner.run(req,{userData:'/tmp/synthetic',onTrace:v=>emit('trace',v),onProgress:v=>emit('progress',v)}).then(v=>emit('result',v));
  if(scenario==='context-exhausted'||scenario==='turn-failed') {
    for(let i=0;i<100&&!frames.some(f=>f.kind==='in'&&f.value.result?.turn);i++)await new Promise(r=>setImmediate(r));
    child.send({method:'turn/completed',params:{threadId:'thr_123',turn:{id:'turn_456',status:'failed',error:{message:scenario==='context-exhausted'?'context window exceeded':'Synthetic turn failure',codexErrorInfo:scenario==='context-exhausted'?'contextWindowExceeded':null}}}});
  }
  await done;await runner.shutdown();
  const name=`codex-${scenario}`;write(`providers/${name}.json`,{baseline:'c7f46a05641b23944d3692c9d80f9677c7e1ef1d',source:'CodexAppServerRunner; synthetic FakeAppServer errors derived from app-server-runner.test.ts',frames});register(name,'codex',`providers/${name}.json`);
}
async function mcp() {
  const {catalog}=await import(path.join(root,'apps/native/crates/agents/scripts/main-catalog.mjs'));
  const {startMcpStdio}=await import('data:text/javascript;base64,'+Buffer.from(tag('apps/desktop/src/main/agent-tools/mcp-stdio.mjs')).toString('base64'));
  const ts=require(path.join(root,'work/oracle/node_modules/typescript'));
  const external={exports:{}};
  Function('require','module','exports',ts.transpileModule(tag('apps/desktop/src/main/agent-tools/external-session.ts'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText)(name=>{
    if(name==='zod')return require(path.join(root,'work/oracle/node_modules/zod'));
    if(name.startsWith('node:'))return require(name);
    return {};
  },external,external.exports);
  const session=new external.exports.ExternalAgentSession({getOwner:()=>null});
  const requests=[
    {jsonrpc:'2.0',id:1,method:'tools/list'},
    {jsonrpc:'2.0',id:'init',method:'initialize',params:{protocolVersion:'2024-11-05'}},
    {jsonrpc:'2.0',method:'notifications/initialized'},
    {jsonrpc:'2.0',id:2,method:'initialize'},
    {jsonrpc:'2.0',id:3,method:'ping'},
    {jsonrpc:'2.0',id:'catalog',method:'tools/list'},
    {jsonrpc:'2.0',id:4,method:'tools/call',params:{name:'get_project_state',arguments:[]}},
    {jsonrpc:'2.0',id:5,method:'tools/call',params:{}},
    {jsonrpc:'2.0',id:6,method:'unknown'},
    {jsonrpc:'2.0',id:null,method:'ping'},
    {jsonrpc:'1.0',id:7,method:'ping'},
    '{synthetic malformed JSON',
    // No provider, document or account is consulted by these synthetic calls.
    {jsonrpc:'2.0',id:'empty-library',method:'tools/call',params:{name:'list_projects',arguments:{}}},
  ];
  const input=new PassThrough(),output=new PassThrough(),responses=[];
  let buffered='';output.on('data',c=>{buffered+=c;let n;while((n=buffered.indexOf('\n'))>=0){responses.push(JSON.parse(buffered.slice(0,n)));buffered=buffered.slice(n+1);}});
  // The list_projects empty-library body is executed from tagged project tools.
  const src=tag('apps/desktop/src/renderer/src/legacy/assistant/mcp-project-tools.ts');
  const module={exports:{}};
  Function('require','module','exports',ts.transpileModule(src,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText)(name=>{
    if(name==='zod')return require(path.join(root,'work/oracle/node_modules/zod'));
    if(name.endsWith('/guards'))return {isRecord:v=>v&&typeof v==='object'&&!Array.isArray(v)};
    return {};
  },module,module.exports);
  // Exported factory discovery must be explicit: no success-shaped fallback.
  const done=startMcpStdio({input,output,instructions:catalog.instructions,listTools:async()=>catalog.tools,callTool:async(name)=>{
    if(name!=='list_projects')throw Error('Unrecorded synthetic tool');
    const PM={proj:{id:'synthetic-home'},isHomeProject:()=>true,Projects:{list:()=>[]}};
    const call=module.exports.mcpProjectTool;
    if(typeof call!=='function')throw Error('Tagged MCP factory changed');
    return {isError:false,content:[{type:'text',text:JSON.stringify(await call(PM,'__mcp_list_projects',{}))}]};
  }});
  for(const r of requests)input.write((typeof r==='string'?r:JSON.stringify(r))+'\n');
  input.end();await done;
  write('mcp-stdio.json',{baseline:'c7f46a05641b23944d3692c9d80f9677c7e1ef1d',source:'unmodified startMcpStdio; executed catalog; synthetic empty-library tool body',requests,responses});
  register('mcp-stdio','mcp','mcp-stdio.json');
  const errors=[
    {jsonrpc:'2.0',id:'init-errors',method:'initialize'},
    ...['get_project_state','finish_session','synthetic_unknown_tool'].map((name,i)=>({jsonrpc:'2.0',id:i+1,method:'tools/call',params:{name,arguments:{}}})),
    {jsonrpc:'2.0',id:4,method:'tools/call',params:{name:'finish_session',arguments:{commit:false}}}
  ];
  const errorInput=new PassThrough(),errorOutput=new PassThrough(),errorResponses=[];
  let errorBuffered='';errorOutput.on('data',c=>{errorBuffered+=c;let n;while((n=errorBuffered.indexOf('\n'))>=0){errorResponses.push(JSON.parse(errorBuffered.slice(0,n)));errorBuffered=errorBuffered.slice(n+1);}});
  const stopped=startMcpStdio({input:errorInput,output:errorOutput,instructions:catalog.instructions,listTools:async()=>catalog.tools,callTool:(name,args)=>session.call(name,args)});
  for(const r of errors)errorInput.write(JSON.stringify(r)+'\n');
  errorInput.end();await stopped;
  write('mcp-tool-errors.json',{baseline:'c7f46a05641b23944d3692c9d80f9677c7e1ef1d',source:'unmodified startMcpStdio + ExternalAgentSession.call; no active session',requests:errors,responses:errorResponses});
  register('mcp-tool-errors','mcp','mcp-tool-errors.json');
}
(async()=>{
  fs.mkdirSync(path.join(dest,'providers'),{recursive:true});
  oracle.execTranscript();
  for(const s of ['happy','question','steer','cancel','compact','async-question','tools','retry'])await oracle.codex(s);
  await oracle.claude();
  for(const name of ['codex-happy','codex-question','codex-steer','codex-cancel','codex-compact','codex-async-question','codex-tools','codex-retry','codex-exec','claude-control'])register(name,name.startsWith('claude')?'claude':name==='codex-exec'?'exec':'codex',`providers/${name}.json`);
  // Reparse the shipped stream with every public callback enabled, including
  // session identity and the runner's onResult stdin-close signal.
  claudeParser('claude-stream',JSON.parse(fs.readFileSync(path.join(dest,'providers/claude-stream.json'))).frames.filter(f=>f.kind==='in').map(f=>f.value));
  for(const failure of ['initialize-error','thread-error','turn-error','missing-thread','missing-turn','turn-failed','context-exhausted'])await codexFailure(failure);
  claudeControls();
  claudeParser('claude-compaction-retry',[
    {type:'system',subtype:'status',status:'compacting'},
    {type:'system',subtype:'status',status:'compacting'},
    {type:'system',subtype:'compact_boundary'},
    {type:'system',subtype:'api_retry',attempt:2,max_retries:5},
    {type:'result',subtype:'success',result:'{"message":"done"}',structured_output:{message:'done'}}
  ]);
  claudeParser('claude-error',[{type:'result',subtype:'error_during_execution',is_error:true,result:'Synthetic provider refusal'}]);
  await mcp();
  const captureFile='apps/native/crates/agents/scripts/capture-main-replay.cjs';
  write('manifest.json',{version:1,baseline:'c7f46a05641b23944d3692c9d80f9677c7e1ef1d',commit:execFileSync('git',['rev-parse','c7f46a05641b23944d3692c9d80f9677c7e1ef1d^{commit}'],{encoding:'utf8'}).trim(),synthetic:true,sources:sources.map(file=>({file,sha256:hash(tag(file))})),recorder:{file:'apps/native/crates/agents/scripts/capture-main-oracle.cjs',sha256:hash(harness)},capture:{file:captureFile,sha256:hash(fs.readFileSync(path.join(root,captureFile)))},helpers:['apps/native/crates/agents/scripts/main-catalog.mjs'].map(file=>({file,sha256:hash(fs.readFileSync(path.join(root,file)))})),dependencies:{typescript:'5.9.3',zod:'4.6.5'},cases});
  for(const [name,version] of Object.entries({typescript:'5.9.3',zod:'4.6.5'}))if(require(path.join(root,`work/oracle/node_modules/${name}/package.json`)).version!==version)throw Error(`Wrong ${name} version`);
  if(checking)for(const file of ['manifest.json',...cases.map(c=>c.file)])if(!fs.readFileSync(path.join(dest,file)).equals(fs.readFileSync(path.join(committed,file))))throw Error(`Frozen oracle regeneration differs: ${file}`);
  console.log(`Recorded ${cases.length} synthetic replay cases.`);
})().catch(e=>{console.error(e);process.exitCode=1;});
