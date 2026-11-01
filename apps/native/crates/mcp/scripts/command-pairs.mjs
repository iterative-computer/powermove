// Result-shape oracle: execute tagged bodies on synthetic data only.
import { tag,evaluate } from './tag-oracle.mjs';
import fs from 'node:fs';
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const PM={proj:input.project,time:0,touch:()=>{},bus:{emit:()=>{}},TYPE_META:{solid:{},group:{}},BLENDS:['normal'],clamp:(n,a,b)=>Math.min(b,Math.max(a,n))};
const source=tag('apps/desktop/src/renderer/src/legacy/core/editing.ts');
// Keep the allowlist tied to the frozen implementation, not a copied fixture.
const {LAYER_FIELDS}=evaluate(source.slice(source.indexOf('const LAYER_FIELDS:'),source.indexOf('let live:')),['LAYER_FIELDS']);
const functions=['setLayer','setComposition','addMarker','removeMarker','createSection','updateSection'];
const stops=['setComposition','addLayer','removeMarker','createSection','updateSection','simplificationRefs'];
const bodies=functions.map((name,i)=>source.slice(source.indexOf(`function ${name}(`),source.indexOf(`function ${stops[i]}(`))).join('\n');
const globals={PM,LAYER_FIELDS,findLayer:id=>PM.proj.layers.find(l=>l.id===id||l.name===id),safePatch:v=>structuredClone(v),finite:n=>{if(typeof n!=='number'||!Number.isFinite(n))throw Error('Invalid number');return n;},isProperty:v=>v&&typeof v==='object'&&Array.isArray(v.kf),clone:structuredClone,layer3DRole:()=>null};
const f=evaluate(bodies,functions,globals);
const names={set_layer:'setLayer',set_composition:'setComposition',add_marker:'addMarker',remove_marker:'removeMarker',create_section:'createSection',update_section:'updateSection'};
process.stdout.write(JSON.stringify(input.commands.map(command=>({command,data:f[names[command.type]](command)}))));
