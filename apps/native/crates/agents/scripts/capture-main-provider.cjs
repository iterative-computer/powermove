/* Execute pinned-main compatible setup methods against synthetic HTTP and a
 * synthetic encrypted-key service. No provider or credential is contacted. */
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),crypto=require('node:crypto');
const {execFileSync}=require('node:child_process');
const root=execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
const base=path.join(root,'apps/native/crates/agents'),reference='c7f46a05641b23944d3692c9d80f9677c7e1ef1d';
const ts=require(path.join(root,'work/oracle/node_modules/typescript'));
if(ts.version!=='5.9.3')throw Error('Reference capture requires TypeScript 5.9.3');
const hash=v=>crypto.createHash('sha256').update(v).digest('hex'),sources=new Map(),cache=new Map();
function load(file){
 if(cache.has(file))return cache.get(file);
 const bytes=execFileSync('git',['show',`${reference}:apps/desktop/src/${file}.ts`]);sources.set('apps/desktop/src/'+file+'.ts',hash(bytes));
 const module={exports:{}};cache.set(file,module.exports);
 const req=name=>{
  if(name.startsWith('node:'))return require(name);
  if(name==='electron')return {safeStorage:{isEncryptionAvailable:()=>true,encryptString:v=>Buffer.from('sealed:'+v),decryptString:v=>v.toString().replace(/^sealed:/,'')}};
  if(name==='../shared/compatible-provider')return load('shared/compatible-provider');
  if(['../shared/ipc','./agent-tools/spec','../shared/effect-authoring','../shared/response-style','./compatible-workspace','./codex/instructions','./codex/workspace','./codex/consent','./agent-tools/trace-format','../shared/media-tools','../shared/agent-models'].includes(name))return {};
  throw Error(`Uninjected dependency ${name}`);
 };
 vm.runInNewContext(ts.transpileModule(bytes.toString(),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,esModuleInterop:true}}).outputText,{module,exports:module.exports,require:req,URL,Buffer,TextDecoder,TextEncoder,Response,AbortSignal:{timeout:timeout=>({timeout})},console});
 cache.set(file,module.exports);return module.exports;
}
const cloud='https://provider.example/v1',local='http://localhost:11434/v1';
const cases=[
 {name:'cloud-models',operation:'list',baseUrl:cloud,payload:{data:[{id:'tool-model',supported_parameters:['tools'],architecture:{input_modalities:['text','image']}},{id:'no-tools',supported_parameters:[]},{id:'unknown'},{id:'tool-model'},{id:'bad\nname'}]}},
 {name:'local-ollama',operation:'list',baseUrl:local,payload:{data:[{id:'zeta'},{id:'alpha'},{id:'no-tools'}]},ollama:true,caps:{zeta:['tools','vision'],alpha:['tools'],'no-tools':['completion']}},
 {name:'missing-model-endpoint',operation:'list',baseUrl:cloud,status:404},
 {name:'saved-address-key',operation:'list',baseUrl:cloud,payload:{data:[{id:'saved-model'}]},saved:{baseUrl:cloud,model:'old',vision:false,hasKey:true,secret:Buffer.from('x').toString('base64')}},
 {name:'other-address-does-not-get-key',operation:'list',baseUrl:'https://other.example/v1',payload:{data:[{id:'other'}]},saved:{baseUrl:cloud,model:'old',vision:false,hasKey:true,secret:Buffer.from('x').toString('base64')}},
 {name:'configure-cloud',operation:'configure',baseUrl:cloud,model:'authored-alias',vision:true,apiKey:'synthetic-key',payload:{data:[{id:'another-model'},{id:'authored-alias'}]}},
 {name:'configure-ollama-context',operation:'configure',baseUrl:local,model:'fixture',vision:false,payload:{data:[{id:'fixture'}]},ollama:true,context:4096},
 {name:'configure-failure-keeps-saved',operation:'configure',baseUrl:cloud,model:'fixture',vision:false,status:401,error:{error:{message:'do not expose rejected secret'}},saved:{baseUrl:cloud,model:'saved-alias',vision:true,hasKey:false}},
 {name:'unsupported-tools',operation:'list',baseUrl:cloud,status:400,error:{error:{message:'tool calling is not supported'}}},
 {name:'rate-limited',operation:'list',baseUrl:cloud,status:429,error:{error:{message:'synthetic quota'}}},
];
(async()=>{
 const {CompatibleProvider}=load('main/compatible-provider');const outputs=[];
 for(const input of cases){
  const directory=fs.mkdtempSync(path.join(root,'work/provider-oracle-'));if(input.saved)fs.writeFileSync(path.join(directory,'agent-provider.json'),JSON.stringify(input.saved));
  const requests=[];
  const fetch=async(url,init={})=>{
   requests.push({method:init.method??'GET',url:String(url),body:init.body?JSON.parse(init.body):null,key:init.headers?.Authorization?.replace(/^Bearer /,'')??null,timeoutMs:init.signal?.timeout});
   if(url.endsWith('/api/version'))return new Response(JSON.stringify(input.ollama?{version:'synthetic'}:{}),{status:input.ollama?200:404});
   if(url.endsWith('/api/show'))return new Response(JSON.stringify({capabilities:input.caps?.[JSON.parse(init.body).model]??[]}));
   if(url.endsWith('/api/ps'))return new Response(JSON.stringify({models:[{name:input.model+':latest',context_length:input.context}]}));
   const status=input.status??200;
   const body=input.error??(url.endsWith('/chat/completions')?{choices:[{message:{content:'OK'}}]}:input.payload??{data:[]});
   return new Response(JSON.stringify(body),{status});
  };
  const provider=new CompatibleProvider(directory,fetch);let result;
  try{result={output:input.operation==='list'?await provider.listModels({baseUrl:input.baseUrl}):await provider.configure({baseUrl:input.baseUrl,model:input.model,vision:input.vision,...(input.apiKey?{apiKey:input.apiKey}:{})})};}catch(error){result={error:error.message};}
  outputs.push({input,...result,requests});
 }

 const streams=[];
 const {readCompletion}=load('main/compatible-provider');
 const event=value=>'data: '+JSON.stringify(value)+'\n';
 const inputs=[
  event({choices:[{delta:{content:'café 🎞'}}]})+'data: [DONE]\n',
  event({choices:[{delta:{tool_calls:[{index:1,id:'one',function:{name:'read_',arguments:'{'}}]}}]})+event({choices:[{delta:{tool_calls:[{index:0,id:'zero',function:{name:'inspect',arguments:'{}'}},{index:1,function:{name:'file',arguments:'}'}}]},finish_reason:'tool_calls'}]}),
  'data: {"choices":[{"delta":{"content":"hel\ndata: lo"}}]}\ndata: [DONE]\n',
  event({choices:[{delta:{content:'unfinished'}}]}),
  'data: not-json\n',
  event({choices:[{delta:{},finish_reason:'length'}]}),
  event({choices:[{delta:{tool_calls:[{index:80}]}}]})+'data: [DONE]\n',
 ];
 for(const text of inputs){
  const bytes=Buffer.from(text);const chunks=[];for(let i=0;i<bytes.length;i+=3)chunks.push([...bytes.subarray(i,i+3)]);
  const traces=[];let result;
  const body=new ReadableStream({start(controller){chunks.forEach(chunk=>controller.enqueue(Uint8Array.from(chunk)));controller.close();}});
  try{result={output:await readCompletion({body},{throwIfAborted(){}},text=>traces.push(text))};}catch(error){result={error:error.message};}
  streams.push({chunks,...result,traces});
 }
 const value={reference,synthetic:true,sources:[...sources].map(([file,sha256])=>({file,sha256})),captureSha256:hash(fs.readFileSync(__filename)),cases:outputs,streams};
 const destination=path.join(base,'fixtures/main/provider.json');fs.mkdirSync(path.dirname(destination),{recursive:true});const bytes=JSON.stringify(value,null,2)+'\n';
 if(process.argv.includes('--check')){if(fs.readFileSync(destination,'utf8')!==bytes)throw Error('Main provider oracle differs');console.log('Main provider setup: identical');}else fs.writeFileSync(destination,bytes);
})().catch(error=>{console.error(error);process.exitCode=1;});
