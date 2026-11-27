// Test-only. Executes pure functions from the frozen tag, not native code.
// Node 22.18+; no npm packages and no app/runtime dependency on Node.
import {execFileSync} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
import {writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
const baselineCommit=execFileSync('git',['rev-parse','v1.2.0^{commit}'],{encoding:'utf8'}).trim();
if(baselineCommit!=='07ed5fa17ee640659cb075f9cad1af2095011d60') throw new Error('Frozen tag identity changed');
const sha256=source=>createHash('sha256').update(source).digest('hex');
const base='apps/desktop/src/renderer/src/panels/agent/';
const source=execFileSync('git',['show',`v1.2.0:${base}slash-commands.ts`],{encoding:'utf8'});
const module=await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source,{mode:'strip'})).toString('base64')}`);
const catalog={models:[{id:'sol',label:'Sol 6.1'},{id:'claude-sonnet',label:'Sonnet'}],providers:[{id:'chatgpt',label:'ChatGPT'},{id:'claude',label:'Claude'}],reasoningEfforts:['low','high','xhigh'],provider:'claude'};
const cases=[];
for(const legacyPhase of ['idle','working','applying']) for(const blocked of [false,true]) for(const draft of ['/', '/n','/model','/model ','/model SOL','/model claude-new-v1','/model claude-a_b','/provider cla','/effort extra','/effort    high','/  model',' /','/model\nsol']) {
 const state={...catalog,legacyPhase,threadSwitchBlocked:blocked};
 const choices=module.slashCommands(draft,state).map(({id,action,value})=>({id,action,...(value===undefined?{}:{value})}));
 cases.push({draft,legacyPhase,blocked,choices});
}
writeFileSync(new URL('../fixtures/slash-v1.2.0.json',import.meta.url),JSON.stringify({baseline:'v1.2.0',baselineCommit,sourceSha256:sha256(source),source:`${base}slash-commands.ts:11-45`,cases},null,2)+'\n');
const placementSource=execFileSync('git',['show',`v1.2.0:${base}ui-placement.ts`],{encoding:'utf8'});
const placement=await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(placementSource,{mode:'strip'})).toString('base64')}`);
const placementCases=[];
for(const source of ['Hello\nPOWERMOVE_UI_TARGET {"kind":"panel","id":"p","label":"Work"}\nAfter','POWERMOVE_UI_TARGET {"label":"brace } in string"} tail','POWERMOVE_UI_TARGET {"id":','POWERMOVE_UI_TARGET blah\nKeep','Keep\n  POW','Use POWERMOVE_UI_TARGET {"id":"p"} inline','POWERMOVE_UI_TARGET {"a":{}}\nPOWERMOVE_UI_TARGET{"b":1}','POWERMOVE_UI_TARGETED is prose','\n\tPOWERMOVE_UI_TARGET\n{"x":1}\nend']) for(const streaming of [false,true]) placementCases.push({source,streaming,expected:placement.splitUIPlacementText(source,streaming)});
writeFileSync(new URL('../fixtures/placement-v1.2.0.json',import.meta.url),JSON.stringify({baseline:'v1.2.0',baselineCommit,sourceSha256:sha256(placementSource),source:`${base}ui-placement.ts:18-59`,cases:placementCases},null,2)+'\n');
