// Test-only execution of the owner's exact main reference. No app/JS runtime
// dependency; synthetic PM seams supply only document access and property data.
import {execFileSync} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
const MAIN='c7f46a05641b23944d3692c9d80f9677c7e1ef1d';
const path='apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts';
const source=(rev,path)=>execFileSync('git',['show',`${rev}:${path}`],{encoding:'utf8'});
const video=source(MAIN,path);
const content=source(MAIN,'apps/desktop/src/renderer/src/legacy/core/content-properties.ts');
const plain=src=>stripTypeScriptTypes(src.replace(/^import[^\n]*\n/gm,''),{mode:'strip'}).replace(/^export /gm,'');
const editVideo=new Function(`${plain(content)}\n${plain(video)}\nreturn editVideo;`)();
const fixtures=[];
for(const rate of [.5,1,2]) for(const operation of ['move','trim-in','trim-out','slip']) {
  const before={id:'clip',name:'Synthetic',type:'video',from:4,dur:4,track:0,on:true,
    d:{asset:'src',trim:{v:6,kf:[],expr:null},speed:rate,embeddedAudio:false,opaque:{retain:true}},
    p:{'position.x':{v:0,kf:[{t:1,v:3,i:'key-a'},{t:3,v:9,i:'key-b'}],expr:null}},
    fx:[],masks:[],extensionData:{authored:'keep'}};
  const after=structuredClone(before);
  const PM={proj:{fps:30,dur:40,assets:{src:{dur:100}},layers:[after]},
    assets:{get:()=>({dur:100,hasAudio:true})},
    groupAncestors:()=>[],P:v=>({v,kf:[],expr:null}),Edit:{mutate:(_label,body)=>body()},
    allProps:layer=>{
      const out=[];const visit=v=>{if(!v||typeof v!=='object')return;if(Array.isArray(v.kf)&&'v' in v){out.push({prop:v});return;}for(const child of Object.values(v))visit(child);};visit(layer);return out;
    }};
  const delta=.375;
  const args=operation==='move'?{operation,layerId:'clip',from:4+delta}:
    operation==='slip'?{operation,layerId:'clip',sourceIn:6+delta*rate}:
    {operation:'trim',layerId:'clip',start:4+(operation==='trim-in'?delta:0),end:8+(operation==='trim-out'?delta:0)};
  editVideo(PM,args,{label:'Synthetic pair'});
  fixtures.push({operation,rate,before,after,delta});
}
process.stdout.write(JSON.stringify({reference:MAIN,legacyBodyUnchanged:video===source('v1.2.0',path),fixtures}));
