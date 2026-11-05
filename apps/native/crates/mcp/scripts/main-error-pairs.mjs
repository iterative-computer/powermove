// Execute the frozen external-session and bridge gates. Zod is test-only and
// needed only to regenerate the committed parser cases, never by the app/tests.
import {tag, evaluate, catalog} from './main-oracle.mjs';
import fs from 'node:fs';
import vm from 'node:vm';
import {stripTypeScriptTypes} from 'node:module';
import path from 'node:path';
import {pathToFileURL, fileURLToPath} from 'node:url';
const zodRoot=process.env.PM_ORACLE_MODULE_ROOT;
if(!zodRoot) throw Error('Set PM_ORACLE_MODULE_ROOT to a disposable npm prefix containing the tagged Zod 4.6.5');
const {z}=await import(pathToFileURL(path.join(zodRoot,'node_modules/zod/index.js')));
const external=tag('apps/desktop/src/main/agent-tools/external-session.ts');
const bridge=tag('apps/desktop/src/main/agent-tools/bridge.ts');
const method=bridge.slice(bridge.indexOf('  async callTool('),bridge.indexOf('  private async runOutsideSandbox'));
const {callTool}=evaluate(method.replace('  async callTool(', 'async function callTool('),['callTool'],{
  OUTSIDE_SANDBOX_TOOL_NAME:'run_outside_sandbox', POWERMOVE_AGENT_TOOLS:catalog.tools.slice(7),
  POWERMOVE_LIVE_INSPECTION_TOOL_NAMES:catalog.inspection,
  POWERMOVE_APP_AGENT_TOOLS:catalog.tools.filter(t=>catalog.app.includes(t.name)),
});
const sessionCode=stripTypeScriptTypes(external.replace(/^import .*;\n/gm,'').replace(/^export /gm,''),{mode:'transform'});
const {ExternalAgentSession}=vm.runInNewContext(sessionCode+';({ExternalAgentSession})',{z});
const names=catalog.tools.map(t=>t.name);
const special=new Set(['start_session','finish_session','list_projects','create_project','open_project']);
const cases=[];
const add=(state,tool,args={},recipe)=>cases.push({state,tool,args,...(recipe?{recipe}:{})});
for(const tool of names){
  add('closed',tool);
  if(!special.has(tool)) add('none',tool);
  if(!special.has(tool)&&!catalog.inspection.includes(tool)) add('editor',tool);
  if(!special.has(tool)&&!catalog.app.includes(tool)) add('app',tool);
}
for(const state of ['none','project','editor','app']) add(state,'invented_tool',{operation:'read'});
for(const tool of ['start_session','create_project','open_project']) add('project',tool);
for(const state of ['none','project']){
  for(const args of [{},{commit:1},{commit:true,extra:1,another:2}]) add(state,'finish_session',args);
}
add('none','finish_session',{commit:true});
for(const args of [{},{prompt:1,access:'x',context:null,x:1},{prompt:'',access:1},{prompt:'x',extra:1},{prompt:'x',access:null},{prompt:'x',access:'editor',context:'app'}]) add('none','start_session',args);
add('none','start_session',{},'oversized-prompt');
for(const test of cases){
  const instance=new ExternalAgentSession({bridge:{callTool:(...a)=>callTool.call({},...a)}});
  if(test.state==='closed') instance.disconnected=true;
  else if(test.state!=='none') instance.session={
    inspectionOnly:test.state==='editor',context:test.state==='app'?'app':'project',
    isClosed:()=>false,isFinishing:()=>false,owner:{isDestroyed:()=>false}
  };
  const args=test.recipe==='oversized-prompt'?{prompt:'😀'.repeat(200001)}:test.args;
  try{await instance.call(test.tool,args);throw Error('oracle case reached a body instead of a refusal')}
  catch(e){test.error=e.message;if(['TypeError','ReferenceError'].includes(e.name)||e.message==='oracle case reached a body instead of a refusal') throw e;}
}
const output={tag:'main',commit:'c7f46a05641b23944d3692c9d80f9677c7e1ef1d',zod:'4.6.5',cases};
const target=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../fixtures/error-precedence-main-c7f46a056.json');
if(process.argv.includes('--write')) fs.writeFileSync(target,JSON.stringify(output,null,2)+'\n');
else if(JSON.stringify(JSON.parse(fs.readFileSync(target)))!==JSON.stringify(output)) throw Error('Error oracle fixture drift');
process.stdout.write(`${cases.length} tagged gate/parser refusals\n`);
