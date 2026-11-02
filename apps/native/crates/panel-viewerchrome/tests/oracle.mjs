// Read-only v1.2.0 source oracle. Node is fixture-generation-only, not runtime.
import {execFileSync} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
import {writeFileSync} from 'node:fs';
const source=execFileSync('git',['show','v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts'],{encoding:'utf8'});
function tagged(name) {
  const match=source.match(new RegExp(`export function ${name}\\([\\s\\S]*?^}`,'m'));
  if(!match) throw new Error(`missing tagged function ${name}`);
  const js=stripTypeScriptTypes(match[0].replace(/^export /,''));
  return new Function(`${js}; return ${name};`)();
}
const origin=tagged('compositionFramePosition'), pan=tagged('zoomPanForPoint'), size=tagged('previewRenderSize');
const cases=[];
for(const zoom of [.02,.05,.125,.25,.5,1,2,8]) {
  const stage={width:801.25,height:599.75},composition={width:1920,height:1080},pointer={x:123.25,y:456.5},point={x:175.125,y:91.75};
  cases.push({zoom,stage,composition,pointer,point,origin:origin(stage,composition,zoom,{x:13.25,y:-17.5}),pan:pan(stage,pointer,point,composition,zoom),size:size(1920,1080,zoom,2,.5)});
}
writeFileSync(new URL('./fixtures/navigation-v1.2.0.json',import.meta.url),JSON.stringify({tag:'v1.2.0',source:'apps/desktop/src/extensions/viewer/viewer.ts:28,180,218',cases},null,2)+'\n');
console.log(`${cases.length} tagged navigation cases written`);
