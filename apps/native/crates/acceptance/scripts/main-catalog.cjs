// Development-only pinned source execution. No Electron/Node dependency in app.
const ts = require('typescript');
const cp = require('node:child_process');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
if (ts.version !== '5.9.3') throw Error('Expected TypeScript 5.9.3');
const root = cp.execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
const reference = 'c7f46a05641b23944d3692c9d80f9677c7e1ef1d';
const sources = {};
function source(file) {
  const text = cp.execFileSync('git',['show',`${reference}:${file}`],{cwd:root,encoding:'utf8',maxBuffer:16*1024*1024});
  sources[file] = {sha256: crypto.createHash('sha256').update(text).digest('hex')};
  return text;
}
function evaluate(text,names,bindings={}) {
  const ast = ts.createSourceFile('reference.ts',text,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
  if (ast.parseDiagnostics.length) throw Error('Pinned reference parse error');
  const imports = ast.statements.filter(ts.isImportDeclaration).map(n=>[n.getFullStart(),n.end]);
  for (const [start,end] of imports.reverse()) text=text.slice(0,start)+text.slice(end);
  const js=ts.transpileModule(text.replace(/^export /gm,''),{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText;
  return vm.runInNewContext(js+`;({${names.join(',')}})`,bindings,{timeout:2000});
}
const base='apps/desktop/src/main/agent-tools/';
const shared=evaluate(source('apps/desktop/src/shared/agent-orchestration.ts').split('export const AGENT_ORCHESTRATION_INSTRUCTIONS')[0],['ORCHESTRATION_TOOL_NAMES']);
const orchestration=evaluate(source(base+'orchestration-spec.ts'),['ORCHESTRATION_TOOLS'],shared);
const EXTENSION_ID=vm.runInNewContext(source('packages/registry/src/manifest.ts').match(/export const EXTENSION_ID = (.*);/)[1]);
const COMMAND_JSON_LIMIT=Number(source('apps/desktop/src/shared/edit-limits.ts').match(/= ([\d_]+)/)[1].replaceAll('_',''));
const agent=evaluate(source(base+'spec.ts'),['POWERMOVE_AGENT_TOOLS','POWERMOVE_LIVE_INSPECTION_TOOL_NAMES','POWERMOVE_APP_AGENT_TOOLS'],{...shared,...orchestration,EXTENSION_ID,COMMAND_JSON_LIMIT});
const external=evaluate(source(base+'external-spec.ts'),['EXTERNAL_MCP_TOOLS','EXTERNAL_MCP_INSTRUCTIONS']);
const harness=source('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
const commandFields=evaluate(harness.slice(harness.indexOf('const FIELDS:'),harness.indexOf('const clone =')),['FIELDS']).FIELDS;
const easing=source('apps/desktop/src/renderer/src/legacy/core/easing.ts');
const easePresets=evaluate(easing.slice(easing.indexOf('const PRESETS:'),easing.indexOf('const Ease:')),['PRESETS']).PRESETS;
const propertyShortcuts=evaluate(source('apps/desktop/src/renderer/src/kernel/property-shortcuts.ts'),['propertyShortcuts']).propertyShortcuts;
const keymap=evaluate(source('apps/desktop/src/extensions/keymap-default/keymap.ts'),['KEYMAP_DEFAULT'],{propertyShortcuts}).KEYMAP_DEFAULT;
const viewer=source('apps/desktop/src/extensions/viewer/index.ts');
const viewerBindings=[...viewer.matchAll(/api\.keybindings\.bind\((\{[^;]+?\})\);/g)].map(match=>evaluate('const binding='+match[1],['binding']).binding);
const timeline=source('apps/desktop/src/extensions/timeline/index.ts');
const timelineBindings=[...timeline.matchAll(/api\.keybindings\??\.bind\((\{[^;]+?\})\);/g)].map(match=>evaluate('const binding='+match[1],['binding']).binding);
const {normalizeChord}=evaluate(source('apps/desktop/src/renderer/src/kernel/keychord.ts'),['normalizeChord']);
const bindings=[...keymap,...viewerBindings,...timelineBindings].map((binding,index)=>({id:`MAIN.KEY.${String(index+1).padStart(3,'0')}`,chord:normalizeChord(binding.key),command:binding.command,args:binding.args||[],priority:binding.priority||0,in_fields:binding.inFields===true,repeat:binding.repeat===true,loose:binding.looseModifiers===true,workflow:'W010',source: index<keymap.length?'apps/desktop/src/extensions/keymap-default/keymap.ts':index<keymap.length+viewerBindings.length?'apps/desktop/src/extensions/viewer/index.ts':'apps/desktop/src/extensions/timeline/index.ts'}));
const catalog={easePresets,commandFields,tools:[...external.EXTERNAL_MCP_TOOLS,...agent.POWERMOVE_AGENT_TOOLS],inspection:agent.POWERMOVE_LIVE_INSPECTION_TOOL_NAMES,app:agent.POWERMOVE_APP_AGENT_TOOLS.map(t=>t.name),instructions:external.EXTERNAL_MCP_INSTRUCTIONS};
const result={reference,source_hashes:sources,catalog,bindings};
const destination=path.join(__dirname,'../data/catalog-main.json');
const output=JSON.stringify(result,null,2)+'\n';
if (process.argv.includes('--write')) fs.writeFileSync(destination,output);
else if (fs.readFileSync(destination,'utf8')!==output) throw Error('Pinned main catalog drift');
console.log(JSON.stringify({reference,tools:catalog.tools.length,sources:Object.keys(sources).length}));
