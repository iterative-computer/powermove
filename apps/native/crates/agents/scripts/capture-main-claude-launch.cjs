/* Source-derived current-main launch evidence, no provider or credential. */
const fs = require('node:fs'), path = require('node:path'), vm = require('node:vm'), crypto = require('node:crypto');
const {execFileSync} = require('node:child_process');
const root = execFileSync('git', ['rev-parse', '--show-toplevel'], {encoding:'utf8'}).trim();
const ts = require(path.join(root, 'work/oracle/node_modules/typescript'));
const reference = 'c7f46a05641b23944d3692c9d80f9677c7e1ef1d';
const sources = [];
function load(file, bindings) {
  const source = execFileSync('git', ['show', `${reference}:${file}`]);
  sources.push({file, sha256: crypto.createHash('sha256').update(source).digest('hex')});
  const module = {exports:{}};
  const requireStub = name => {
    if (!(name in bindings)) throw Error(`Unexpected import ${name}`);
    return bindings[name];
  };
  vm.runInNewContext(ts.transpileModule(source.toString(), {compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,
    {module,exports:module.exports,require:requireStub});
  return module.exports;
}
const models = load('apps/desktop/src/shared/agent-models.ts', {});
const tools = ['mcp__powermove__get_project_state', 'mcp__powermove__project_memory'];
const inspection = ['mcp__powermove__get_project_state'];
const adapter = load('apps/desktop/src/main/claude/adapter.ts', {
  '../../shared/agent-testing': {AGENT_TESTING_INSTRUCTIONS:'Synthetic testing instructions'},
  '../../shared/agent-models': models,
  '../agent-tools/spec': {POWERMOVE_LIVE_INSPECTION_MCP_TOOL_NAMES:inspection,POWERMOVE_MCP_TOOL_NAMES:tools},
  '../codex/instructions': {AGENT_SHELL_NETWORK_INSTRUCTIONS:'Shell commands have full internet access.'}
});
const cases = [];
for (const access of ['editor','project','computer']) for (const approval of ['auto','supervised']) for (const ask of [false,true]) {
  const options = {schema:{type:'object',properties:{text:{type:'string'}}},prompt:'Preserve "quotes"\nAnd Unicode 🎬', imagePaths:['/synthetic/reference image.png'],
    model:'opus', reasoningEffort:'high', sessionId:'synthetic-session', access, approval,
    extensionsDir:'/synthetic/Extensions', instructions:'Synthetic original instructions',additionalInstructions:'Original extra instructions',
    nativeTools:{command:'/synthetic/Powermove',args:['--powermove-mcp']},
    externalMcpServers:{external:{command:'/synthetic/external'},powermove:{command:'/must-not-be-used'}},askOutsideSandbox:ask};
  cases.push({access,approval,options:{...options,effort:'high',toolNames:tools,inspectionToolNames:inspection,shellNetworkInstructions:'Shell commands have full internet access.'},
    argv:adapter.buildClaudeArgv(options),message:JSON.parse(adapter.claudeUserMessage(options.prompt,options.imagePaths))});
}
// The standalone live probe has no native tools, selected model or resume id.
for (const access of ['editor','project','computer']) {
  const options = {schema:{type:'object'},prompt:'Synthetic standalone check',imagePaths:[],model:null,
    reasoningEffort:null,sessionId:null,access,approval:'auto',instructions:'Synthetic original instructions',
    additionalInstructions:'',nativeTools:null,externalMcpServers:{powermove:{command:'/must-not-be-used'}},askOutsideSandbox:false};
  cases.push({access,approval:'auto',options:{...options,effort:null,toolNames:tools,inspectionToolNames:inspection,
    shellNetworkInstructions:'Shell commands have full internet access.'},argv:adapter.buildClaudeArgv(options),
    message:JSON.parse(adapter.claudeUserMessage(options.prompt,options.imagePaths))});
}
const recorderFile = 'apps/native/crates/agents/scripts/capture-main-claude-launch.cjs';
const recorder = {file:recorderFile,sha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(root,recorderFile))).digest('hex')};
const result = {reference,sources,recorder,cases};
const dest = path.join(root,'apps/native/crates/agents/fixtures/main-claude-launch.json');
const bytes = JSON.stringify(result,null,2)+'\n';
if (process.argv.includes('--check')) {
  if (fs.readFileSync(dest,'utf8') !== bytes) throw Error('Current-main Claude launch fixture drift');
} else fs.writeFileSync(dest,bytes);
process.stdout.write(`${cases.length} current-main native Claude launch cases\n`);
