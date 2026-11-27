/** Apply/remove only this overlay, never alter the frozen checkout's source history. */
import { readFile, writeFile, mkdir, copyFile, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import {createHash} from 'node:crypto';
const root = path.resolve(import.meta.dir);
const ref = path.resolve(process.argv[2] ?? '/tmp/pm-ref-v1.2.0');
const remove = process.argv.includes('--remove');
const config = path.join(ref, 'apps/desktop/electron.vite.config.ts');
const original = execFileSync('git', ['-C', ref, 'show', 'HEAD:apps/desktop/electron.vite.config.ts'], { encoding:'utf8' });
const edited = `import { traceRecorderPlugin, traceRecorderMainPlugin } from './scripts/trace-recorder/transform.mjs';\n${original.replace('plugins: [svelte(),', 'plugins: [traceRecorderPlugin(), svelte(),').replace('main: {', 'main: { plugins: [traceRecorderMainPlugin()],')}`;
if(!original.includes('plugins: [svelte(),')||!original.includes('main: {')) throw new Error('Unsupported Vite config; update the overlay adapter for this reference');
const current = await readFile(config, 'utf8');
if (current !== original && current !== edited) throw new Error('Refusing to overwrite an unrelated Vite config edit');
if (remove) {
  await writeFile(config, original);
  await rm(path.join(ref,'apps/desktop/e2e/helpers/app.trace.mjs'),{force:true});
  for (const dir of ['apps/desktop/scripts/trace-recorder', 'apps/desktop/src/renderer/src/trace-recorder']) await rm(path.join(ref, dir), {recursive:true,force:true});
} else {
  for (const dir of ['apps/desktop/scripts/trace-recorder', 'apps/desktop/src/renderer/src/trace-recorder']) await mkdir(path.join(ref, dir), {recursive:true});
  await copyFile(path.join(root,'overlay/transform.mjs'),path.join(ref,'apps/desktop/scripts/trace-recorder/transform.mjs'));
  const hash=createHash('sha256');
  for(const file of ['core.ts','overlay/runtime.ts','overlay/transform.mjs']) hash.update(await readFile(path.join(root,file)));
  const runtime=(await readFile(path.join(root,'overlay/runtime.ts'),'utf8')).replace('__PM_TRACE_BUILD_FINGERPRINT__',hash.digest('hex'));
  await writeFile(path.join(ref,'apps/desktop/src/renderer/src/trace-recorder/runtime.ts'),runtime);
  await copyFile(path.join(root,'core.ts'),path.join(ref,'apps/desktop/src/renderer/src/trace-recorder/core.ts'));
  await writeFile(config,edited);
  const helper=path.join(ref,'apps/desktop/e2e/helpers/app.ts');
  // Reuse the existing harness, adding finite startup waits and cleanup for direct CLI use.
  const helperOriginal=await readFile(helper,'utf8');
  for(const anchor of ['_electron.launch({','// Native dialogs are substituted','return { app, page };']) {
    if(helperOriginal.indexOf(anchor)<0||helperOriginal.indexOf(anchor)!==helperOriginal.lastIndexOf(anchor))throw new Error(`Unsupported harness anchor: ${anchor}`);
  }
  const helperSource=helperOriginal
    .replace('_electron.launch({','_electron.launch({ timeout: 30_000,')
    .replace('// Native dialogs are substituted', 'app.context().setDefaultTimeout(30_000);\n  try {\n  // Native dialogs are substituted')
    .replace('return { app, page };','return { app, page };\n  } catch(error) { await app.close().catch(()=>{}); throw error; }');
  const build=await Bun.build({entrypoints:[helper],target:'node',packages:'external',define:{__dirname:JSON.stringify(path.dirname(helper))},plugins:[{name:'trace-harness-cli',setup(build){build.onLoad({filter:/e2e\/helpers\/app\.ts$/},()=>({contents:helperSource,loader:'ts'}));}}]});
  if(!build.success) throw new AggregateError(build.logs,'Harness transpilation failed');
  await writeFile(path.join(path.dirname(helper),'app.trace.mjs'), await build.outputs[0].text());
}
console.log(`${remove ? 'Removed' : 'Applied'} trace overlay: ${ref}`);
