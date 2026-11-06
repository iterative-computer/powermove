/* Primary reference: owner-pinned main. Expectations come only from executing
 * its unmodified public TypeScript. All inputs/profiles/keys are synthetic. */
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),crypto=require('node:crypto');
const {execFileSync}=require('node:child_process');
const root=execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
const base=path.join(root,'apps/native/crates/agents');
const REFERENCE='c7f46a05641b23944d3692c9d80f9677c7e1ef1d';
const ts=require(path.join(root,'work/oracle/node_modules/typescript'));
if(ts.version!=='5.9.3')throw Error('Reference capture requires TypeScript 5.9.3');
const sources=new Map(),cache=new Map();const hash=v=>crypto.createHash('sha256').update(v).digest('hex');
function load(file){
 if(cache.has(file))return cache.get(file);
 const bytes=execFileSync('git',['show',`${REFERENCE}:apps/desktop/src/${file}.ts`]);sources.set('apps/desktop/src/'+file+'.ts',hash(bytes));
 const module={exports:{}};cache.set(file,module.exports);
 const req=name=>{
  if(name.startsWith('node:'))return require(name);
  if(name==='electron')throw Error('A pure contract unexpectedly needs Electron');
  if(file==='main/claude/adapter'&&name==='../agent-tools/spec')return {POWERMOVE_LIVE_INSPECTION_MCP_TOOL_NAMES:[],POWERMOVE_MCP_TOOL_NAMES:[]};
  if(file==='main/claude/adapter'&&name==='../codex/instructions')return {AGENT_SHELL_NETWORK_INSTRUCTIONS:'synthetic unobserved instruction dependency'};

  if(name.startsWith('.'))return load(path.posix.normalize(path.posix.join(path.posix.dirname(file),name)));
  throw Error(`Uninjected dependency ${name}`);
 };
 vm.runInNewContext(ts.transpileModule(bytes.toString()+(file==='main/agent-clients'?'\n;module.exports.__catalog = CLIENTS;':''),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,esModuleInterop:true}}).outputText,{module,exports:module.exports,require:req,URL,TextEncoder,TextDecoder,Buffer,console});
 cache.set(file,module.exports);return module.exports;
}
const out={reference:REFERENCE,synthetic:true,sources:[],captureSha256:hash(fs.readFileSync(__filename)),memory:[],urls:[],groups:[],markdown:[],media:[],visual:[],registration:[],claude:[]};
const clone=v=>JSON.parse(JSON.stringify(v));
function capture(list,input,fn){try{list.push({input:clone(input),output:clone(fn())});}catch(error){list.push({input:clone(input),error:error.message});}}

const adapter=load('main/claude/adapter');
for(const access of ['editor','project','computer'])for(const approval of ['auto','supervised']){
 const input={access,approval};
 const argv=adapter.buildClaudeArgv({access,approval,askOutsideSandbox:access==='project',schema:{},prompt:'synthetic',imagePaths:[],model:null,reasoningEffort:null,sessionId:null});
 const flag=name=>{const at=argv.indexOf(name);return at<0?null:argv[at+1];};
 out.claude.push({input,output:{mode:flag('--permission-mode'),settings:flag('--settings')?JSON.parse(flag('--settings')):null,allowed:flag('--allowedTools'),disallowed:flag('--disallowedTools'),skip:argv.includes('--dangerously-skip-permissions')}});
}
const memory=load('renderer/src/core/memory');
const operations=[
 {operation:'add',kind:'goal',text:' Original goal ',why:' Because intent ',resolved:true},
 {operation:'add',kind:'goal',text:'Revised goal',resolved:false},
 {operation:'add',kind:'constraint',text:'Keep original footage'},
 {operation:'add',kind:'decision',text:'Use authored timing',why:'Review approval'},
 {operation:'add',kind:'open',text:'Check audio'},
 {operation:'update',id:'mem-5',resolved:true,why:''},
 {operation:'update',id:'mem-3',kind:'goal'},
 {operation:'remove',id:'missing'},
 {operation:'add',kind:'bad',text:'Refuse'},
 {operation:'add',kind:'feedback',text:'   '},
 {operation:'remove',id:'mem-4'},
];
let entries=[];
operations.forEach((operation,i)=>{
 const input={kind:'apply',entries:clone(entries),operation,now:i+1,uid:`mem-${i+1}`,author:'agent'};
 capture(out.memory,input,()=>{const result=memory.applyMemoryOperation(entries,operation,{now:i+1,uid:()=>`mem-${i+1}`,author:'agent'});entries=result.entries;return result;});
});
for(const [kind,input] of [['sanitize',null],['sanitize',[null,{id:'a',kind:'goal',text:' Keep ',extra:'retained by native archive'},{id:'a',kind:'goal',text:'duplicate'},{id:'bad',kind:'unknown',text:'opaque'},{id:'empty',kind:'decision',text:' '}]],['index',entries],['list',entries]]){
 capture(out.memory,{kind,entries:input,now:100,filter:{kind:'constraint',limit:2}},()=>kind==='sanitize'?memory.sanitizeMemory(input,100):kind==='index'?memory.memoryIndex(input):memory.listMemory(input,{kind:'constraint',limit:2}));
}
const capped=Array.from({length:100},(_,i)=>({id:`c${i}`,kind:i===0?'goal':'decision',text:`entry${i}`,author:'agent',createdAt:i,updatedAt:i,...(i===10?{resolved:true}:{})}));
capture(out.memory,{kind:'apply',entries:capped,operation:{operation:'add',kind:'feedback',text:'new feedback'},now:101,uid:'new',author:'agent'},()=>memory.applyMemoryOperation(capped,{operation:'add',kind:'feedback',text:'new feedback'},{now:101,uid:()=> 'new',author:'agent'}));
const provider=load('shared/compatible-provider');out.presets=provider.COMPATIBLE_PRESETS;
for(const value of ['localhost:11434','http://127.0.0.1:1234/v1/','[::1]:8080/chat/completions','https://provider.example/deployment/chat/completions/','provider.example','http://provider.example/v1','https://user:pass@provider.example','https://provider.example/v1?key=synthetic','https://provider.example/#fragment','https://PROVIDER.example:443/a//'])capture(out.urls,value,()=>provider.providerUrl(value));
const groups=load('renderer/src/panels/agent/work-groups');
const conversations=[[],[{role:'notice',text:'retained'}],[{role:'user',text:'task'},{role:'trace',durationMs:2300,steps:[{kind:'thinking',text:'work'},{kind:'text',text:'reply'}]}],[{role:'user',text:'task'},{role:'trace',durationMs:1200,steps:[{kind:'tool',label:'Read'}]},{role:'user',steering:true,text:'direction'},{role:'trace',durationMs:2600,steps:[{kind:'tool',label:'Write'},{kind:'text',text:'answer'}]},{role:'assistant',text:'notes'}],[{role:'user',text:'first'},{role:'trace',steps:[{kind:'tool',label:'unfinished'}]},{role:'user',text:'next'},{role:'assistant',text:'done'}]];
for(const conversation of conversations)for(const liveFrom of [null,0,2])capture(out.groups,{conversation,liveFrom},()=>groups.groupConversation(conversation,liveFrom));
const markdown=load('renderer/src/panels/agent/markdown');
const examples=['# Authored **heading**\n\nParagraph with *emphasis* and `code`.','- [x] done\n- [ ] pending\n  - child\n\n1. first\n2. second','> ## Quote\n> body\n>\n> - nested item','Title\n---\n\n---','| A | B |\n| :--- | ---: |\n| **one** | two |','```rust\nlet x = 1;\n```','Before\n```js\nstill streaming','two  \nlines\\\nhard','[safe](https://example.test/a) [unsafe](javascript:alert) <script>bad</script>','![local](/tmp/frame.png) `/tmp/clip.mov` /tmp/sound.wav','![remote](https://example.test/image.png)','<b>bold</b> <em>em</em><br>line <img src="/tmp/frame.png" alt="frame">','snake_case ***both*** ~~gone~~ <https://example.test>','- first\n\n  paragraph\n  ```txt\n  fenced\n  ```\n- second'];
for(const text of examples)capture(out.markdown,{text},()=>({blocks:markdown.blocksFromMarkdown(text),plain:markdown.plainText(markdown.blocksFromMarkdown(text))}));
for(const text of ['**streaming','**streaming**','```\npartial','```\npartial\n```'])capture(out.markdown,{text},()=>({blocks:markdown.blocksFromMarkdown(text),plain:markdown.plainText(markdown.blocksFromMarkdown(text))}));

// Execute the actual main Markdown tests with real assertions; record every
// authored public parser input. No test or assertion is skipped.
const assert=require('node:assert/strict');out.inline=[];out.words=[];out.markdownSourceTests=0;
const wrapped={...markdown,
 blocksFromMarkdown(text){const blocks=markdown.blocksFromMarkdown(text);out.markdown.push({input:{text:text??''},output:clone({blocks,plain:markdown.plainText(blocks)})});return blocks;},
 inlineRuns(text,flags){const value=markdown.inlineRuns(text,flags);out.inline.push({text,flags:flags??{},output:clone(value)});return value;},
 wordsFromRuns(runs){const value=markdown.wordsFromRuns(runs);out.words.push({runs:clone(runs),output:clone(value)});return value;},
};
const testFile='apps/desktop/src/renderer/src/panels/agent/markdown.test.ts';
const testBytes=execFileSync('git',['show',`${REFERENCE}:${testFile}`]);sources.set(testFile,hash(testBytes));
function subset(actual,expected){if(Array.isArray(expected))assert.equal(actual.length,expected.length);if(expected&&typeof expected==='object'){for(const [k,v]of Object.entries(expected))subset(actual[k],v);}else assert.equal(actual,expected);}
const expect=actual=>({toEqual:expected=>assert.deepEqual(clone(actual),clone(expected)),toBe:expected=>assert.equal(actual,expected),toHaveLength:length=>assert.equal(actual.length,length),toMatchObject:expected=>subset(actual,expected)});
const testModule={exports:{}};
vm.runInNewContext(ts.transpileModule(testBytes.toString(),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,{module:testModule,exports:testModule.exports,require:name=>{if(name==='vitest')return {describe:(_name,fn)=>fn(),it:(_name,fn)=>{out.markdownSourceTests++;fn();},expect};if(name==='./markdown')return wrapped;throw Error(`Unknown test dependency ${name}`);},console});
const media=load('shared/agent-media');
for(const value of ['/tmp/frame.PNG','file:///tmp/caf%C3%A9.png','~/clips/test.mov','/tmp/../escape.png','https://example.test/a.png','/tmp/audio.flac?literal','relative.png'])capture(out.media,value,()=>({extension:media.agentMediaExtension(value),kind:media.agentMediaKind(value),local:media.localMediaPath(value),url:media.agentFileUrl(value)}));
const visual=load('shared/visual-reply');
for(const value of [{title:' Authored ',html:'<script>document.body.textContent="hello"</script>'},{title:'',html:'x'},{title:'Reply',html:'x',height:159},{title:'Reply',html:'x',height:900},{title:'Reply',html:'x',unknown:true}])capture(out.visual,{kind:'input',value},()=>visual.visualReplyInput(value));
for(const data of [{channel:'powermove-visual',token:'fixture',type:'ready'},{channel:'powermove-visual',token:'fixture',type:'size',height:160.2},{channel:'powermove-visual',token:'fixture',type:'size',height:99999},{channel:'powermove-visual',token:'fixture',type:'error',message:'frame error'},{channel:'powermove-visual',token:'wrong',type:'ready'},{channel:'powermove-visual',token:'fixture',type:'ready',extra:true}])capture(out.visual,{kind:'event',data},()=>{const frame={};return visual.visualReplyEvent({origin:'null',source:frame,data},frame,'fixture');});

(async()=>{
const clients=load('main/agent-clients');out.clients=clients.__catalog;
const paths={'claude-code':'.claude.json',codex:'.codex/config.toml','claude-desktop':'Library/Application Support/Claude/claude_desktop_config.json','gemini-cli':'.gemini/settings.json',cursor:'.cursor/mcp.json',vscode:'Library/Application Support/Code/User/mcp.json',windsurf:'.codeium/windsurf/mcp_config.json','lm-studio':'.lmstudio/mcp.json',opencode:'.config/opencode/opencode.json'};
for(const [id,relative] of Object.entries(paths))for(const action of ['connect','disconnect']){
 const home=fs.mkdtempSync(path.join(root,'work/client-oracle-'));const file=path.join(home,relative);fs.mkdirSync(path.dirname(file),{recursive:true});const calls=[];
 const env={homeDir:home,userDataDir:path.join(home,'userData'),executablePath:'/Applications/Powermove.app/Contents/MacOS/Powermove',runClaude:async args=>{calls.push(args);return {code:0,output:''};}};
 let before=id==='codex'?'model = "fixture"\n\n[mcp_servers.other]\ncommand = "other"\n':JSON.stringify({opaque:{keep:true}});
 fs.writeFileSync(file,before);
 if(action==='disconnect'){
  await clients.connectAgentClient(id,env);
  if(id==='claude-code')fs.writeFileSync(file,JSON.stringify({mcpServers:{powermove:{type:'stdio',command:env.executablePath,args:['--powermove-mcp'],env:{}}}}));
  before=fs.readFileSync(file,'utf8');calls.length=0;
 }
 if(action==='connect')await clients.connectAgentClient(id,env);else await clients.disconnectAgentClient(id,env);
 out.registration.push({id,relative,action,executable:env.executablePath,before,after:fs.readFileSync(file,'utf8'),calls});
}
const destination=path.join(base,'fixtures/main/contracts.json');
fs.mkdirSync(path.dirname(destination),{recursive:true});
out.sources=[...sources].map(([file,sha256])=>({file,sha256}));
// Project memory, client registration, work groups and client metadata have native oracles elsewhere.
for(const key of ['memory','registration','groups','clients'])delete out[key];
const serialized=JSON.stringify(out,null,2)+'\n';
if(process.argv.includes('--check')){if(fs.readFileSync(destination,'utf8')!==serialized)throw Error('Main contract oracle differs');console.log('Main contracts: identical');}else fs.writeFileSync(destination,serialized);
})().catch(error=>{console.error(error);process.exitCode=1;});
