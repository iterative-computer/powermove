// Resolve inventory attribution against executable tagged definitions, including
// the complete external+agent union and the renderer state projection.
import {tag, catalog} from './main-oracle.mjs';
import {catalog as legacy} from './tag-oracle.mjs';
import {execFileSync} from 'node:child_process';
const harness=tag('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
const project={id:'synthetic-memory-contract',name:'Synthetic',w:320,h:180,fps:30,dur:1,layers:[],assets:{},memory:[{id:'synthetic-goal',kind:'goal',text:'Opaque authored data',author:'agent',createdAt:0,updatedAt:0}]};
const state=JSON.parse(execFileSync(process.execPath,[new URL('source-pairs.mjs',import.meta.url).pathname],{input:JSON.stringify({project}),encoding:'utf8'}));
const spec=catalog.tools.find(t=>t.name==='get_project_state');
process.stdout.write(JSON.stringify({
  tag:'main-c7f46a056',toolCount:catalog.tools.length,legacyToolCount:legacy.tools.length,legacyProjectMemoryTool:legacy.tools.some(t=>t.name==='project_memory'),
  projectMemoryTool:catalog.tools.some(t=>t.name==='project_memory'),
  projectStateMentionsMemory:/\bmemory\b|project_memory/.test(spec.description),
  projectStateReturnsMemory:Object.hasOwn(state,'memory')||Object.hasOwn(state,'memoryIndex'),
  rendererMentionsProjectMemory:harness.includes('project_memory'),
}));
