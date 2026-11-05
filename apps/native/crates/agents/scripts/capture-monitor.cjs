/* Public synthetic evidence only. Execute the unmodified frozen monitor with
 * injected clocks; expected states are never obtained from Rust. */
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const crypto = require('node:crypto');
const {execFileSync} = require('node:child_process');
const root = execFileSync('git', ['rev-parse','--show-toplevel'], {encoding:'utf8'}).trim();
const ts = require(path.join(root,'work/oracle/node_modules/typescript'));
const source = 'apps/desktop/src/main/agent-thread-monitor.ts';
const bytes = execFileSync('git',['show',`v1.2.0:${source}`]);
const hash = v => crypto.createHash('sha256').update(v).digest('hex');
let now = 0, next = 0;
const timers = new Map();
class Clock extends Date { static now() { return now; } }
const moduleObject = {exports:{}};
vm.runInNewContext(ts.transpileModule(bytes.toString(), {compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText, {
  module:moduleObject, exports:moduleObject.exports, Date:Clock,
  setTimeout(fn,ms) { const id=++next; timers.set(id,{fn,at:now+ms}); return {id,unref(){}}; },
  clearTimeout(timer) { if(timer)timers.delete(timer.id); },
});
const monitor = new moduleObject.exports.AgentThreadMonitor();
const leases = new Map();
const operations = [];
const request = (projectId,threadId,id) => ({projectId,threadId,id,provider:'chatgpt',model:null,reasoningEffort:null});
function step(op, at) {
  now = at;
  let result = null;
  try {
    switch(op.kind) {
      case 'reserve': result=monitor.reserve(op.project,op.thread,op.run).state; break;
      case 'begin': result=monitor.begin(op.request).state; break;
      case 'progress': monitor.progress(monitor.get(op.project,op.thread,op.run),op.text); break;
      case 'trace': monitor.trace(monitor.get(op.project,op.thread,op.run),op.trace); break;
      case 'finish': monitor.finish(monitor.get(op.project,op.thread,op.run),op.result); break;
      case 'get': result=monitor.get(op.project,op.thread,op.run)?.state ?? null; break;
      case 'watch': leases.set(op.lease,monitor.watch(op.parent,op.target)); break;
      case 'release': leases.get(op.lease)(); break;
      case 'tick': for(const [id,t] of [...timers]) if(t.at<=now){timers.delete(id);t.fn();} break;
      case 'shutdown': monitor.shutdown(); break;
      default: throw Error(`Unknown operation ${op.kind}`);
    }
    result={ok:true,value:result};
  } catch(error) { result={ok:false,error:error.message}; }
  operations.push({at,op,result:JSON.parse(JSON.stringify(result))});
}
step({kind:'begin',request:request('p','a','root')},100);
step({kind:'begin',request:request('p','b','child')},110);
step({kind:'reserve',project:'p',thread:'b',run:'future'},120);
step({kind:'watch',project:'p',parent:'root',target:'child',lease:'one'},130);
step({kind:'watch',project:'p',parent:'root',target:'child',lease:'two'},140);
step({kind:'release',lease:'one'},150);
step({kind:'release',lease:'one'},151);
step({kind:'watch',project:'p',parent:'child',target:'root',lease:'cycle'},160);
step({kind:'get',project:'q',thread:'b',run:'child'},170);
step({kind:'trace',project:'p',thread:'b',run:'child',trace:{kind:'question',blocking:true}},180);
step({kind:'get',project:'p',thread:'b',run:'child'},190);
step({kind:'progress',project:'p',thread:'b',run:'child',text:'Waiting for watched threads…'},200);
step({kind:'get',project:'p',thread:'b',run:'child'},210);
step({kind:'trace',project:'p',thread:'b',run:'child',trace:{kind:'tool-start',label:'Read  file\ncontents'}},220);
step({kind:'get',project:'p',thread:'b',run:'child'},230);
step({kind:'finish',project:'p',thread:'b',run:'child',result:{ok:true,text:'Authored child result',access:'editor'}},240);
step({kind:'finish',project:'p',thread:'b',run:'child',result:{ok:false,error:'late failure',cancelled:false}},250);
step({kind:'get',project:'p',thread:'b',run:'child'},260);
step({kind:'get',project:'p',thread:'b'},270);
step({kind:'begin',request:request('p','b','child')},280);
step({kind:'release',lease:'two'},290);
step({kind:'watch',project:'p',parent:'child',target:'root',lease:'reverse'},300);
step({kind:'tick'},30119);
step({kind:'get',project:'p',thread:'b',run:'future'},30119);
step({kind:'tick'},30120);
step({kind:'get',project:'p',thread:'b',run:'future'},30120);
step({kind:'shutdown'},30130);
step({kind:'get',project:'p',thread:'a',run:'root'},30130);
const fixture={baseline:'07ed5fa17ee640659cb075f9cad1af2095011d60',source,sourceSha256:hash(bytes),captureSha256:hash(fs.readFileSync(__filename)),operations};
const serialized=JSON.stringify(fixture,null,2)+'\n';
const destination=path.join(root,'apps/native/crates/agents/fixtures/monitor.json');
if(process.argv.includes('--check')) {
  if(fs.readFileSync(destination,'utf8')!==serialized)throw Error('Monitor fixture differs from frozen-source execution');
  console.log('Monitor oracle: identical (28 operations)');
} else fs.writeFileSync(destination,serialized);
