import {tag,evaluate} from './main-oracle.mjs';
import fs from 'node:fs';
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const source=tag('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
const constants=source.slice(source.indexOf('const SCENE_OPERATIONS'),source.indexOf('function safeTimes'));
const body=source.slice(source.indexOf('function cleanCommand('),source.indexOf('function sanitizeProposal('));
const limit=Number(tag('apps/desktop/src/shared/edit-limits.ts').match(/= ([\d_]+)/)[1].replaceAll('_',''));
const {cleanCommand}=evaluate(constants+body,['cleanCommand'],{COMMAND_JSON_LIMIT:limit});
if(input.tool==='select_layers') {
  const start=source.indexOf("  if (request.tool === 'select_layers') {");
  const end=source.indexOf("  if (request.tool === 'validate_effect') {",start);
  const PM={sel:input.selection??{layers:[],keys:[],chan:null},L:id=>(input.layers??[]).find(l=>l.id===id),selectLayers:(ids,add)=>{PM.sel={layers:add?[...new Set([...PM.sel.layers,...ids])]:ids,keys:[],chan:null}}};
  const {select}=evaluate('function select(request){'+source.slice(start,end)+'}', ['select'], {PM,liveToolTransactions:new Map(),liveToolObservations:new Map(),toolText:value=>value,clone:structuredClone,currentRevision:()=>0});
  try{const result=select({tool:'select_layers',arguments:input.arguments});process.stdout.write(JSON.stringify({result:result.content[0]}));}
  catch(e){process.stdout.write(JSON.stringify({error:e.message}));}
  process.exit(0);
}
const raw=Array.isArray(input.commands)?input.commands:[];
const commands=raw.map(cleanCommand);
const invalid=commands.indexOf(null);
process.stdout.write(JSON.stringify(invalid!==-1?{error:`Invalid or oversized command at index ${invalid}. No commands were applied. Each command must be valid JSON with a supported type and at most ${limit} characters.`}:!commands.length?{error:'No valid Powermove edit commands were supplied.'}:{commands}));
