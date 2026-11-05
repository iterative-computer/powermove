// Regenerate only the owner-selected 3D contract. No app or extension code runs.
import {execFileSync} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
import vm from 'node:vm';
import fs from 'node:fs';
const source='b81b94c66';
const file='apps/desktop/src/main/agent-tools/spec.ts';
const raw=execFileSync('git',['show',`${source}:${file}`],{encoding:'utf8'});
const code=stripTypeScriptTypes(raw.replace(/^import .*;\n/gm,'').replace(/^export /gm,''),{mode:'strip'});
const tools=vm.runInNewContext(`${code}\n;POWERMOVE_AGENT_TOOLS`,{COMMAND_JSON_LIMIT:2000000,EXTENSION_ID:/^[a-z][a-z0-9.-]+$/,ORCHESTRATION_TOOL_NAMES:[],ORCHESTRATION_TOOLS:[]});
const result={source,tools:tools.filter(t=>['get_3d_scene','edit_3d'].includes(t.name))};
const target=new URL('../fixtures/scene3d-source-catalog.json',import.meta.url);
if(process.argv.includes('--write'))fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');
else if(JSON.stringify(JSON.parse(fs.readFileSync(target)))!==JSON.stringify(result))throw Error('3D source catalog drift');
