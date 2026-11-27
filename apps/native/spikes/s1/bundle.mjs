import {build} from 'rolldown';
import {parseSync} from 'oxc-parser';
import {mkdir,writeFile,readFile} from 'node:fs/promises';
await mkdir('.generated',{recursive:true});
for (const kind of ['preact','react']) {
  const source=await readFile(`${kind}.js`,'utf8');
  const parsed=parseSync(`${kind}.js`,source);
  if(parsed.errors.length)throw Error(JSON.stringify(parsed.errors));
  await build({input:`${kind}.js`,platform:'neutral',resolve:{mainFields:['module','main']},transform:{define:{'process.env.NODE_ENV':JSON.stringify('development')}},output:{file:`.generated/${kind}.js`,format:'iife',name:`${kind}Probe`,intro:'globalThis.self=globalThis;'}});
  // The bundle assigns its result; return JSON to the Rust V8 harness.
  await writeFile(`.generated/${kind}.js`,(await readFile(`.generated/${kind}.js`,'utf8'))+'\nJSON.stringify(globalThis.probeResult)\n');
}
await build({input:'compiler.js',platform:'neutral',resolve:{mainFields:['module','main']},output:{file:'.generated/compiler.js',format:'iife',name:'compilerProbe'}});
await writeFile('.generated/compiler.js',(await readFile('.generated/compiler.js','utf8'))+'\nJSON.stringify(globalThis.probeResult)\n');
