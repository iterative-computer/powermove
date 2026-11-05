// Resolve inventory attribution against executable tagged definitions, including
// the complete external+agent union and the renderer state projection.
import {tag, catalog} from './tag-oracle.mjs';
import {execFileSync} from 'node:child_process';
const harness=tag('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
const project={id:'synthetic-memory-contract',name:'Synthetic',w:320,h:180,fps:30,dur:1,layers:[],assets:{},memory:{goal:'Opaque authored data',rules:['Preserve'],decisions:[]}};
const state=JSON.parse(execFileSync(process.execPath,[new URL('source-pairs.mjs',import.meta.url).pathname],{input:JSON.stringify({project}),encoding:'utf8'}));
const spec=catalog.tools.find(t=>t.name==='get_project_state');
process.stdout.write(JSON.stringify({
  tag:'v1.2.0',toolCount:catalog.tools.length,
  projectMemoryTool:catalog.tools.some(t=>t.name==='project_memory'),
  projectStateMentionsMemory:/\bmemory\b|project_memory/.test(spec.description),
  projectStateReturnsMemory:Object.hasOwn(state,'memory')||Object.hasOwn(state,'memoryIndex'),
  rendererMentionsProjectMemory:harness.includes('project_memory'),
}));
